/**
 * **上流の観測を、画面の行に組む純粋関数**（作業単位 D-9 / `src/lib/upstream.ts`）。
 *
 * # ここで見るもの
 *
 * | 見るもの | なぜ |
 * |---|---|
 * | **どの行に、どちらの面の、どの欄が入るか** | このリポジトリが繰り返し落としているのは「載っているか」ではなく**「どれのものか」** |
 * | **`null` が値と同じ見え方にならないこと** | **`answered_by` の `null` は「上流が黙っている」**——値があるときと同じ文面にすると、**停止が画面から消える** |
 * | **内訳を1つに潰さないこと** | `drifted_other` と `drifted_absent` は**調査先がまるで違う** |
 *
 * # ⚠ **フィクスチャは、2つの面で1つも同じ値にしない**
 *
 * 同値だと、**面を取り違える変異が観測できない**（CLAUDE.md の最頻の穴）。
 * **数まで全部ずらしてある**ので、`the_fixture_never_makes_two_faces_equal`
 * に当たる検査をここにも置く。
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { UpstreamReport, UpstreamView } from "../src/lib/api";
import {
  ANSWERED_BY_ANOTHER_NOTE,
  NO_ANSWER_LABEL,
  UNDECIDED_ANSWER_LABEL,
  UNNAMED_ANSWER_LABEL,
  UNREAD_CONTEXT_LABEL,
  answeredByLabel,
  answeredNameLabel,
  contextTokensLabel,
  reportRows,
} from "../src/lib/upstream";

/** 本文の面。**裏方とは1欄も同じ値にしない。** */
const MAIN: UpstreamView = {
  face: "main",
  endpoint: "http://127.0.0.1:1234/v1/chat/completions",
  model: "main-model-for-tests",
  // **頼んだ名前と違う綴りを名乗った上流**（作業単位 B-45 / `C-18` の実測）。
  answered_name: "answering-model-for-tests.gguf",
  queried_context_tokens: 16384,
  answered_by: "native-api",
  unauthorized: false,
  checked_at: "2026-11-03T09:58:00+09:00",
  check_result: "answered",
  measured: {
    checked: 10,
    matched: 7,
    drifted_other: 1,
    drifted_absent: 2,
    unanswered: 0,
  },
  // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
  launch: {
    state: "external",
    since: null,
    answered_name: null,
    reason: null,
    log: null,
  },
};

/** 裏方の面。**上とは1欄も同じ値にしない**（`null` の側も持たせる）。 */
const SUB: UpstreamView = {
  face: "sub",
  endpoint: "http://127.0.0.1:5678/v1/chat/completions",
  model: "sub-model-for-tests",
  // **名乗らなかった面**（`null`）。
  answered_name: null,
  queried_context_tokens: null,
  answered_by: null,
  unauthorized: false,
  // **照会したが答えなかった面**（作業単位 B-66）。「まだ聞いていない」面は下の別の腕。
  checked_at: "2026-11-03T09:57:00+09:00",
  check_result: "no_answer",
  measured: {
    checked: 9,
    matched: 3,
    drifted_other: 4,
    drifted_absent: 0,
    unanswered: 2,
  },
  // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
  launch: {
    state: "external",
    since: null,
    answered_name: null,
    reason: null,
    log: null,
  },
};

const REPORT: UpstreamReport = { main: MAIN, sub: SUB, profile_sample: null, selected_profile: null };

/** その行の `[見出し, 本文の面, 裏方の面]`。 */
function rowsOf(report: UpstreamReport): string[][] {
  return reportRows(report).map((row) => [row.label, row.main, row.sub]);
}

/** 見出しで1行を引く。**無ければ落とす**（探せていないことを緑にしない）。 */
function rowNamed(label: string): string[] {
  const found = rowsOf(REPORT).find((row) => row[0] === label);
  expect(found, `${label} の行があること`).toBeDefined();
  return found as string[];
}

