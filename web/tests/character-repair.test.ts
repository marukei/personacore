/**
 * **壊れた行（`unreadable`）を修復するときに、何を送り、何と言うか**
 * （作業単位 D-6 / `src/lib/characterRepair.ts` / ADR-0016 §3・§4）。
 *
 * # なぜ純粋関数で見るのか
 *
 * この作業単位の核心は **「`PATCH` は読み出しを通らずに直せる」**（ADR-0016 §4）で、
 * その形は**送る本文の側**にしか現れない ——
 * **D-2 の編集は差分だけを送る**ので、**差分を取る元の値が読めない行には使えない。**
 * 「全部の欄が必ず載っていること」は画面越しには観測しづらいので、ここで単体で見張る。
 *
 * # 画面側との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **送る本文・入口を出すか・失敗の案内** | **ここ** |
 * | 一覧のどの行を押したらどの識別子へ飛ぶか | `character-list-repair.test.ts` |
 * | 修復フォーム単体の欄・失敗の表示 | `character-repair-form.test.ts` |
 * | `#/characters` の繋ぎ目 | `app-routing-characters.test.ts` |
 *
 * **I/O を持たないので jsdom は要らない**（既定の `node` で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError, type UnreadableCharacterView } from "../src/lib/api";
import {
  LIMITS,
  buildUpdate,
  type CharacterDraft,
} from "../src/lib/characterEdit";
import {
  NOT_FOUND_CODE,
  COLUMNS_WITHOUT_A_FIELD,
  REPAIR_COLUMNS,
  SCALE_BELOW_STORED_CODE,
  UNREPAIRABLE_COLUMN,
  buildRepair,
  columnLabel,
  describeRepairFailure,
  emptyRepairDraft,
  isRepairColumn,
  repairTargetOf,
  type RepairDraft,
} from "../src/lib/characterRepair";

/**
 * 識別子が読めた行。**壊れた列は1つ。**
 *
 * **`id` / `handle` / `invalid_columns` の3つが、下の行と全部違う**
 * ——同値にすると取り違えが観測できない（CLAUDE.md の最頻の穴）。
 */
const BROKEN_KOU: UnreadableCharacterView = {
  id: 11,
  handle: "kou",
  invalid_columns: ["display_name"],
};

/** 識別子が読めた行。**壊れた列が複数。** */
const BROKEN_SEN: UnreadableCharacterView = {
  id: 12,
  handle: "sen",
  invalid_columns: ["base_persona", "stat_scale"],
};

/**
 * **識別子そのものが読めない行。**
 *
 * **`handle` が `null` ⟺ `invalid_columns` に `handle` が入っている**
 * （`domain::UnreadableCharacter` の `handle` は `Handle::parse` を通ったときだけ `Some`）。
 * **この行は `PATCH` で直せない** —— 要求型に識別子の口が無い（ADR-0016 §1）。
 */
const BROKEN_NAMELESS: UnreadableCharacterView = {
  id: 13,
  handle: null,
  invalid_columns: ["handle", "base_persona"],
};

/**
 * 尺度だけ埋めた下書き（**他は空のまま**）。
 *
 * **走らせ方の列は壊れている側**（作業単位 D-15）—— 5つのキーが全部載る形。
 * 壊れていない側（キーを省く）は `tests/drift-mode-choice.test.ts` の「修復の本文」が見る。
 */
function draftWithScale(scale: string): RepairDraft {
  return { ...emptyRepairDraft(true), stat_scale: scale };
}

