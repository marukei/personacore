/**
 * **属性の置き換えの組み立て**（作業単位 D-12a / `src/lib/attributesEdit.ts`）。
 *
 * # `attributes-panel.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・要約の数・案内の文言・写しの突き合わせ | **ここ** |
 * | 画面が何を出すか・どの識別子へ送るか・失敗の見え方 | `attributes-panel.test.ts` |
 *
 * # ⚠ **これは4つ目の「丸ごと置き換え」である**
 *
 * `PUT .../attributes` は**要求に含まれない属性が消える**
 * （`api.ts` の `ReplaceAttributesRequest` / `admin.rs` の `replace_attributes`）。
 * **差分モードの作法を持ち込むと、送らなかった属性が黙って消える。**
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError } from "../src/lib/api";
import type { AttributeRevisionsView, AttributeView } from "../src/lib/api";
import {
  ATTRIBUTE_LIMITS,
  DUPLICATE_ATTRIBUTE_CODE,
  KEY_CONTROL_CODE,
  MAX_ATTRIBUTES,
  NOT_FOUND_CODE,
  NO_AFTER_LABEL,
  NO_BEFORE_LABEL,
  VALUE_CONTROL_CODE,
  afterLabel,
  attributeCounter,
  attributeFieldLabel,
  attributeSubjectLabel,
  beforeLabel,
  buildReplaceAttributes,
  describeReplaceAttributesFailure,
  draftOfAttributes,
  emptyAttributeRow,
  isOverAttributeLimit,
  keptAttributeRows,
  replacePlan,
  replaceSummary,
  revisionCountNotice,
  type AttributesDraft,
} from "../src/lib/attributesEdit";

/**
 * いまの値。
 *
 * **⚠ 主語をまたいで同じキーがある**（`年齢`）——
 * **`(subject, key)` で突き合わせていることの唯一の証拠。**
 * **キーだけで突き合わせる実装は、ここで数が変わる**（下の要約の検査）。
 *
 * **⚠ 値は全部違う綴り**（取り違えが観測できるように）。
 */
const STORED: AttributeView[] = [
  { subject: "character", key: "一人称", value: "僕" },
  { subject: "character", key: "年齢", value: "19" },
  { subject: "user", key: "年齢", value: "32" },
  { subject: "character", key: "種族", value: "猫獣人" },
  { subject: "user", key: "好物", value: "珈琲" },
];

/** 送る本文を組み立てて、成功したことまで確かめる。 */
function built(draft: AttributesDraft): { subject: string; key: string; value: string }[] {
  const result = buildReplaceAttributes(draft);
  expect(result.ok, result.ok ? "" : result.message).toBe(true);
  if (!result.ok) {
    throw new Error(result.message);
  }
  return result.body.attributes;
}

/** その行を引く。**添字ではなく `(subject, key)` で引く**（並びに寄りかからない）。 */
function rowOf(draft: AttributesDraft, subject: string, key: string): AttributesDraft["rows"][number] {
  const found = draft.rows.find((row) => row.subject === subject && row.key === key);
  expect(found, `${subject} / ${key} の行があること`).toBeDefined();
  return found as AttributesDraft["rows"][number];
}

describe("フィクスチャそのもの（同値化を避ける）", () => {
  it("主語をまたいで同じキーがあり、値は互いに違う", () => {
    const ages = STORED.filter((row) => row.key === "年齢");
    expect(ages).toHaveLength(2);
    expect(ages[0]?.subject).not.toBe(ages[1]?.subject);
    expect(ages[0]?.value).not.toBe(ages[1]?.value);
    // **値が全部違う**（どの2つを取り違えても観測できる）。
    expect(new Set(STORED.map((row) => row.value)).size).toBe(STORED.length);
  });
});