describe("フィクスチャ", () => {
  it("2つの面が、1欄も同じ値になっていない", () => {
    expect(MAIN.face).not.toBe(SUB.face);
    expect(MAIN.endpoint).not.toBe(SUB.endpoint);
    expect(MAIN.model).not.toBe(SUB.model);
    expect(MAIN.queried_context_tokens).not.toBe(SUB.queried_context_tokens);
    expect(MAIN.answered_by).not.toBe(SUB.answered_by);
    expect(MAIN.answered_name).not.toBe(SUB.answered_name);
    // **頼んだ名前と答えた名前も同値にしない**
    // ——同値だと、食い違いの一言が出る枝に一度も入らない（作業単位 B-45）。
    expect(MAIN.answered_name).not.toBe(MAIN.model);
    expect(MAIN.measured).not.toEqual(SUB.measured);
    // **内訳も1つずつ違う**（1欄だけを見る変異が、隣の欄で偶然当たらないように）。
    expect(MAIN.measured.matched).not.toBe(SUB.measured.matched);
    expect(MAIN.measured.drifted_other).not.toBe(SUB.measured.drifted_other);
    expect(MAIN.measured.drifted_absent).not.toBe(SUB.measured.drifted_absent);
    expect(MAIN.measured.unanswered).not.toBe(SUB.measured.unanswered);
  });

  /** **宛先とモデルは、綴りとしても混ざらない**（入れ替えが観測できる形）。 */
  it("宛先とモデルは、どちらの面でも別の綴り", () => {
    expect(MAIN.endpoint).not.toBe(MAIN.model);
    expect(SUB.endpoint).not.toBe(SUB.model);
    expect(MAIN.endpoint).not.toContain(MAIN.model);
    expect(SUB.endpoint).not.toContain(SUB.model);
  });
});