describe("修復の入口を出すかどうか", () => {
  it("識別子が読めた行は、その行の id と識別子と壊れた列を運ぶ", () => {
    const target = repairTargetOf(BROKEN_SEN);
    expect(target).not.toBeNull();
    const row = target as NonNullable<typeof target>;
    expect(row.id).toBe(BROKEN_SEN.id);
    expect(row.handle).toBe(BROKEN_SEN.handle);
    expect(row.invalid_columns).toEqual(BROKEN_SEN.invalid_columns);
    // **対照: 隣の行のものではない**（帰属）。
    expect(row.id).not.toBe(BROKEN_KOU.id);
    expect(row.handle).not.toBe(BROKEN_KOU.handle);
    expect(row.invalid_columns).not.toEqual(BROKEN_KOU.invalid_columns);
  });

  /**
   * **⚠ 無い操作を案内しない**（D-7 と同じ立場）。
   *
   * 識別子は不変（ADR-0016 §1）なので `update_character` では読めるようにならず、
   * 掃除（`delete_character`）も綴りを渡せない以上届かない
   * （`domain::UnreadableCharacter` の `handle` の doc）。
   * **入口が無いことを、入口を出さないことで表す。**
   */
  it("識別子が読めない行には、入口が無い", () => {
    expect(repairTargetOf(BROKEN_NAMELESS)).toBeNull();
  });

  it("同じ一覧に、出る行と出ない行が同居できる", () => {
    const rows = [BROKEN_KOU, BROKEN_NAMELESS, BROKEN_SEN];
    const targets = rows.map(repairTargetOf);
    expect(targets.map((row) => row?.handle ?? null)).toEqual([
      BROKEN_KOU.handle,
      null,
      BROKEN_SEN.handle,
    ]);
  });
});

describe("全部の欄を送る（差分ではない）", () => {
  /**
   * **これがこの作業単位の存在理由。**
   *
   * 元の値が読めないので「変えていない欄」を決めようがない。
   * **1つでもキーが落ちると、その列は壊れたまま残る**（サーバはキーの無い列を触らない /
   * `admin.rs` の `UpdateCharacterRequest`）。
   */
  /**
   * **走らせ方の列が壊れている行では、5つとも載る**（作業単位 D-15）。
   * 壊れていない行では走らせ方だけ省く（`drift-mode-choice.test.ts` の「修復の本文」）。
   */
  it("空欄でも、5つのキーが必ず載る（走らせ方の列が壊れている行）", () => {
    const built = buildRepair(draftWithScale("5"));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    // **5つ目の `stat_drift_mode` は作業単位 D-15 で足した**（「全部載せる」の主張は同じ）。
    expect(Object.keys(built.body).sort()).toEqual([
      "base_persona",
      "display_name",
      "output_format_instruction",
      "stat_drift_mode",
      "stat_scale",
    ]);
    // **走らせ方の既定は「既定に従う」＝ `null`（未設定に戻す）。**
    expect(built.body.stat_drift_mode).toBeNull();
    // **空欄は空文字のまま送る**（こちらで別の言い方をしない / `buildCreate` と同じ立場）。
    expect(built.body.display_name).toBe("");
    expect(built.body.base_persona).toBe("");
    // **形式指示だけが3値。** 空は「消す」（`null`）。
    expect(built.body.output_format_instruction).toBeNull();
    expect(built.body.stat_scale).toBe(5);
  });

  it("書いた欄は、その値がそのまま載る", () => {
    const built = buildRepair({
      display_name: "宗",
      base_persona: "低い声で話す。",
      output_format_instruction: "台詞は「」で囲む。",
      stat_scale: "7",
      stat_drift_mode: "on",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body).toEqual({
      display_name: "宗",
      base_persona: "低い声で話す。",
      output_format_instruction: "台詞は「」で囲む。",
      stat_scale: 7,
      stat_drift_mode: "on",
    });
  });

  it("識別子のキーは絶対に載らない（不変 / `deny_unknown_fields`）", () => {
    const built = buildRepair(draftWithScale("5"));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(Object.keys(built.body)).not.toContain("handle");
    expect(Object.keys(built.body)).not.toContain("name");
  });

  /**
   * **D-2 のモードとの対比**（＝ここが第3のモードである、ということの権威）。
   *
   * **同じ値の下書きを、両方のモードへ通す。**
   * 差分モードはキーを1つも載せない（変えていないから）。
   * **修復モードは5つ全部載せる**（5つ目の走らせ方は作業単位 D-15 で足した）。
   * **これが無いと「全部の欄を送るはずが、差分だけ送る」変異が観測できない。**
   */
  it("同じ内容でも、差分モードはキーを落とし、修復モードは落とさない", () => {
    const original = {
      id: 12,
      handle: "sen",
      display_name: "宗",
      base_persona: "低い声で話す。",
      output_format_instruction: null,
      stat_scale: 7,
      stat_drift_mode: null,
      axis_discovery_slots: 0,
      archived_at: null,
      created_at: "2026-08-27T00:00:00Z",
    };
    const repairDraft: RepairDraft = {
      display_name: original.display_name,
      base_persona: original.base_persona,
      output_format_instruction: "",
      stat_scale: String(original.stat_scale),
      // **元は未設定（`null`）＝「既定に従う」**。差分モードでは変えていない扱い。
      stat_drift_mode: "",
    };
    const editDraft: CharacterDraft = {
      handle: original.handle,
      ...repairDraft,
    };

    const diff = buildUpdate(original, editDraft);
    expect(diff.ok).toBe(true);
    if (!diff.ok || diff.kind !== "update") {
      return;
    }
    // **差分モード: 1つも送らない。**
    expect(Object.keys(diff.body)).toEqual([]);

    const repair = buildRepair(repairDraft);
    expect(repair.ok).toBe(true);
    if (!repair.ok) {
      return;
    }
    // **修復モード: 5つとも送る。**
    expect(Object.keys(repair.body).length).toBe(5);
    expect(Object.keys(repair.body).length).toBeGreaterThan(
      Object.keys(diff.body).length,
    );
  });
});

describe("尺度は省略できない", () => {
  /**
   * **この列は NOT NULL で、保存されている値も読めない。**
   * キーごと落とすと「変えない」になり、**壊れている尺度がそのまま残る。**
   */
  it("空なら送らずに理由を返す", () => {
    const built = buildRepair(emptyRepairDraft(true));
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("尺度");
  });

  it("整数でなければ送らない", () => {
    const built = buildRepair(draftWithScale("5.5"));
    expect(built.ok).toBe(false);
  });

  it("整数なら数として載る（文字列で送らない）", () => {
    const built = buildRepair(draftWithScale(" 12 "));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body.stat_scale).toBe(12);
    expect(typeof built.body.stat_scale).toBe("number");
  });

  it("空のときと、整数でないときで、別のことを言う", () => {
    const empty = buildRepair(emptyRepairDraft(true));
    const invalid = buildRepair(draftWithScale("いち"));
    expect(empty.ok).toBe(false);
    expect(invalid.ok).toBe(false);
    if (empty.ok || invalid.ok) {
      return;
    }
    expect(empty.message).not.toBe(invalid.message);
  });
});

