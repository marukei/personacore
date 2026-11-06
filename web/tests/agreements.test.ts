/**
 * **約束・決まりの画面の純粋な部分**（作業単位 A-12a / `src/lib/agreements.ts`）。
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・上限の案内・失敗の読み分け・写しの突き合わせ | **ここ** |
 * | 画面が何を出すか・どの識別子へ送るか | `agreements-panel.test.ts` |
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError, type AgreementView, type AgreementsView } from "../src/lib/api";
import {
  AGREEMENT_CONTENT_CONTROL_CODE,
  AGREEMENT_CONTENT_MAX_CHARS,
  AGREEMENT_ENDINGS,
  AGREEMENT_KEEPERS,
  AGREEMENT_KINDS,
  AGREEMENT_LIMIT_REACHED_CODE,
  AGREEMENT_NOT_ACTIVE_CODE,
  AGREEMENT_NOT_FOUND_CODE,
  AGREEMENT_NOT_PROPOSED_CODE,
  CHARACTER_NOT_FOUND_CODE,
  activeAgreements,
  adoptDraftOf,
  agreementKeeperLabel,
  agreementKindLabel,
  agreementLimitNotice,
  agreementStateLabel,
  buildAdoptAgreement,
  buildCreateAgreement,
  describeAgreementFailure,
  describeProposalFailure,
  droppedNotice,
  endedAgreements,
  isAtAgreementLimit,
  proposalKindNote,
  proposalLimitNotice,
  proposedAgreements,
  transitionBeforeLabel,
  transitionCountNotice,
  transitionKindLabel,
} from "../src/lib/agreements";

/** 3件の一覧（**有効・終わった・有効** —— 真ん中だけが終わっている）。上限は 3。 */
function view(states: string[], limit: number): AgreementsView {
  return {
    character: "mio",
    agreements: states.map((state, index) => ({
      id: index + 1,
      content: `取り決め${index}`,
      kind: index % 2 === 0 ? "rule" : "promise",
      keeper: (["user", "character", "both"] as const)[index % 3] ?? "user",
      state: state as AgreementsView["agreements"][number]["state"],
      source_session_id: null,
      created_at: "2026-10-22T00:00:00Z",
    })),
    active_limit: limit,
    last_turn: null,
  };
}

describe("直近のターンで落ちた件数の案内", () => {
  const at = (raw: string): string => `<${raw}>`;

  it("落ちたときだけ、件数と時刻を言う", () => {
    const listed = view(["active", "active"], 3);
    expect(droppedNotice(listed, at)).toBeNull();
    expect(
      droppedNotice({ ...listed, last_turn: { included: 5, dropped: 0, at: "t" } }, at),
    ).toBeNull();
    expect(
      droppedNotice({ ...listed, last_turn: { included: 5, dropped: 2, at: "t" } }, at),
    ).toBe(
      "直近のターン（<t>）では、有効な約束・決まりのうち古い 2 件が" +
        "本文に載りませんでした（予算に入りきらなかったため。載ったのは新しい 5 件）。" +
        "古いものを終わらせると、残りが載りやすくなります。",
    );
  });
});

describe("本文の組み立て", () => {
  it("中身・種類・守る側が揃えば、そのまま送る（正規化しない）", () => {
    expect(
      buildCreateAgreement({ content: " 毎朝あいさつする ", kind: "rule", keeper: "both" }),
    ).toEqual({
      ok: true,
      body: { content: " 毎朝あいさつする ", kind: "rule", keeper: "both" },
    });
  });

  it("空・長すぎ・種類と守る側の未選択は送らない", () => {
    expect(buildCreateAgreement({ content: "　 ", kind: "rule", keeper: "user" }).ok).toBe(
      false,
    );
    expect(
      buildCreateAgreement({
        content: "約".repeat(AGREEMENT_CONTENT_MAX_CHARS + 1),
        kind: "rule",
        keeper: "user",
      }),
    ).toEqual({ ok: false, message: "中身が長すぎます（上限 60 文字）。" });
    expect(
      buildCreateAgreement({
        content: "約".repeat(AGREEMENT_CONTENT_MAX_CHARS),
        kind: "rule",
        keeper: "user",
      }).ok,
    ).toBe(true);
    expect(buildCreateAgreement({ content: "約束", kind: "", keeper: "user" })).toEqual({
      ok: false,
      message: "約束か決まりかを選んでください。",
    });
    expect(buildCreateAgreement({ content: "約束", kind: "rule", keeper: "" })).toEqual({
      ok: false,
      message: "誰が守るのかを選んでください。",
    });
    // **知らない綴りは押し込まない**（`as` で送らない）。
    expect(buildCreateAgreement({ content: "約束", kind: "rule", keeper: "assistant" }).ok).toBe(
      false,
    );
  });
});