describe("いまの値を下書きに写す", () => {
  it("全件を、主語ごと写す", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    expect(draft.rows).toHaveLength(STORED.length);
    expect(draft.rows.map((row) => [row.subject, row.key, row.value])).toEqual(
      STORED.map((row) => [row.subject, row.key, row.value]),
    );
    // **全部が「保存済み」で、1つも外れていない。**
    expect(draft.rows.every((row) => row.stored)).toBe(true);
    expect(draft.rows.every((row) => !row.removed)).toBe(true);
  });

  it("足す行は、主語を決めない（推測で埋めない）", () => {
    const row = emptyAttributeRow();
    expect(row.subject).toBe("");
    expect(row.key).toBe("");
    expect(row.value).toBe("");
    expect(row.stored).toBe(false);
    expect(row.removed).toBe(false);
  });
});

describe("送る本文は、丸ごと置き換え", () => {
  /**
   * **1件だけ直しても、触っていない行まで送る。**
   *
   * **⚠ 直すのは真ん中の行**（`D-2` のレビューの実測 ——
   * 端を触ると「常に先頭」「常に最終」の変異が片側だけ通り抜ける）。
   */
  it("1件だけ直しても、触っていない行まで送る", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "user", "年齢").value = "33";

    expect(built(draft)).toEqual([
      { subject: "character", key: "一人称", value: "僕" },
      { subject: "character", key: "年齢", value: "19" },
      { subject: "user", key: "年齢", value: "33" },
      { subject: "character", key: "種族", value: "猫獣人" },
      { subject: "user", key: "好物", value: "珈琲" },
    ]);
  });

  it("外した行だけが載らない（それが「消す」の唯一の表現）", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "character", "種族").removed = true;

    expect(built(draft).map((row) => `${row.subject}/${row.key}`)).toEqual([
      "character/一人称",
      "character/年齢",
      "user/年齢",
      "user/好物",
    ]);
  });

  it("全部外すと、空の配列になる（＝全属性が消える）", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    for (const row of draft.rows) {
      row.removed = true;
    }
    expect(built(draft)).toEqual([]);
    expect(keptAttributeRows(draft)).toEqual([]);
  });

  it("値は生のまま送る（正規化はサーバの仕事）", () => {
    const draft: AttributesDraft = {
      rows: [{ subject: "character", key: " 一人称 ", value: " 僕 ", removed: false, stored: false }],
    };
    expect(built(draft)).toEqual([
      { subject: "character", key: " 一人称 ", value: " 僕 " },
    ]);
  });
});