/**
 * **上限の門は、4列すべてについて効いていること。**
 *
 * # ⚠ **列ごとに1本ずつ置く**（D-6 のレビューの是正1）
 *
 * **実測**: `REPAIR_LIMITED_FIELDS` から `"output_format_instruction"` を落とす変異が
 * **262 全緑**だった。**対照が痛烈で、編集側の同じ変異は 2 赤** ——
 * そのうち1本が `character-edit.test.ts` の
 * **`出力形式指示は、作成でも更新でも止まる`**（**D-2 が「この列は別に見張る」と
 * 名指しで置いた見張り番**）。
 *
 * **doc は写したのに、doc が名指ししていた見張り番だけを写していなかった**
 * ——CLAUDE.md の「doc を写すときは、その doc が名指ししている見張り番も
 * 一緒に写っているかを問う」の6例目。
 *
 * **代表1列で済ませないこと。** 1列だけだと、他の3列がリストから落ちても緑になる。
 */
describe("上限を超えた欄は、送る前に止める", () => {
  /**
   * **上限は `characterEdit.ts` の `LIMITS` をそのまま使う**
   * （写しを2つにしない。写し元を読む見張り番は `character-edit.test.ts`）。
   *
   * **`LIMITS` から欄ごとに引く**ので、上限の値を変えてもこの検査は追随する。
   */
  it.each([
    ["display_name" as const, "表示名"],
    ["base_persona" as const, "ペルソナ本文"],
    ["output_format_instruction" as const, "出力形式指示"],
  ])("%s: 上限ちょうどは通り、1文字超えると止まる", (field, label) => {
    const justFits = buildRepair({
      ...draftWithScale("5"),
      [field]: "あ".repeat(LIMITS[field]),
    });
    expect(justFits.ok, `${label} は上限ちょうどなら通ること`).toBe(true);

    const oneOver = buildRepair({
      ...draftWithScale("5"),
      [field]: "あ".repeat(LIMITS[field] + 1),
    });
    expect(oneOver.ok, `${label} は1文字超えたら止まること`).toBe(false);
    if (oneOver.ok) {
      return;
    }
    // **止まっただけでなく、その欄を名乗っている。**
    expect(oneOver.message).toContain(label);
  });

  /**
   * **4列目（`stat_scale`）の門は文字数ではない。**
   *
   * この列に `MAX_CHARS` は無い（`LIMITS` に無い）。門は
   * **「省略できない」と「整数であること」**で、そちらは
   * `尺度は省略できない` が見ている。**ここでは、文字数の門に
   * 巻き込まれていないこと**（長い数字列が「長すぎます」にならないこと）を見る。
   */
  it("stat_scale: 文字数では止めない（門は整数かどうか）", () => {
    const long = buildRepair(draftWithScale("9".repeat(80)));
    expect(long.ok).toBe(false);
    if (long.ok) {
      return;
    }
    // **上限超過のメッセージではない**（`Number.isSafeInteger` で落ちている）。
    expect(long.message).not.toContain("長すぎます");
    expect(long.message).toContain("尺度");
  });

  it("止めたときは、その欄とその欄の上限を名乗る", () => {
    const built = buildRepair({
      ...draftWithScale("5"),
      display_name: "あ".repeat(LIMITS.display_name + 1),
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("表示名");
    expect(built.message).toContain(String(LIMITS.display_name));
    // **他の欄の上限を出していない。**
    expect(built.message).not.toContain(String(LIMITS.base_persona));
    expect(built.message).not.toContain("ペルソナ本文");
  });

  /**
   * **識別子は数えない。** 数えると、**上限を下げた後の DB で、
   * 保存済みの識別子が長すぎる行の修復ができなくなる**
   * （`characterEdit.ts` の `UPDATE_FIELDS` と同じ理由）。
   * そもそも修復の下書きに識別子の欄は無い。
   */
  it("下書きに識別子の欄が無い", () => {
    expect(Object.keys(emptyRepairDraft(true)).sort()).toEqual([
      "base_persona",
      "display_name",
      "output_format_instruction",
      "stat_drift_mode",
      "stat_scale",
    ]);
  });
});

describe("読めなかった列の見出し", () => {
  it("直せる4列は、それぞれ別の見出しを持つ", () => {
    const labels = REPAIR_COLUMNS.map(columnLabel);
    expect(new Set(labels).size).toBe(REPAIR_COLUMNS.length);
    expect(columnLabel("display_name")).toBe("表示名");
    expect(columnLabel("base_persona")).toBe("ペルソナ本文");
    expect(columnLabel("output_format_instruction")).toBe("出力形式指示");
    expect(columnLabel("stat_scale")).not.toBe(columnLabel("base_persona"));
  });

  it("識別子は、直せる列に数えない", () => {
    expect(isRepairColumn(UNREPAIRABLE_COLUMN)).toBe(false);
    for (const column of REPAIR_COLUMNS) {
      expect(isRepairColumn(column)).toBe(true);
      expect(column).not.toBe(UNREPAIRABLE_COLUMN);
    }
  });

  /**
   * **知らない綴りを落とさない。** サーバが列を増やした日に、
   * **壊れている列が画面から消える**のがいちばん質の悪い壊れ方
   * （`MemoryList` が生の主語を併記しているのと同じ立場）。
   */
  it("知らない列名は、そのまま出す（落とさない）", () => {
    expect(columnLabel("locked")).toContain("locked");
    expect(isRepairColumn("locked")).toBe(false);
  });
});

/**
 * **列名は `src/domain/character.rs` の `CharacterColumn::as_str` の写しである。**
 *
 * # なぜ見張るのか
 *
 * **型ではなく値の写し。** 綴りがずれると、**壊れている欄に印が付かなくなる**
 * ——画面は「読めなかった列」を並べたまま、どの欄を直せばよいかを言わなくなるのに、
 * **型検査もテストも緑のまま。**
 *
 * **先例は2つ**（`characterEdit.ts` の `LIMITS` / `characterDelete.ts` の `code`）。
 * **同じ形で塞ぐ。**
 */
describe("列名は写し元と一致する", () => {
  it("`src/domain/character.rs` の `CharacterColumn::as_str` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/domain/character.rs", import.meta.url)),
      "utf8",
    );
    const names = columnNamesOf(source);

    // **過不足なく一致する。** 増えても減っても赤くなる。
    // **欄の無い列（作業単位 A-9a の `axis_discovery_slots`）も数える** ——
    // サーバが指し示しうる列は、直せる・直せない・欄が無い、のどれかに必ず分類されている。
    expect(new Set(names)).toEqual(
      new Set([UNREPAIRABLE_COLUMN, ...REPAIR_COLUMNS, ...COLUMNS_WITHOUT_A_FIELD]),
    );
    expect(names.length).toBe(
      REPAIR_COLUMNS.length + 1 + COLUMNS_WITHOUT_A_FIELD.length,
    );
    // **欄の無い列を、直せる列として案内しない。**
    for (const column of COLUMNS_WITHOUT_A_FIELD) {
      expect(isRepairColumn(column)).toBe(false);
      expect(column).not.toBe(UNREPAIRABLE_COLUMN);
    }
    // **対照: 直せない列は1つだけで、直せる列に混ざっていない。**
    expect(names).toContain(UNREPAIRABLE_COLUMN);
    expect(REPAIR_COLUMNS).not.toContain(UNREPAIRABLE_COLUMN);
  });

  /**
   * **走査する側が壊れていないこと**（`15b` の「そもそも探せていないから緑」）。
   *
   * **⚠ 実ファイルだけを見る対照は自己成就する** ——
   * 走査の綴りを壊す変異が、**別の綴りを走査対象そのものに書き込む**と
   * 1件見つけて緑になる。**だから、植えたソースを同じ関数へ通す。**
   */
  it("対照: 植えたソースからは、その場の綴りを読む", () => {
    const planted = [
      "impl CharacterColumn {",
      "    pub const fn as_str(self) -> &'static str {",
      "        match self {",
      '            Self::Handle => "planted_handle",',
      '            Self::DisplayName => "planted_display_name",',
      "        }",
      "    }",
      "}",
    ].join("\n");
    expect(columnNamesOf(planted)).toEqual([
      "planted_handle",
      "planted_display_name",
    ]);
    expect(columnNamesOf(planted)).not.toContain(UNREPAIRABLE_COLUMN);
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => columnNamesOf("")).toThrow();
    // `impl` はあるが `as_str` が無い。
    expect(() =>
      columnNamesOf(
        "impl CharacterColumn {\n    pub const ALL: [Self; 0] = [];\n}",
      ),
    ).toThrow();
    // **次の `impl` の綴りを拾ってこない。**
    const two = [
      "impl CharacterColumn {",
      "    pub const ALL: [Self; 0] = [];",
      "}",
      "",
      "impl Handle {",
      "    pub const fn as_str(self) -> &'static str {",
      '        match self { Self::Handle => "handle", }',
      "    }",
      "}",
    ].join("\n");
    expect(() => columnNamesOf(two)).toThrow();
  });
});