describe("行の組み立て", () => {
  /**
   * **1行ずつ、`[見出し, 本文, 裏方]` の並びで固定する。**
   *
   * **行全体を1つの文字列で見ないこと**（CLAUDE.md / `D-4` の実測 ——
   * **`textContent` への `toContain` は、列の順番を一切見ていない**）。
   */
  it("面・宛先・モデルが、それぞれ正しい行と列に入る", () => {
    expect(rowsOf(REPORT)).toEqual([
      ["応答が名乗った面", "main", "sub"],
      [
        "宛先",
        "http://127.0.0.1:1234/v1/chat/completions",
        "http://127.0.0.1:5678/v1/chat/completions",
      ],
      ["依頼するモデルの綴り", "main-model-for-tests", "sub-model-for-tests"],
      // **文面は直書きする**（定数から作ると、緩めたときに期待値も一緒に動く）。
      [
        "上流が答えたモデルの綴り",
        "answering-model-for-tests.gguf（頼んだ名前と違うものが答えています）",
        "（上流は名乗っていません）",
      ],
      ["上流が申告したコンテキスト長", "16384 トークン", UNREAD_CONTEXT_LABEL],
      // **文面は直書きする。** `answeredByLabel` から作ると、
      // **緩めたときに期待値も一緒に動く**（CLAUDE.md）。
      ["上流の答え方", "上流の独自 API が答えました", NO_ANSWER_LABEL],
      ["突き合わせ: 見た行", "10 行", "9 行"],
      ["突き合わせ: 一致", "7 行", "3 行"],
      ["突き合わせ: 別のモデルが載っている", "1 行", "4 行"],
      ["突き合わせ: 依頼する綴りが一覧に無い", "2 行", "0 行"],
      ["突き合わせ: 聞けなかった", "0 行", "2 行"],
    ]);
  });

  /**
   * **面を入れ替えたら、全部の行が入れ替わる。**
   *
   * `main` の列に `sub` の観測を入れる変異は、**応答が名乗った面**の行で
   * まず現れる（ADR の決定どおり、応答は自分の面を名乗る）。
   */
  it("面を入れ替えると、行の中身も入れ替わる", () => {
    const swapped = rowsOf({ main: SUB, sub: MAIN, profile_sample: null, selected_profile: null });
    expect(swapped).not.toEqual(rowsOf(REPORT));
    expect(swapped[0]).toEqual(["応答が名乗った面", "sub", "main"]);
  });

  /**
   * **まだ照会していない面は、「読めませんでした」「答えませんでした」と言わない**（作業単位 B-66 / Issue origin#94 のコメント1）。
   *
   * 「読めませんでした」は聞いて失敗したとも読める。**照会していない面（`check_result: "not_checked"`）では
   * 「まだ確かめていません」と言い、照会したが答えなかった面（上の `SUB`）とは別の文面にする。**
   * 裏方の面だけを「まだ確かめていない」にして、本文の面（答えた）の行が動かないことも見る。
   *
   * > **レビュー是正**: 1周目は表の3行とカードの名前・長さを「まだ聞いていません」、印と「最後の確認」を
   * > 「まだ確かめていません」と、同じ状態を2通りに言っていた。**「まだ確かめていません」に揃えた。**
   */
  /**
   * **届いたが一覧が読めない面の「上流の答え方」は、黙っていたとは言わない**（レビュー是正）。
   * 「上流が答えませんでした」は聞いたのに黙っていた面のこと（表の注記がそう言う）。一覧の口が無い上流は届いている。
   */
  it("一覧が読めない面の答え方は「上流が答えませんでした」と言わない", () => {
    const listless: UpstreamView = { ...SUB, check_result: "no_model_list" };
    const row = rowsOf({ ...REPORT, sub: listless }).find((found) => found[0] === "上流の答え方");
    expect(row).toEqual(["上流の答え方", "上流の独自 API が答えました", "モデルの一覧を読めませんでした（上流には届いています）"]);
  });

  it("まだ照会していない面は、答えた名前・長さ・答え方の3行で「まだ確かめていません」と言う", () => {
    const unasked: UpstreamView = { ...SUB, checked_at: null, check_result: "not_checked" };
    const rows = rowsOf({ ...REPORT, sub: unasked });
    const pick = (label: string) => rows.find((row) => row[0] === label);
    expect([
      pick("上流が答えたモデルの綴り"),
      pick("上流が申告したコンテキスト長"),
      pick("上流の答え方"),
    ]).toEqual([
      [
        "上流が答えたモデルの綴り",
        "answering-model-for-tests.gguf（頼んだ名前と違うものが答えています）",
        "まだ確かめていません",
      ],
      ["上流が申告したコンテキスト長", "16384 トークン", "まだ確かめていません"],
      ["上流の答え方", "上流の独自 API が答えました", "まだ確かめていません"],
    ]);
    // **照会したが答えなかった面は、今までの文面のまま**（2つを同じ文面に潰さない）。
    expect([rowNamed("上流が申告したコンテキスト長")[2], rowNamed("上流の答え方")[2]]).toEqual([
      "読めませんでした",
      "上流が答えませんでした",
    ]);
  });

  /** **見出しは重複しない**（同じ行が2つあると、引いた行がどちらか決まらない）。 */
  it("見出しが重複していない", () => {
    const labels = reportRows(REPORT).map((row) => row.label);
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("`null` は、値と同じ見え方にしない", () => {
  /**
   * **綴りそのものを1度だけ固定する。**
   *
   * 以下の検査は定数を使って突き合わせるので、
   * **定数の中身が空文字列になっても、そこだけでは赤くならない。**
   */
  it("`null` のときの文面が、実際に何かを言っている", () => {
    expect(NO_ANSWER_LABEL).toBe("上流が答えませんでした");
    expect(UNREAD_CONTEXT_LABEL).toBe("読めませんでした");
  });

  /**
   * **`answered_by` の `null` は「上流が黙っている」。**
   *
   * 上流が停止しても**予算は現在値を保つ**ので、
   * **ここが `null` になることだけが、外から見える手掛かりになる。**
   * 値があるときと同じ文面にすると、**停止が画面から消える。**
   */
  it("答えなしは、どの答え方とも違う文面", () => {
    expect(answeredByLabel(null)).toBe(NO_ANSWER_LABEL);
    for (const spelling of [
      "native-api",
      "openai-exact-id",
      "openai-sole-entry",
      "openai-catalogue-only",
    ]) {
      expect(answeredByLabel(spelling), spelling).not.toBe(NO_ANSWER_LABEL);
    }
  });

  it("4つの答え方は、互いに違う文面", () => {
    const labels = [
      "native-api",
      "openai-exact-id",
      "openai-sole-entry",
      "openai-catalogue-only",
    ].map((spelling) => answeredByLabel(spelling));
    expect(new Set(labels).size).toBe(labels.length);
  });

  /**
   * **知らない綴りは、そのまま出す**（`statsEdit.ts` の `runModeLabel` と同じ立場）。
   *
   * サーバが5つ目の答え方を足した日に、
   * **画面からその欄が消えるのがいちばん質の悪い壊れ方**である。
   */
  it("知らない綴りは、そのまま出す", () => {
    expect(answeredByLabel("some-new-way")).toBe("some-new-way");
    expect(answeredByLabel("some-new-way")).not.toBe(NO_ANSWER_LABEL);
  });

  /**
   * **名乗らなかったことを、綴りに見える文字列で埋めない**（作業単位 B-45）。
   *
   * 「名乗らなかった」と「この名前だった」は、まるで違う事実である。
   */
  it("答えた名前の `null` は、綴りと同じ見え方にしない", () => {
    expect(UNNAMED_ANSWER_LABEL).toBe("（上流は名乗っていません）");
    expect(answeredNameLabel(null, "any-model", null)).toBe(
      UNNAMED_ANSWER_LABEL,
    );
    expect(answeredNameLabel("", "any-model", null)).not.toBe(
      UNNAMED_ANSWER_LABEL,
    );
    expect(answeredNameLabel("any-model", "any-model", null)).not.toBe(
      UNNAMED_ANSWER_LABEL,
    );
  });

  /**
   * **`null` は2つの事実を潰している**（レビュー是正 / 2026-09-17）。
   *
   * | `answered_name` | `answered_by` | 起きていること |
   * |---|---|---|
   * | `null` | `null` | **往復そのものが失敗した／一覧が空。** 上流は黙っている |
   * | `null` | 値がある | **一覧は読めたが、2件以上あって一致も無い。** どれが答えたか決められない |
   *
   * **後者で「上流は名乗っていません」と出すのは偽である**
   * ——上流は現に一覧を出している。**分からないのはこちらの側。**
   */
  it("一覧は読めたが名指しできない面は、黙っている面と別の文面", () => {
    expect(UNDECIDED_ANSWER_LABEL).toBe(
      "上流は一覧を出しましたが、どれが答えたかは決められません",
    );
    expect(UNDECIDED_ANSWER_LABEL).not.toBe(UNNAMED_ANSWER_LABEL);

    // **同じ `answered_name === null` でも、`answered_by` で分かれる。**
    expect(
      answeredNameLabel(null, "any-model", "openai-catalogue-only"),
    ).toBe(UNDECIDED_ANSWER_LABEL);
    expect(answeredNameLabel(null, "any-model", null)).toBe(
      UNNAMED_ANSWER_LABEL,
    );

    // **名乗った面は、どちらの文面にもならない。**
    for (const answeredBy of [null, "native-api", "openai-catalogue-only"]) {
      const named = answeredNameLabel("some-name.gguf", "any-model", answeredBy);
      expect(named, String(answeredBy)).not.toBe(UNDECIDED_ANSWER_LABEL);
      expect(named, String(answeredBy)).not.toBe(UNNAMED_ANSWER_LABEL);
      expect(named, String(answeredBy)).toContain("some-name.gguf");
    }

    // **綴りに見える文字列にしない**（「名指しできない」と「この名前だった」は別）。
    expect(UNDECIDED_ANSWER_LABEL).not.toContain("any-model");
  });

  it("コンテキスト長の `null` は、数と同じ見え方にしない", () => {
    expect(contextTokensLabel(null)).toBe(UNREAD_CONTEXT_LABEL);
    expect(contextTokensLabel(0)).not.toBe(UNREAD_CONTEXT_LABEL);
    expect(contextTokensLabel(0)).toContain("0");
    expect(contextTokensLabel(16384)).toContain("16384");
  });

  /**
   * **食い違っているときだけ、そう言う**（作業単位 B-45）。
   *
   * **常に言う実装も、一度も言わない実装も赤にする**ため、
   * **同じ腕と違う腕を両方回す。**
   *
   * **⚠ `C-18` が実測した壊れ方**——`llama serve` は `model` の欄を無視して
   * いま載っているモデルで答えるので、**綴りを並べるだけでは
   * 「別名で同じものを指しているのだろう」と読まれる。**
   */
  it("頼んだ名前と違うときだけ、そのことを言う", () => {
    expect(ANSWERED_BY_ANOTHER_NOTE).toBe("頼んだ名前と違うものが答えています");

    const same = answeredNameLabel("model-a", "model-a", "native-api");
    expect(same).toBe("model-a");
    expect(same).not.toContain(ANSWERED_BY_ANOTHER_NOTE);

    const different = answeredNameLabel("model-b.gguf", "model-a", "native-api");
    expect(different).toContain("model-b.gguf");
    expect(different).toContain(ANSWERED_BY_ANOTHER_NOTE);
    // **答えた名前を、頼んだ名前に置き換えていないこと。**
    expect(different).not.toContain("model-a");

    // **名乗らなかった面では言わない**（食い違っているとは言えない）。
    for (const answeredBy of [null, "openai-catalogue-only"]) {
      expect(
        answeredNameLabel(null, "model-a", answeredBy),
        String(answeredBy),
      ).not.toContain(ANSWERED_BY_ANOTHER_NOTE);
    }
  });

  it("行に組んだときも、`null` の側は数を名乗らない", () => {
    const context = rowNamed("上流が申告したコンテキスト長");
    expect(context[2]).toBe(UNREAD_CONTEXT_LABEL);
    expect(context[2]).not.toMatch(/\d/u);

    const answered = rowNamed("上流の答え方");
    expect(answered[2]).toBe(NO_ANSWER_LABEL);
    expect(answered[2]).not.toBe(answered[1]);

    // **答えた名前も同じ**——名乗らなかった側が、名乗った側と同じ見え方にならない。
    const name = rowNamed("上流が答えたモデルの綴り");
    expect(name[2]).toBe(UNNAMED_ANSWER_LABEL);
    expect(name[2]).not.toBe(name[1]);
    // **頼んだ名前で埋めないこと**（埋めると、食い違いが画面から消える）。
    expect(name[2]).not.toBe(SUB.model);
  });
});

/**
 * **`answered_by` の4つの綴りは、`src/llm/response.rs` の写しである**
 * （作業単位 D-9 のレビュー是正 / 2026-09-08）。
 *
 * # なぜ見張るのか —— **実測で、無防備だった**
 *
 * **`m19`（`OPENAI_SOLE_ENTRY` を `"openai-single-entry"` にずらす）が 714 全緑・exit 0。**
 * ずれると `answeredByLabel` はどの腕にも当たらず
 * **「知らない綴りはそのまま出す」枝へ落ちる**ので、
 * **画面には生の `openai-sole-entry` が出る。**
 *
 * **⚠ 消えるのは `upstream.ts` が自分で書いている説明のほうである** ——
 * 「`openai-sole-entry` には**推論が1つ入っている**ので、
 * `openai-exact-id` と同じ見出しにすると、
 * **推測で読んだ値を、上流が名乗った値として読むことになる**」。
 * **綴りがずれた瞬間、その区別が画面から消える。**
 *
 * # ⚠ 欄名の見張り番では届かない
 *
 * `api-fields.test.ts` が見ているのは **`serde` の欄名**（`answered_by` という綴り）で、
 * **その欄に入る値の綴りではない。**
 * **利用者が実際に読む文を決めているのは、こちらの写しのほうである。**
 *
 * # 何を突き合わせるか
 *
 * **`AnsweredBy::label` の `match` の腕が返す綴りを実際に読み、
 * その1つずつが `answeredByLabel` で「生の綴りのまま」出ないことを見る。**
 *
 * **一覧を写して並べる形にはしていない。**
 * 写しをもう1つ作ると、**その写しがずれたときに誰も見ない**
 * （CLAUDE.md「同じ発見を2箇所に書かない」）。
 * **この形なら、サーバが5つ目の腕を足した日にも赤くなる**
 * ——「写すか、写さないと決めるか」を必ず1回通る。
 *
 * **実測**（`m19` を是正後のツリーへ当て直した / 複製ツリー）:
 * **1 赤・exit 1**（下の `it.each` の `openai-sole-entry` の腕）。
 * **この節を外すと 726 全緑・exit 0**（`m19_noguard`）
 * ——**赤にしているのはこの1本である。**
 *
 * **⚠ この節は写し元（`src/llm/response.rs`）をリポジトリ相対で読む。**
 * **変異検査の複製は、`web/` だけでなくリポジトリ根の形で取ること**
 * （CLAUDE.md / `B-14` —— `web/` だけ複製すると、写し元を読む見張り番が落ちて
 * 「変異が効いた」に見える）。
 */

/**
 * `impl <型> { fn <名前> … }` の中の `Self::X => "綴り",` を、**現れる順に**全部。
 *
 * **読めなければ throw する**（探せていないことを緑にしない / CLAUDE.md `15b`）。
 *
 * **⚠ 日本語の doc コメントで壊れないこと**（CLAUDE.md `B-21d`）——
 * 判定は**行を trim した先頭からの完全一致**で、`///` や `//` の行は数えない。
 * **⚠ 改行は `\r` を落としてから見る**（写し元は CRLF / CLAUDE.md `E-1`）。
 */
export function labelArmsOf(
  source: string,
  implName: string,
  fnName: string,
): string[] {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));

  const implAt = lines.flatMap((line, index) =>
    line === `impl ${implName} {` ? [index] : [],
  );
  if (implAt.length !== 1 || implAt[0] === undefined) {
    throw new Error(`impl ${implName} の宣言が ${implAt.length} 個あります`);
  }
  const implEnd = lines.findIndex(
    (line, index) => index > (implAt[0] as number) && line === "}",
  );
  if (implEnd < 0) {
    throw new Error(`impl ${implName} の終わりが見つかりません`);
  }
  const body = lines.slice(implAt[0] + 1, implEnd);

  // **正規表現を組み立てない**（名前がメタ文字を含むと、黙って別のものを探す）。
  // `fn <名前>(` が、その行の**語の切れ目**で始まっていること。
  const fnHead = `fn ${fnName}(`;
  const fnAt = body.flatMap((line, index) => {
    const at = line.indexOf(fnHead);
    if (at < 0) {
      return [];
    }
    const before = line.slice(0, at);
    return before === "" || before.endsWith(" ") ? [index] : [];
  });
  if (fnAt.length !== 1 || fnAt[0] === undefined) {
    throw new Error(`fn ${fnName} の宣言が ${fnAt.length} 個あります`);
  }
  const indent = /^\s*/u.exec(body[fnAt[0]] as string)?.[0] ?? "";
  const fnEnd = body.findIndex(
    (line, index) => index > (fnAt[0] as number) && line === `${indent}}`,
  );
  if (fnEnd < 0) {
    throw new Error(`fn ${fnName} の終わりが見つかりません`);
  }

  const arms = body
    .slice(fnAt[0] + 1, fnEnd)
    .map((line) => line.trim())
    .filter((line) => !line.startsWith("//"))
    .flatMap((line) => {
      const found = /^Self::[A-Za-z0-9_]+ => "([^"]*)",$/u.exec(line);
      return found?.[1] === undefined ? [] : [found[1]];
    });
  if (arms.length === 0) {
    throw new Error(`fn ${fnName} に綴りを返す腕が1つもありません`);
  }
  return arms;
}