describe("送れない下書きは、送らない", () => {
  function refused(draft: AttributesDraft): string {
    const result = buildReplaceAttributes(draft);
    expect(result.ok).toBe(false);
    if (result.ok) {
      throw new Error("送れてしまった");
    }
    return result.message;
  }

  it("主語の選ばれていない行があれば送らない", () => {
    const draft: AttributesDraft = {
      rows: [{ subject: "", key: "住所", value: "東京", removed: false, stored: false }],
    };
    const message = refused(draft);
    expect(message).toContain("主語");
    // **実在する操作だけを名指しする。**
    expect(message).toContain("「外す」");
  });

  /**
   * **知らない主語の綴りは送らない。**
   *
   * # なぜ「選ばれていない」と同じ扱いにしないのか
   *
   * `<select>` は2つしか出さないので、**この状態はサーバが3つ目の綴りを返した回にだけ起きる。**
   * そのとき「主語を選んでください」と言うのは嘘になる
   * ——**選ぶ操作では直らない**（画面に無い綴りが保存されている）。
   * **綴りをそのまま出して、何が起きているかを言う。**
   */
  it("知らない主語の綴りは送らない（綴りをそのまま出す）", () => {
    const draft: AttributesDraft = {
      rows: [
        { subject: "assistant", key: "住所", value: "東京", removed: false, stored: true },
      ],
    };
    const message = refused(draft);
    expect(message).toContain("assistant");
    // **「選ばれていない」とは別の文言**（直し方が違う）。
    expect(message).not.toBe(refused({
      rows: [{ subject: "", key: "住所", value: "東京", removed: false, stored: false }],
    }));
  });

  it("キーが空の行があれば送らない", () => {
    const draft: AttributesDraft = {
      rows: [{ subject: "user", key: "  ", value: "東京", removed: false, stored: false }],
    };
    expect(refused(draft)).toContain("キー");
  });

  it("値が空の行があれば送らない（どのキーの話かを名乗る）", () => {
    const draft: AttributesDraft = {
      rows: [{ subject: "user", key: "住所", value: "   ", removed: false, stored: false }],
    };
    const message = refused(draft);
    expect(message).toContain("住所");
    expect(message).toContain("値");
  });

  it("上限を超えたキーは送らない（キーの上限を名乗る）", () => {
    const draft: AttributesDraft = {
      rows: [
        {
          subject: "user",
          key: "あ".repeat(ATTRIBUTE_LIMITS.key + 1),
          value: "東京",
          removed: false,
          stored: false,
        },
      ],
    };
    const message = refused(draft);
    expect(message).toContain("キー");
    expect(message).toContain(String(ATTRIBUTE_LIMITS.key));
    // **他の欄の上限を名乗らない**（上限の帰属 / `character-edit.test.ts` の先例）。
    expect(message).not.toContain(String(ATTRIBUTE_LIMITS.value));
  });

  it("上限を超えた値は送らない（値の上限を名乗る）", () => {
    const draft: AttributesDraft = {
      rows: [
        {
          subject: "user",
          key: "住所",
          value: "あ".repeat(ATTRIBUTE_LIMITS.value + 1),
          removed: false,
          stored: false,
        },
      ],
    };
    const message = refused(draft);
    expect(message).toContain("値");
    expect(message).toContain(String(ATTRIBUTE_LIMITS.value));
    expect(message).not.toContain(String(ATTRIBUTE_LIMITS.key));
  });

  it("外した行の中身は、組み立てを止めない（送らない行だから）", () => {
    const draft: AttributesDraft = {
      rows: [
        { subject: "", key: "", value: "", removed: true, stored: true },
        { subject: "user", key: "住所", value: "東京", removed: false, stored: false },
      ],
    };
    expect(built(draft)).toEqual([{ subject: "user", key: "住所", value: "東京" }]);
  });

  it("件数の上限を超えたら送らない（上限と、いまの件数を名乗る）", () => {
    const rows = Array.from({ length: MAX_ATTRIBUTES + 1 }, (_, index) => ({
      subject: "character",
      key: `キー${index}`,
      value: `値${index}`,
      removed: false,
      stored: false,
    }));
    const message = refused({ rows });
    expect(message).toContain(String(MAX_ATTRIBUTES));
    expect(message).toContain(String(MAX_ATTRIBUTES + 1));
  });

  it("上限ちょうどは送れる（境界の反対側）", () => {
    const rows = Array.from({ length: MAX_ATTRIBUTES }, (_, index) => ({
      subject: "character",
      key: `キー${index}`,
      value: `値${index}`,
      removed: false,
      stored: false,
    }));
    expect(built({ rows })).toHaveLength(MAX_ATTRIBUTES);
  });

  it("外した行は件数に数えない", () => {
    const rows = Array.from({ length: MAX_ATTRIBUTES + 2 }, (_, index) => ({
      subject: "character",
      key: `キー${index}`,
      value: `値${index}`,
      removed: index < 2,
      stored: true,
    }));
    expect(built({ rows })).toHaveLength(MAX_ATTRIBUTES);
  });
});