/**
 * `CharacterColumn::as_str` が返す列名を全部取り出す。
 *
 * **`impl CharacterColumn` の中だけを見る**（`character.rs` には `as_str` が5本ある）。
 * **次の `impl` の手前で切る。** 切らないと、`as_str` を持たない `impl` を
 * 尋ねたときに次の型の綴りを返す（探せていないのに緑になる）。
 */
function columnNamesOf(source: string): string[] {
  const start = source.indexOf("impl CharacterColumn {");
  if (start < 0) {
    throw new Error("impl CharacterColumn が見つかりません");
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("\nimpl ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const at = block.indexOf("fn as_str");
  if (at < 0) {
    throw new Error("impl CharacterColumn に as_str がありません");
  }
  const names = [...block.slice(at).matchAll(/Self::\w+ => "([^"]+)",/g)].map(
    (found) => found[1] as string,
  );
  if (names.length === 0) {
    throw new Error("as_str から列名を1つも読めません");
  }
  return names;
}

/**
 * サーバが実際に返すもの（`src/server/error.rs` の
 * `ApiError::stat_scale_below_stored_value` を読んで写した）。
 *
 * **⚠ これは前提を書いたフィクスチャであって、写しの正しさを見張るものではない。**
 * 綴りの突き合わせは下の「`code` は写し元と一致する」が**写し元を実際に読んで**行う。
 */
const SCALE_BELOW_STORED = new ApiRequestError(
  "保存されている性格ステータスの値より小さい尺度には変更できません",
  409,
  "invalid_request_error",
  "stat_scale_below_stored_value",
);

/** 同上（`ApiError::character_not_found`）。 */
const NOT_FOUND = new ApiRequestError(
  "指定されたキャラクターが存在しません",
  404,
  "invalid_request_error",
  "character_not_found",
);

/**
 * **`classify_update` が返しうる拒否のうち、画面で区別するもの。**
 *
 * `src/server/admin.rs` の `classify_update` は3つに分ける ——
 * `UnknownCharacter` → 404 / `StatScaleBelowStoredValue` → 409 / それ以外 → 500。
 * **前の2つだけを区別する。** 500 は「サーバの障害」で、次の一手が無い
 * （推測で埋めない / `characterDelete.ts` と同じ立場）。
 */
describe("修復が拒否されたときの読み分け", () => {
  it("尺度が保存済みの値より小さい: もっと大きい尺度を、と言う", () => {
    const failure = describeRepairFailure(SCALE_BELOW_STORED);
    expect(failure.reason).toBe("scale-below-stored");
    expect(failure.message).toBe(SCALE_BELOW_STORED.message);
    expect(failure.guidance).not.toBeNull();
    expect(failure.guidance as string).toContain("大きい尺度");
  });

  /**
   * **⚠ 無い操作を案内しない**（D-7 と同じ）。
   * 「先に性格値を下げてください」と書くと、
   * **利用者は存在しないボタンを探すことになる。**
   *
   * # ⚠ **結論は同じ。根拠が変わった**（2026-08-28 / 作業単位 D-4）
   *
   * **性格値を変える画面はできた**（`#/stats/{識別子}`）。
   * **それでも、この案内からそこへ導いてはならない。**
   *
   * **`GET .../stats` は `find_character` → `Store::find_character_by_handle` を通り、
   * 全フィールドを `parse` する。**
   * `src/store/character.rs` の `update_character` の doc が
   * **「直したい行ほど読めない。修復の対象はどれも `InvalidStoredValue` になる行」**と書いている。
   * **つまり、この画面が相手にしている行では、性格値の画面が開かない。**
   *
   * **⚠ 「画面ができたのに案内していない、直そう」と読まないこと。**
   * **足せる条件は「読めない行でも開ける読み出しがサーバにできたとき」**である。
   */
  it("案内が、この行では開けない画面へ導いていない", () => {
    const guidance = describeRepairFailure(SCALE_BELOW_STORED)
      .guidance as string;
    expect(guidance).not.toContain("下げてください");
    expect(guidance).not.toContain("性格値を下げ");
    // **無いことを、無いと言う。**
    expect(guidance).toContain("ありません");
  });

  it("その識別子が居ない: 取り直す、と言う", () => {
    const failure = describeRepairFailure(NOT_FOUND);
    expect(failure.reason).toBe("not-found");
    expect(failure.message).toBe(NOT_FOUND.message);
    expect(failure.guidance as string).toContain("再読み込み");
    // **尺度側の一手と混ざっていない**（対処が別物）。
    expect(failure.guidance as string).not.toContain("大きい尺度");
  });

  it("2種の案内は、文言そのものが違う", () => {
    expect(describeRepairFailure(NOT_FOUND).guidance).not.toBe(
      describeRepairFailure(SCALE_BELOW_STORED).guidance,
    );
    expect(describeRepairFailure(NOT_FOUND).reason).not.toBe(
      describeRepairFailure(SCALE_BELOW_STORED).reason,
    );
  });

  it("`code` の無い 409 を、尺度の拒否にしない", () => {
    const failure = describeRepairFailure(
      new ApiRequestError("サーバが 409 を返しました", 409, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
  });

  it("`code` の無い 404 を、居ない扱いにしない", () => {
    const failure = describeRepairFailure(
      new ApiRequestError("サーバが 404 を返しました", 404, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
  });

  /**
   * **400 は素通しする。**
   *
   * `character_update` の `parse` が返す拒否（`persona_too_long` など）は
   * **サーバが壊れている欄を名乗る**ので、こちらから言い換えない。
   */
  it("知らない `code` は、そのまま「分からない」（文言だけ出す）", () => {
    const failure = describeRepairFailure(
      new ApiRequestError(
        "ペルソナ本文が長すぎます",
        400,
        "invalid_request_error",
        "persona_too_long",
      ),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("ペルソナ本文が長すぎます");
  });

  it("`ApiRequestError` ですらない失敗は、中身を画面へ流さない", () => {
    const failure = describeRepairFailure(
      new Error("C:\\Users\\someone\\Rust\\personacore\\web で落ちた"),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("予期しない失敗です");
    expect(failure.message).not.toContain("Rust");
  });
});

/**
 * **文言は素通しする（決め打ちしない）** —— `character-delete.test.ts` と同じ理由。
 *
 * `src/server/error.rs` は `character_already_exists` の文言を
 * **`code` を変えずに改訂した記録**を持っている。**起きたことである。**
 */
describe("文言は素通しする", () => {
  const REWORDED_SCALE = new ApiRequestError(
    "その尺度では、保存済みの値が収まりません（文言が改訂された想定）",
    409,
    "invalid_request_error",
    SCALE_BELOW_STORED_CODE,
  );

  const REWORDED_NOT_FOUND = new ApiRequestError(
    "その識別子は見つかりませんでした（文言が改訂された想定）",
    404,
    "invalid_request_error",
    NOT_FOUND_CODE,
  );

  it("尺度側: 文言が改訂されても、そのまま出す", () => {
    const failure = describeRepairFailure(REWORDED_SCALE);
    expect(failure.reason).toBe("scale-below-stored");
    expect(failure.message).toBe(REWORDED_SCALE.message);
    expect(failure.message).not.toBe(SCALE_BELOW_STORED.message);
    expect(failure.message).not.toContain("変更できません");
    expect(failure.guidance as string).toContain("大きい尺度");
  });

  it("居ない側: 文言が改訂されても、そのまま出す", () => {
    const failure = describeRepairFailure(REWORDED_NOT_FOUND);
    expect(failure.reason).toBe("not-found");
    expect(failure.message).toBe(REWORDED_NOT_FOUND.message);
    expect(failure.message).not.toBe(NOT_FOUND.message);
    expect(failure.guidance as string).toContain("再読み込み");
  });

  it("2つの枝が、互いの文言を出していない", () => {
    expect(describeRepairFailure(REWORDED_SCALE).message).not.toBe(
      describeRepairFailure(REWORDED_NOT_FOUND).message,
    );
  });
});

/**
 * **`code` の綴りは `src/server/error.rs` の写しである**（`characterDelete.ts` と同じ形）。
 *
 * 綴りが1文字ずれると**どちらの枝にも入らず `unknown` に落ち、案内が黙って消える。**
 */
describe("`code` は写し元と一致する", () => {
  it("`src/server/error.rs` の `code` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
      "utf8",
    );

    expect(codeOf(source, "stat_scale_below_stored_value")).toBe(
      SCALE_BELOW_STORED_CODE,
    );
    expect(codeOf(source, "character_not_found")).toBe(NOT_FOUND_CODE);
    // **対照: 2つは別物**（同じ値を2回読んで緑、という形にしない）。
    expect(SCALE_BELOW_STORED_CODE).not.toBe(NOT_FOUND_CODE);
  });

  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "    pub const fn stat_scale_below_stored_value() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      '            code: Some("planted_code_for_the_control"),',
      "        }",
      "    }",
    ].join("\n");
    expect(codeOf(planted, "stat_scale_below_stored_value")).toBe(
      "planted_code_for_the_control",
    );
    expect(codeOf(planted, "stat_scale_below_stored_value")).not.toBe(
      SCALE_BELOW_STORED_CODE,
    );
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => codeOf("", "stat_scale_below_stored_value")).toThrow();

    const planted = [
      "    pub const fn stat_scale_below_stored_value() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      "        }",
      "    }",
      "",
      "    pub const fn character_not_found() -> Self {",
      "        Self {",
      '            code: Some("character_not_found"),',
      "        }",
      "    }",
    ].join("\n");
    expect(() => codeOf(planted, "stat_scale_below_stored_value")).toThrow();
  });
});

/**
 * `error.rs` のその構築子から `code` を1つ取り出す。
 *
 * **次の `pub const fn` の手前で切る**（`character-delete.test.ts` と同じ理由 ——
 * `code` を持たない構築子を尋ねたときに次の構築子の値を返さない）。
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