/** 提案1件（**種類はいつも `rule`**。サーバは提案の種類を判定しない）。 */
const PROPOSAL: AgreementView = {
  id: 32,
  content: "寝る前におやすみを言う",
  kind: "rule",
  keeper: "both",
  state: "proposed",
  source_session_id: 7,
  created_at: "2026-10-23T00:00:00Z",
};

describe("提案を採る本文の組み立て（作業単位 A-12c-2）", () => {
  /**
   * **直す下書きは、中身と守る側を提案から写し、種類は空で始める**（「未判定」を既定にしない）。
   * 種類を提案から写すと、判定していない「決まり」を利用者が選んだかのように送る。
   */
  it("直す下書きは中身と守る側を写し、種類は選んでいない状態で始める", () => {
    expect(adoptDraftOf(PROPOSAL)).toEqual({
      content: "寝る前におやすみを言う",
      kind: "",
      keeper: "both",
    });
  });

  it("中身・種類・守る側が揃えば、その3つを送る（正規化しない）", () => {
    expect(
      buildAdoptAgreement({ content: " 寝る前に電話する ", kind: "promise", keeper: "user" }),
    ).toEqual({
      ok: true,
      body: { content: " 寝る前に電話する ", kind: "promise", keeper: "user" },
    });
  });

  it("空・長すぎ・種類と守る側の未選択は送らない（書くときと同じ理由）", () => {
    expect(buildAdoptAgreement({ content: " ", kind: "rule", keeper: "user" })).toEqual({
      ok: false,
      message: "中身が空です。",
    });
    expect(
      buildAdoptAgreement({
        content: "約".repeat(AGREEMENT_CONTENT_MAX_CHARS + 1),
        kind: "rule",
        keeper: "user",
      }).ok,
    ).toBe(false);
    expect(buildAdoptAgreement({ content: "約束", kind: "", keeper: "user" })).toEqual({
      ok: false,
      message: "約束か決まりかを選んでください。",
    });
    expect(buildAdoptAgreement({ content: "約束", kind: "rule", keeper: "" })).toEqual({
      ok: false,
      message: "誰が守るのかを選んでください。",
    });
  });

  /**
   * **記録の種類は、その取り決めがいま提案中・却下なら「未判定」**。採ったもの・一覧に無いもの・
   * 一覧が読めていないときは記録の種類のまま（3件にして真ん中だけが提案中）。
   */
  it("記録の種類は、いま提案中・却下の取り決めだけ「未判定」", () => {
    const listed = view(["active", "proposed", "rejected"], 16).agreements;
    const row = (agreementId: number) => ({
      id: 900 + agreementId,
      agreement_id: agreementId,
      content: "c",
      kind: "rule" as const,
      keeper: "user" as const,
      before: null,
      after: "proposed" as const,
      created_at: "2026-10-23T00:00:00Z",
    });
    expect([1, 2, 3, 4].map((id) => transitionKindLabel(row(id), listed))).toEqual([
      "決まり",
      "未判定",
      "未判定",
      "決まり",
    ]);
    expect(transitionKindLabel(row(2), null)).toBe("決まり");
  });

  /** **種類は判定していないことを言い、そのまま採ったら何になるかは応答の `kind` から言う**（写さない）。 */
  it("種類の注記は「未判定」と、そのまま採ったときの種類を言う", () => {
    expect(proposalKindNote(PROPOSAL)).toBe(
      "種類は未判定です（そのまま採ると「決まり」になります。違うなら「直して採る」で選べます）。",
    );
    expect(proposalKindNote({ ...PROPOSAL, kind: "promise" })).toBe(
      "種類は未判定です（そのまま採ると「約束」になります。違うなら「直して採る」で選べます）。",
    );
  });
});