describe("文字数の欄", () => {
  it("符号位置で数える（UTF-16 の個数ではない）", () => {
    // **絵文字は1文字。** `.length` なら 2 になる。
    expect(attributeCounter("key", "🐈")).toBe(`1 / ${ATTRIBUTE_LIMITS.key}`);
  });

  it("欄ごとに自分の上限を出す（他の欄の上限を出さない）", () => {
    expect(attributeCounter("key", "あ")).toBe(`1 / ${ATTRIBUTE_LIMITS.key}`);
    expect(attributeCounter("value", "あ")).toBe(`1 / ${ATTRIBUTE_LIMITS.value}`);
    // **2つの上限は別の数**（同値なら取り違えが観測できない）。
    expect(ATTRIBUTE_LIMITS.key).not.toBe(ATTRIBUTE_LIMITS.value);
  });

  it("欄の見出しは、欄ごとに別の綴り", () => {
    expect(attributeFieldLabel("key")).not.toBe(attributeFieldLabel("value"));
  });

  it("上限ちょうどは越えていない。1文字多いと越えている", () => {
    expect(isOverAttributeLimit("key", "あ".repeat(ATTRIBUTE_LIMITS.key))).toBe(false);
    expect(isOverAttributeLimit("key", "あ".repeat(ATTRIBUTE_LIMITS.key + 1))).toBe(true);
    expect(isOverAttributeLimit("value", "あ".repeat(ATTRIBUTE_LIMITS.value))).toBe(false);
    expect(isOverAttributeLimit("value", "あ".repeat(ATTRIBUTE_LIMITS.value + 1))).toBe(true);
  });

  it("前後の空白は数えない（サーバも正規化してから数える）", () => {
    expect(attributeCounter("key", `  ${"あ".repeat(ATTRIBUTE_LIMITS.key)}  `)).toBe(
      `${ATTRIBUTE_LIMITS.key} / ${ATTRIBUTE_LIMITS.key}`,
    );
  });
});

/**
 * **要約の数は、サーバが履歴に書く差分と同じ数え方である。**
 *
 * 写し元は `src/domain/attribute.rs` の `attribute_changes`:
 * **`(subject, key)` の組で突き合わせ、値まで同じなら記録しない。**
 */
describe("保存すると何が記録されるか、押す前に言う", () => {
  /**
   * **追加・変更・削除を同時に含む下書き。**
   *
   * **⚠ `user / 年齢` を `19` にしてある** ——
   * **`character / 年齢` の保存値と同じ綴り**なので、
   * **キーだけで突き合わせる実装では「変わっていない」と読まれて数が減る**
   * （組で突き合わせていれば `32 → 19` の変更である）。
   */
  function mixed(): AttributesDraft {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "character", "年齢").value = "20";
    rowOf(draft, "user", "年齢").value = "19";
    rowOf(draft, "character", "一人称").removed = true;
    rowOf(draft, "character", "種族").removed = true;
    rowOf(draft, "user", "好物").removed = true;
    draft.rows.push({
      subject: "user",
      key: "住所",
      value: "東京",
      removed: false,
      stored: false,
    });
    return draft;
  }

  it("追加・変更・削除を、それぞれ数える", () => {
    const plan = replacePlan(STORED, mixed());
    // **3つとも別の数**（入れ替える変異が観測できる）。
    expect(plan.added).toBe(1);
    expect(plan.changed).toBe(2);
    expect(plan.removed.map((row) => `${row.subject}/${row.key}`)).toEqual([
      "character/一人称",
      "character/種族",
      "user/好物",
    ]);
    expect(plan.kept).toBe(3);
  });

  it("要約は、3つの数を見出しつきで言う", () => {
    expect(replaceSummary(STORED, mixed())).toBe(
      "保存すると、この属性は 3 件になります。" +
        "変更の記録には 追加 1 件 / 変更 2 件 / 削除 3 件 が残ります。" +
        "消えるのは『キャラクターの一人称』『キャラクターの種族』『利用者の好物』です" +
        "（置き換えなので、送らない行は消えます）。",
    );
  });

  /**
   * **主語をまたいだ差分を出さない**（ADR-0028 §決定4 の帰属）。
   *
   * **同じキーを主語だけ違えて2件持つ状態**で、
   * **片方だけを直したら、直した側の1件しか変更に数えない。**
   */
  it("同じキーでも、主語が違えば別の属性として数える", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "user", "年齢").value = "33";
    const plan = replacePlan(STORED, draft);
    expect(plan).toEqual({ kept: 5, added: 0, changed: 1, removed: [] });

    // **もう片方を直しても、同じく1件**（キーだけで突き合わせていたら
    // どちらかが「もう一方の値」と比べられる）。
    const other = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(other, "character", "年齢").value = "20";
    expect(replacePlan(STORED, other)).toEqual({
      kept: 5,
      added: 0,
      changed: 1,
      removed: [],
    });
  });

  it("主語を替えるのは「削除 ＋ 追加」である（改名と同じ形）", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "user", "好物").subject = "character";
    const plan = replacePlan(STORED, draft);
    expect(plan.added).toBe(1);
    expect(plan.changed).toBe(0);
    expect(plan.removed.map((row) => `${row.subject}/${row.key}`)).toEqual([
      "user/好物",
    ]);
  });

  it("同じ内容なら、記録は増えないと言う", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    const plan = replacePlan(STORED, draft);
    expect(plan).toEqual({ kept: 5, added: 0, changed: 0, removed: [] });
    const summary = replaceSummary(STORED, draft);
    expect(summary).toContain("いまの内容と同じ");
    expect(summary).toContain("変更の記録は増えません");
  });

  it("前後の空白を足しただけでは、変更に数えない（サーバも正規化する）", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    rowOf(draft, "character", "種族").value = "  猫獣人  ";
    expect(replacePlan(STORED, draft).changed).toBe(0);
  });

  it("1件も残らないときは、そう言う", () => {
    const draft = draftOfAttributes({ character: "mio", attributes: STORED });
    for (const row of draft.rows) {
      row.removed = true;
    }
    const summary = replaceSummary(STORED, draft);
    expect(summary).toContain("1件も残りません");
    expect(summary).toContain("削除 5 件");
  });
});