const RESPONSE_RS = readFileSync(
  fileURLToPath(new URL("../../src/llm/response.rs", import.meta.url)),
  "utf8",
);

/** 写し元が実際に返す綴り。**`upstream.ts` はこれを1つ残らず知っていなければならない。** */
const RUST_SPELLINGS = labelArmsOf(RESPONSE_RS, "AnsweredBy", "label");

describe("`answered_by` の綴りは、写し元と一致する", () => {
  /**
   * **写し元の腕が4つある。**
   *
   * 減ったときも増えたときも、ここで1回止まる
   * （腕が増えれば下の `it.each` も赤くなるが、**数が変わったこと自体を名指しする**）。
   */
  it("写し元の `match` に、綴りを返す腕が4つある", () => {
    expect(RUST_SPELLINGS).toHaveLength(4);
    expect(new Set(RUST_SPELLINGS).size).toBe(RUST_SPELLINGS.length);
  });

  /**
   * **これが `m19` を捕まえる本命。**
   *
   * 写しがずれると `answeredByLabel` は**その綴りをそのまま返す**
   * （「知らない綴りは、そのまま出す」枝）。
   * **つまり「生の綴りが画面に出る」ことが、ずれの唯一の見え方である。**
   */
  it.each(RUST_SPELLINGS)("`%s` は、生の綴りのまま画面に出ない", (spelling) => {
    expect(answeredByLabel(spelling)).not.toBe(spelling);
  });

  /** **写し元の綴りは、どれも「答えなし」の文面にならない。** */
  it.each(RUST_SPELLINGS)("`%s` は、答えなしの文面にならない", (spelling) => {
    expect(answeredByLabel(spelling)).not.toBe(NO_ANSWER_LABEL);
  });

  /**
   * **写し元の4つが、互いに違う見出しになる。**
   *
   * `openai-sole-entry` には**推論が1つ入っている**ので、
   * `openai-exact-id` と同じ見出しにすると、
   * **推測で読んだ値を、上流が名乗った値として読むことになる。**
   */
  it("写し元の4つが、互いに違う見出しになる", () => {
    const labels = RUST_SPELLINGS.map((spelling) => answeredByLabel(spelling));
    expect(new Set(labels).size).toBe(labels.length);
  });

  /**
   * **⚠ 「互いに違う」では足りない。文面そのものを固定する。**
   *
   * **実測（`m22`）: `openai-exact-id` と `openai-sole-entry` の返す文を入れ替えると
   * 736 全緑・exit 0 だった。** 綴りはずれていないので写しの検査に掛からず、
   * 4つは依然として互いに違うので上の検査にも掛からない。
   * **`upstream.ts` の doc が名指ししている実害そのもの**——
   * **推測で読んだ値（一覧が1件だったから、それを読んだ）を、
   * 上流が名乗った値（依頼した綴りがそのまま在った）として読むことになる。**
   *
   * **表の突き合わせ（`upstream-panel.test.ts`）でも止まらない** ——
   * あちらのフィクスチャは `native-api` と `null` しか通らないので、
   * **OpenAI 側の3つの文面は、そこに一度も現れない。**
   *
   * **この1本を足して `m22` は 1 赤・exit 1**、
   * **この1本だけを外すと 736 全緑・exit 0**（`m22_noguard`）
   * ——**赤にしているのはこの1本である。**
   */
  it("4つの綴りが、それぞれ決まった文面になる", () => {
    expect(answeredByLabel("native-api")).toBe("上流の独自 API が答えました");
    expect(answeredByLabel("openai-exact-id")).toBe(
      "OpenAI 互換の一覧に、依頼した綴りがそのまま在りました",
    );
    expect(answeredByLabel("openai-sole-entry")).toBe(
      "OpenAI 互換の一覧が1件だけだったので、それを読みました",
    );
    expect(answeredByLabel("openai-catalogue-only")).toBe(
      "OpenAI 互換の一覧は読めましたが、長さは読めませんでした",
    );
  });
});