describe("提案を採る・捨てるときの失敗の読み分け（作業単位 A-12c-2）", () => {
  const failed = (code: string): ApiRequestError =>
    new ApiRequestError("サーバの文言", 409, "invalid_request_error", code);

  /** **黙って失敗しない** —— 上限なら「古いものを終わらせてください」と言い、提案が残っていることも言う。 */
  it("上限は、古いものを終わらせる一手を言う（提案は残る）", () => {
    expect(describeProposalFailure(failed(AGREEMENT_LIMIT_REACHED_CODE))).toEqual({
      reason: "limit",
      message: "サーバの文言",
      guidance:
        "有効な約束・決まりが上限に達しているので、採れませんでした。" +
        "古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。" +
        "終わらせてから、もう一度採れます（提案はそのまま残っています）。",
    });
  });

  it("理由ごとに別の種別。分類できないものに一手を書かない", () => {
    expect(describeProposalFailure(failed(AGREEMENT_NOT_PROPOSED_CODE))).toEqual({
      reason: "decided",
      message: "サーバの文言",
      guidance: "別の画面で先に採ったか、捨てたのかもしれません。一覧を取り直しました。",
    });
    expect(describeProposalFailure(failed(AGREEMENT_NOT_FOUND_CODE)).reason).toBe("not-found");
    expect(describeProposalFailure(failed(CHARACTER_NOT_FOUND_CODE)).reason).toBe("not-found");
    expect(describeProposalFailure(failed(AGREEMENT_CONTENT_CONTROL_CODE)).reason).toBe(
      "control",
    );
    // **書くときの「終わっている」は、採るときには起きない**（採る口は `agreement_not_active` を返さない）。
    expect(describeProposalFailure(failed(AGREEMENT_NOT_ACTIVE_CODE)).reason).toBe("unknown");
    expect(describeProposalFailure(new Error("内部"))).toEqual({
      reason: "unknown",
      message: "予期しない失敗です",
      guidance: null,
    });
  });
});

describe("有効と終わったものの分け方・上限", () => {
  it("真ん中だけが終わっていれば、有効は前後の2件、終わったものは真ん中の1件", () => {
    const listed = view(["active", "fulfilled", "active"], 3);
    expect(activeAgreements(listed).map((agreement) => agreement.id)).toEqual([1, 3]);
    expect(endedAgreements(listed).map((agreement) => agreement.id)).toEqual([2]);
  });

  /**
   * **提案中・却下は「終わったもの」ではない**（作業単位 A-12c-2）。
   * 提案中は「提案」の欄、却下は記録の側にだけ出る。**知らない綴りは今までどおり終わった側**
   * （サーバが状態を足した回に、行を黙って消さない）。
   */
  it("提案中は提案の側、却下はどちらにも出ない。終わった3つと知らない綴りは終わった側", () => {
    const listed = view(
      ["proposed", "active", "rejected", "broken", "proposed", "withdrawn", "expired", "fulfilled"],
      16,
    );
    expect(proposedAgreements(listed).map((agreement) => agreement.id)).toEqual([1, 5]);
    expect(activeAgreements(listed).map((agreement) => agreement.id)).toEqual([2]);
    expect(endedAgreements(listed).map((agreement) => agreement.id)).toEqual([4, 6, 7, 8]);
  });

  it("提案中は上限に数えない", () => {
    expect(isAtAgreementLimit(view(["active", "proposed", "active"], 3))).toBe(false);
    expect(isAtAgreementLimit(view(["active", "proposed", "active"], 2))).toBe(true);
  });

  it("上限に達していたら、提案を採る前に古いものを終わらせるよう促す。達していなければ何も言わない", () => {
    expect(proposalLimitNotice(view(["active", "proposed", "active"], 3))).toBeNull();
    expect(proposalLimitNotice(view(["active", "proposed", "active"], 2))).toBe(
      "有効な約束・決まりが上限の 2 件に達しているので、いまは採れません。古いものを終わらせてください。",
    );
  });

  it("上限は応答の `active_limit` で数え、有効なものだけを数える", () => {
    expect(isAtAgreementLimit(view(["active", "fulfilled", "active"], 3))).toBe(false);
    expect(isAtAgreementLimit(view(["active", "active", "active"], 3))).toBe(true);
    // **写しを持っていない**（上限 2 なら 2 件で止まる）。
    expect(isAtAgreementLimit(view(["active", "broken", "active"], 2))).toBe(true);
  });

  it("上限に達したら、古いものを終わらせるよう促す", () => {
    expect(agreementLimitNotice(view(["active", "fulfilled", "active"], 3))).toBe(
      "有効な約束・決まりは 3 件までです（いま 2 件）。",
    );
    expect(agreementLimitNotice(view(["active", "active", "active"], 3))).toBe(
      "有効な約束・決まりが上限の 3 件に達しています。" +
        "新しく書く前に、古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。",
    );
  });
});