describe("主語の見出し", () => {
  it("2つは別の綴り", () => {
    expect(attributeSubjectLabel("user")).toBe("利用者");
    expect(attributeSubjectLabel("character")).toBe("キャラクター");
    expect(attributeSubjectLabel("user")).not.toBe(
      attributeSubjectLabel("character"),
    );
  });

  /**
   * **知らない綴りは、そのまま返す。**
   *
   * 属性は**削れない枠に毎ターン載る**ので、
   * **画面から主語の欄が消えるのがいちばん質の悪い壊れ方**である
   * （`statsEdit.ts` の `runModeLabel` と同じ立場）。
   */
  it("知らない綴りは、そのまま返す", () => {
    expect(attributeSubjectLabel("assistant")).toBe("assistant");
    expect(attributeSubjectLabel("")).toBe("");
  });
});

describe("履歴の前後の見出し", () => {
  it("`null` は「無かった」「消えた」で、2つは別の綴り", () => {
    expect(beforeLabel(null)).toBe(NO_BEFORE_LABEL);
    expect(afterLabel(null)).toBe(NO_AFTER_LABEL);
    // **同じ綴りにすると、追加と削除が画面で同じ行になる。**
    expect(NO_BEFORE_LABEL).not.toBe(NO_AFTER_LABEL);
  });

  it("値はそのまま出す（空文字はサーバが作らないので、そのまま）", () => {
    expect(beforeLabel("僕")).toBe("僕");
    expect(afterLabel("私")).toBe("私");
  });
});