/**
 * **走査本体の対照**（CLAUDE.md `15b` —— **実ファイルだけを読む対照は自己成就しうる**）。
 *
 * **植えたソースを同じ関数に通す。** 期待値そのものを固定する
 * （`contains` で「行き過ぎ」だけを見ない / CLAUDE.md `E-1`）。
 */
describe("対照: 走査本体は、植えたソースからその場の腕を読む", () => {
  const planted = [
    "impl AnsweredBy {",
    "    /// 綴りは `\"decoy-in-doc\"` である（doc の中の囮）。",
    "    pub const fn label(self) -> &'static str {",
    "        match self {",
    '            // Self::Decoy => "decoy-in-comment",',
    '            Self::Planted => "planted-arm",',
    '            Self::Other => "other-arm",',
    "        }",
    "    }",
    "",
    "    /// 別の関数。**ここの腕は読まない。**",
    "    pub const fn other(self) -> &'static str {",
    "        match self {",
    '            Self::Planted => "must-not-be-read",',
    "        }",
    "    }",
    "}",
  ].join("\n");

  it("植えたソースの腕を、現れる順にそのまま読む", () => {
    expect(labelArmsOf(planted, "AnsweredBy", "label")).toEqual([
      "planted-arm",
      "other-arm",
    ]);
  });

  /** **CRLF でも同じ答え**（写し元は CRLF / CLAUDE.md `E-1`）。 */
  it("CRLF の入力でも、同じ答えを返す", () => {
    expect(
      labelArmsOf(planted.replace(/\n/gu, "\r\n"), "AnsweredBy", "label"),
    ).toEqual(["planted-arm", "other-arm"]);
  });

  it("別の関数を指せば、そちらの腕を読む", () => {
    expect(labelArmsOf(planted, "AnsweredBy", "other")).toEqual([
      "must-not-be-read",
    ]);
  });

  it("腕が1つも無ければ落とす（探せていないことを緑にしない）", () => {
    const empty = [
      "impl AnsweredBy {",
      "    pub const fn label(self) -> &'static str {",
      "        match self {",
      "        }",
      "    }",
      "}",
    ].join("\n");
    expect(() => labelArmsOf(empty, "AnsweredBy", "label")).toThrow();
  });

  it("`impl` が無ければ落とす", () => {
    expect(() => labelArmsOf("// 何も無い\n", "AnsweredBy", "label")).toThrow();
  });

  it("`fn` が無ければ落とす", () => {
    expect(() => labelArmsOf(planted, "AnsweredBy", "missing")).toThrow();
  });

  /** **対照: 写し元を本当に読んでいる**（植えた綴りは実ファイルに無い）。 */
  it("写し元からは、植えた綴りが読めない", () => {
    expect(RUST_SPELLINGS).not.toContain("planted-arm");
    expect(RUST_SPELLINGS).toContain("openai-sole-entry");
  });
});