describe("見出し", () => {
  it("種類・守る側・状態はそれぞれ別の綴り。知らない綴りはそのまま出す", () => {
    expect(AGREEMENT_KINDS.map(agreementKindLabel)).toEqual(["約束", "決まり"]);
    expect(AGREEMENT_KEEPERS.map(agreementKeeperLabel)).toEqual([
      "利用者が守る",
      "キャラクターが守る",
      "両方が守る",
    ]);
    expect(AGREEMENT_ENDINGS.map(agreementStateLabel)).toEqual([
      "果たした",
      "破った",
      "取り消した",
    ]);
    expect(agreementStateLabel("active")).toBe("有効");
    // **提案中・却下は作業単位 A-12c-2 で見出しを持った**（それまでは知らない綴りの例だった）。
    expect(agreementStateLabel("proposed")).toBe("提案中");
    expect(agreementStateLabel("rejected")).toBe("却下");
    expect(agreementStateLabel("expired")).toBe("expired");
    expect(agreementKeeperLabel("someone")).toBe("someone");
    expect(transitionBeforeLabel(null)).toBe("（書いた）");
    expect(transitionBeforeLabel("active")).toBe("有効");
  });

  /**
   * **「無かった → 提案中」は「書いた」と言わない**（作業単位 A-12c-2）。提案は利用者が書いたものではない。
   * **「無かった → 有効」は今までどおり「書いた」**（同じ `null` で、後の側だけが違う対）。
   */
  it("記録の「前」が無いとき、後が提案中なら「提案された」、有効なら「書いた」", () => {
    expect(transitionBeforeLabel(null, "proposed")).toBe("（提案された）");
    expect(transitionBeforeLabel(null, "active")).toBe("（書いた）");
    // **前があるなら、後の側は見ない。**
    expect(transitionBeforeLabel("proposed", "rejected")).toBe("提案中");
  });

  it("記録の総数を必ず言う（上限は応答のもの）", () => {
    expect(
      transitionCountNotice({
        character: "mio",
        transitions: [],
        total_transitions: 0,
        transitions_limit: 200,
      }),
    ).toBe("まだ記録はありません。");
  });
});

describe("失敗の読み分け", () => {
  const failed = (code: string): ApiRequestError =>
    new ApiRequestError("サーバの文言", 409, "invalid_request_error", code);

  it("上限は、古いものを終わらせる一手を言う", () => {
    const described = describeAgreementFailure(failed(AGREEMENT_LIMIT_REACHED_CODE));
    expect(described.reason).toBe("limit");
    expect(described.message).toBe("サーバの文言");
    expect(described.guidance).toContain("古いものを終わらせてから");
  });

  it("理由ごとに別の種別。分類できないものに一手を書かない", () => {
    expect(describeAgreementFailure(failed(AGREEMENT_NOT_ACTIVE_CODE)).reason).toBe("ended");
    expect(describeAgreementFailure(failed(AGREEMENT_NOT_FOUND_CODE)).reason).toBe("not-found");
    expect(describeAgreementFailure(failed(CHARACTER_NOT_FOUND_CODE)).reason).toBe("not-found");
    expect(describeAgreementFailure(failed(AGREEMENT_CONTENT_CONTROL_CODE)).reason).toBe(
      "control",
    );
    expect(describeAgreementFailure(failed("agreement_content_too_long"))).toEqual({
      reason: "unknown",
      message: "サーバの文言",
      guidance: null,
    });
    expect(describeAgreementFailure(new Error("内部"))).toEqual({
      reason: "unknown",
      message: "予期しない失敗です",
      guidance: null,
    });
  });
});

/**
 * **写し元と一致する**（中身の上限・綴り / `src/domain/agreement.rs`、`code` / `src/server/error.rs`）。
 *
 * **型ではなく値**なので、ずれても型検査は通る。画面が嘘の数を出すか、
 * サーバが断る綴りを送る。**植えたソースの対照を対にする**（探せていないことを緑にしない）。
 */