describe("履歴の件数の案内", () => {
  function view(
    total: number,
    limit: number,
    count: number,
  ): AttributeRevisionsView {
    return {
      character: "mio",
      revisions: Array.from({ length: count }, (_, index) => ({
        id: index + 1,
        subject: "character",
        key: "一人称",
        before: "僕",
        after: "私",
        created_at: "2026-09-09T00:00:00Z",
      })),
      total_revisions: total,
      revisions_limit: limit,
    };
  }

  it("1件も無ければ、そう言う", () => {
    const notice = revisionCountNotice(view(0, 200, 0));
    expect(notice).toContain("まだ変更はありません");
  });

  /**
   * **総数と、出した件数と、上限と、残りを全部言う**（`DriftView` の先例）。
   *
   * # ⚠ **数だけを探すと、役の入れ替えが観測できない**
   *
   * `toContain("208")` は**その数が文中のどこかに在る**しか言わない ——
   * **総数と出した件数を入れ替える変異が 959 全緑を通った**
   * （D-12a のレビューが実測 / 2026-09-09）。
   * **数は必ず、それを名乗る語と一緒に探すこと。**
   */
  it("上限で切られているときは、総数と出した件数を両方言う", () => {
    const notice = revisionCountNotice(view(208, 200, 200));
    // **役に結ぶ**（数だけを探さない）。
    expect(notice).toContain("全 208 件");
    expect(notice).toContain("新しい 200 件");
    expect(notice).toContain("残り 8 件");
  });

  /**
   * **⚠ 上限と出した件数を見分けられるのは、この腕だけ。**
   *
   * 切られている状態では**出した件数と上限が必ず同じ数**になるので、
   * **その2つを入れ替える変異は上の腕では観測できない**
   * （フィクスチャではなく、状態が同値化を作っている）。
   * **ここは 3 件と 200 件で違うので、上限の役をここで押さえる。**
   */
  it("全部出しているときは、そう言う", () => {
    const notice = revisionCountNotice(view(3, 200, 3));
    expect(notice).toContain("全 3 件");
    expect(notice).toContain("上限は 200 件");
    expect(notice).toContain("すべて出しています");
    expect(notice).not.toContain("ここに出ていません");
  });
});

describe("置き換えの失敗を読み分ける", () => {
  function failure(code: string | null): ReturnType<
    typeof describeReplaceAttributesFailure
  > {
    return describeReplaceAttributesFailure(
      new ApiRequestError("サーバが言ったこと", 400, "invalid_request_error", code),
    );
  }

  it("重複は、直す手を言う", () => {
    const described = failure(DUPLICATE_ATTRIBUTE_CODE);
    expect(described.reason).toBe("duplicate");
    expect(described.message).toBe("サーバが言ったこと");
    expect(described.guidance).not.toBeNull();
    expect(described.guidance as string).toContain("「外す」");
  });

  it("制御文字は、キー側でも値側でも同じ手を言う", () => {
    for (const code of [KEY_CONTROL_CODE, VALUE_CONTROL_CODE]) {
      const described = failure(code);
      expect(described.reason, code).toBe("control");
      expect(described.guidance, code).not.toBeNull();
    }
    // **2つの綴りは別物**（同じ定数を2回読んで緑、という形にしない）。
    expect(KEY_CONTROL_CODE).not.toBe(VALUE_CONTROL_CODE);
  });

  it("居ないと言われたら、取り直しを言う", () => {
    const described = failure(NOT_FOUND_CODE);
    expect(described.reason).toBe("not-found");
    expect(described.guidance as string).toContain("「キャラクターの一覧へ」");
  });

  /**
   * **分類できなかった失敗に、次の一手を書かない。**
   *
   * サーバの文言は**壊れている欄を自分で名乗る**（`attribute_key_too_long` など）ので、
   * こちらから言い換えると情報が減るだけになる（`statsEdit.ts` と同じ判断）。
   */
  it("知らない `code` は、文言だけを出して手を書かない", () => {
    const described = failure("attribute_key_too_long");
    expect(described.reason).toBe("unknown");
    expect(described.message).toBe("サーバが言ったこと");
    expect(described.guidance).toBeNull();
  });

  it("`code` が無い失敗も、手を書かない", () => {
    expect(failure(null).reason).toBe("unknown");
    expect(failure(null).guidance).toBeNull();
  });

  /** **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約）。 */
  it("知らない失敗は、内部の詳細を出さない", () => {
    const described = describeReplaceAttributesFailure(
      new Error("C:\\Users\\someone\\personacore.sqlite が開けません"),
    );
    expect(described.reason).toBe("unknown");
    expect(described.message).toBe("予期しない失敗です");
    expect(described.message).not.toContain("sqlite");
    expect(described.guidance).toBeNull();
  });

  it("4つの理由は、互いに別の綴り", () => {
    expect(
      new Set([
        DUPLICATE_ATTRIBUTE_CODE,
        KEY_CONTROL_CODE,
        VALUE_CONTROL_CODE,
        NOT_FOUND_CODE,
      ]).size,
    ).toBe(4);
  });
});