describe("写し元と一致する", () => {
  const AGREEMENT_RS = readFileSync(
    fileURLToPath(new URL("../../src/domain/agreement.rs", import.meta.url)),
    "utf8",
  );
  const ERROR_RS = readFileSync(
    fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
    "utf8",
  );

  it("中身の上限が `AgreementContent::MAX_CHARS` と一致する", () => {
    expect(constOf(AGREEMENT_RS, "AgreementContent", "MAX_CHARS")).toBe(
      AGREEMENT_CONTENT_MAX_CHARS,
    );
  });

  it("綴りが domain の `label` と一致する（並びも）", () => {
    expect(labelsOf(AGREEMENT_RS, "AgreementKind")).toEqual([...AGREEMENT_KINDS]);
    expect(labelsOf(AGREEMENT_RS, "AgreementKeeper")).toEqual([...AGREEMENT_KEEPERS]);
    // 提案中・却下は作業単位 A-12c-1 で増えた（並びは `AgreementState::ALL` —— CHECK と同じ）。
    expect(labelsOf(AGREEMENT_RS, "AgreementState")).toEqual([
      "proposed",
      "active",
      "rejected",
      ...AGREEMENT_ENDINGS,
    ]);
  });

  it("`code` が `error.rs` と一致する", () => {
    expect(codeOf(ERROR_RS, "agreement_limit_reached")).toBe(AGREEMENT_LIMIT_REACHED_CODE);
    expect(codeOf(ERROR_RS, "agreement_not_active")).toBe(AGREEMENT_NOT_ACTIVE_CODE);
    expect(codeOf(ERROR_RS, "agreement_not_found")).toBe(AGREEMENT_NOT_FOUND_CODE);
    expect(codeOf(ERROR_RS, "agreement_not_proposed")).toBe(AGREEMENT_NOT_PROPOSED_CODE);
    expect(codeOf(ERROR_RS, "character_not_found")).toBe(CHARACTER_NOT_FOUND_CODE);
    expect(ERROR_RS).toContain(`"${AGREEMENT_CONTENT_CONTROL_CODE}"`);
  });

  it("対照: 植えたソースからは、その場の値を読む・見つからなければ落ちる", () => {
    const planted = [
      "impl AgreementContent {",
      "    pub const MAX_CHARS: usize = 7;",
      "}",
      "impl AgreementKind {",
      "    pub const fn label(self) -> &'static str {",
      '            Self::Promise => "p",',
      '            Self::Rule => "r",',
      "    }",
      "}",
      "    pub const fn agreement_not_found() -> Self {",
      '            code: Some("planted_code"),',
      "    }",
    ].join("\n");
    expect(constOf(planted, "AgreementContent", "MAX_CHARS")).toBe(7);
    expect(labelsOf(planted, "AgreementKind")).toEqual(["p", "r"]);
    expect(codeOf(planted, "agreement_not_found")).toBe("planted_code");
    expect(() => constOf("", "AgreementContent", "MAX_CHARS")).toThrow();
    expect(() => labelsOf("", "AgreementKind")).toThrow();
    expect(() => codeOf("", "agreement_not_found")).toThrow();
  });
});

/** その型の `impl` ブロックから定数を1つ取り出す（**次の `impl` の手前で切る**）。 */
function constOf(source: string, typeName: string, constName: string): number {
  const block = implBlock(source, typeName);
  const found = new RegExp(`pub const ${constName}: usize = (\\d+);`).exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`impl ${typeName} に ${constName} がありません`);
  }
  return Number.parseInt(found[1], 10);
}

/** その型の `label` の腕の綴りを、書かれた順に取り出す。 */
function labelsOf(source: string, typeName: string): string[] {
  const block = implBlock(source, typeName);
  const start = block.indexOf("fn label(");
  if (start < 0) {
    throw new Error(`impl ${typeName} に label がありません`);
  }
  const body = block.slice(start, block.indexOf("\n    }", start));
  const found = [...body.matchAll(/Self::\w+ => "([^"]+)"/gu)].map((match) => match[1] ?? "");
  if (found.length === 0) {
    throw new Error(`impl ${typeName} の label に腕がありません`);
  }
  return found;
}

function implBlock(source: string, typeName: string): string {
  const start = source.indexOf(`impl ${typeName} {`);
  if (start < 0) {
    throw new Error(`impl ${typeName} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("\nimpl ");
  return end < 0 ? rest : rest.slice(0, end);
}

/** `error.rs` のその構築子から `code` を1つ取り出す（次の `pub const fn` の手前で切る）。 */
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