/**
 * **上限は `src/domain/attribute.rs` の写しである。**
 *
 * # ⚠ 写し元は `character.rs` ではない
 *
 * `characterEdit.ts` の `LIMITS` は `src/domain/character.rs` を読むが、
 * **属性の上限はそちらに無い**（`AttributeKey` / `AttributeValue` / `AttributeSet` は
 * 別のモジュール）。**読む先を間違えると、この検査は「探せていないから緑」になる**
 * ——だから**見つからなければ落ちる**対照を対にしてある（`15b`）。
 *
 * # ずれると何が消えるのか
 *
 * **型ではなく値**なので、ずれても型検査は通る。
 * **画面が嘘の数を出す**（「12 文字まで」と書いてあるのにサーバが 400 を返す、
 * あるいは**その逆で、通る長さを画面が止める**）。
 */
describe("上限は写し元と一致する", () => {
  const ATTRIBUTE_RS = readFileSync(
    fileURLToPath(new URL("../../src/domain/attribute.rs", import.meta.url)),
    "utf8",
  );

  it("`src/domain/attribute.rs` の上限と一致する", () => {
    expect(constOf(ATTRIBUTE_RS, "AttributeKey", "MAX_CHARS")).toBe(
      ATTRIBUTE_LIMITS.key,
    );
    expect(constOf(ATTRIBUTE_RS, "AttributeValue", "MAX_CHARS")).toBe(
      ATTRIBUTE_LIMITS.value,
    );
    expect(constOf(ATTRIBUTE_RS, "AttributeSet", "MAX_LEN")).toBe(MAX_ATTRIBUTES);
  });

  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "impl AttributeKey {",
      "    pub const MAX_CHARS: usize = 99;",
      "}",
      "impl AttributeSet {",
      "    pub const MAX_LEN: usize = 3;",
      "}",
    ].join("\n");
    expect(constOf(planted, "AttributeKey", "MAX_CHARS")).toBe(99);
    expect(constOf(planted, "AttributeSet", "MAX_LEN")).toBe(3);
    // **実ファイルの値ではない**（同じ値を2回読んで緑、という形にしない）。
    expect(constOf(planted, "AttributeKey", "MAX_CHARS")).not.toBe(
      ATTRIBUTE_LIMITS.key,
    );
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => constOf("", "AttributeKey", "MAX_CHARS")).toThrow();
    // **次の `impl` の値を拾ってこない。**
    const planted = [
      "impl AttributeKey {",
      "}",
      "impl AttributeValue {",
      "    pub const MAX_CHARS: usize = 20;",
      "}",
    ].join("\n");
    expect(() => constOf(planted, "AttributeKey", "MAX_CHARS")).toThrow();
    expect(constOf(planted, "AttributeValue", "MAX_CHARS")).toBe(20);
  });
});

/**
 * **`code` の綴りも写しである**（`src/server/error.rs`）。
 *
 * 綴りが1文字ずれると**どの枝にも入らず `unknown` に落ち、案内が黙って消える**
 * （`statsEdit.ts` / `characterDelete.ts` と同じ性質）。
 */
describe("`code` は写し元と一致する", () => {
  const ERROR_RS = readFileSync(
    fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
    "utf8",
  );

  it("`src/server/error.rs` の `code` と一致する", () => {
    expect(codeOf(ERROR_RS, "duplicate_attribute")).toBe(DUPLICATE_ATTRIBUTE_CODE);
    expect(codeOf(ERROR_RS, "character_not_found")).toBe(NOT_FOUND_CODE);
    // **理由ごとに分岐する構築子**（綴りは `match` の腕にある）。
    expect(
      codeInArm(ERROR_RS, "invalid_attribute_key", "AttributeKeyError::Control"),
    ).toBe(KEY_CONTROL_CODE);
    expect(
      codeInArm(
        ERROR_RS,
        "invalid_attribute_value",
        "AttributeValueError::Control",
      ),
    ).toBe(VALUE_CONTROL_CODE);
  });

  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "    pub const fn duplicate_attribute() -> Self {",
      "        Self {",
      '            code: Some("planted_code_for_the_control"),',
      "        }",
      "    }",
    ].join("\n");
    expect(codeOf(planted, "duplicate_attribute")).toBe(
      "planted_code_for_the_control",
    );
    expect(codeOf(planted, "duplicate_attribute")).not.toBe(
      DUPLICATE_ATTRIBUTE_CODE,
    );
  });

  it("対照: 分岐する構築子でも、植えた腕の値を読む", () => {
    const planted = [
      "    pub const fn invalid_attribute_key(reason: AttributeKeyError) -> Self {",
      "        let (message, code) = match reason {",
      '            AttributeKeyError::Empty => ("空です", "planted_empty_arm"),',
      "            AttributeKeyError::Control => (",
      '                "改行や制御文字は使えません",',
      '                "planted_control_arm",',
      "            ),",
      "        };",
      "    }",
    ].join("\n");
    expect(
      codeInArm(planted, "invalid_attribute_key", "AttributeKeyError::Control"),
    ).toBe("planted_control_arm");
    // **腕ごとに別の値を読む**（最初の腕を返す実装を落とす）。
    expect(
      codeInArm(planted, "invalid_attribute_key", "AttributeKeyError::Empty"),
    ).toBe("planted_empty_arm");
  });

  it("対照: 見つからなければ落ちる", () => {
    expect(() => codeOf("", "duplicate_attribute")).toThrow();
    expect(() =>
      codeInArm("", "invalid_attribute_key", "AttributeKeyError::Control"),
    ).toThrow();
    // **次の構築子の値を拾ってこない。**
    const planted = [
      "    pub const fn duplicate_attribute() -> Self {",
      "        Self {",
      "            status: StatusCode::BAD_REQUEST,",
      "        }",
      "    }",
      "",
      "    pub const fn character_not_found() -> Self {",
      "        Self {",
      '            code: Some("character_not_found"),',
      "        }",
      "    }",
    ].join("\n");
    expect(() => codeOf(planted, "duplicate_attribute")).toThrow();
  });
});

/**
 * その型の `impl` ブロックから定数を1つ取り出す。
 *
 * **次の `impl` の手前で切る。** 切らないと、その定数を持たない型を尋ねたときに
 * **次の型の値を返してしまう**（探せていないのに緑になる /
 * `character-edit.test.ts` の `maxCharsOf` と同じ罠）。
 */
function constOf(source: string, typeName: string, constName: string): number {
  const start = source.indexOf(`impl ${typeName} {`);
  if (start < 0) {
    throw new Error(`impl ${typeName} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("\nimpl ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = new RegExp(`pub const ${constName}: usize = (\\d+);`).exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`impl ${typeName} に ${constName} がありません`);
  }
  return Number.parseInt(found[1], 10);
}

/**
 * `error.rs` のその構築子から `code` を1つ取り出す。
 *
 * **次の `pub const fn` の手前で切る**（`stats-edit.test.ts` の `codeOf` と同じ形）。
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
 * `code: Some(code)` なので [`codeOf`] では読めない ——
 * 綴りは `match` の腕にある。**腕の中の、英小文字と `_` だけの文字列**が `code`
 * （もう一方は日本語の文言）。
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
