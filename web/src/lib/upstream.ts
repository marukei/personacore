/**
 * **上流の観測を、画面の表の行に組む**（作業単位 D-9）。**I/O を持たない。**
 *
 * # なぜ画面から出しているのか
 *
 * **帰属（どの行の、どちらの面の、どの欄か）を1箇所で決めるため。**
 * このリポジトリが繰り返し落としているのは「載っているか」ではなく
 * **「どれのものか」**で、`.svelte` の中に散らすと、
 * **入れ替えの変異を1本のテストで押さえられない。**
 *
 * # `null` を、値と同じ見え方にしない
 *
 * **`answered_by` の `null` は「上流が黙っている」。**
 * 上流が停止しても**予算は現在値を保つ**ので、
 * **ここが `null` になることだけが、外から見える手掛かりになる。**
 * 値があるときと同じ文面にすると、**停止が画面から消える。**
 *
 * **`queried_context_tokens` の `null` も同じ** ——
 * `0` で埋めると「窓が 0 トークンである」と読める。
 *
 * # 知らない綴りは、そのまま出す
 *
 * `statsEdit.ts` の `runModeLabel` / `deltaLabel` と同じ立場。
 * サーバが5つ目の答え方を足した日に、
 * **画面からその欄が消えるのがいちばん質の悪い壊れ方**である。
 */

import type { UpstreamReport, UpstreamView } from "./api";

/**
 * **上流が答えなかったときの文面**（`answered_by` が `null`）。
 *
 * **どの答え方の文面とも別の綴りにすること。**
 * 同じにすると、**停止しているのに答えているように読める。**
 */
export const NO_ANSWER_LABEL = "上流が答えませんでした";

/**
 * **申告された長さが読めなかったときの文面**（`queried_context_tokens` が `null`）。
 *
 * **数を1文字も含めないこと。** 含めると「その長さだった」と読める。
 *
 * **照会した面にだけ使う**（作業単位 B-66 / Issue origin#94 のコメント1）。「読めませんでした」は聞いて失敗したとも
 * 読めるので、**まだ照会していない面は [`NOT_ASKED_LABEL`]**。
 */
export const UNREAD_CONTEXT_LABEL = "読めませんでした";

/**
 * **最後の照会の結果の綴り**（`src/server/upstream.rs` の `CheckOutcome::label` と `NOT_CHECKED` の写し / 作業単位 B-66）。
 *
 * **閉じた型にしていない**（`answered_by` と同じ —— 知らない綴りはそのまま出す）。
 */
export const CHECK_RESULT = {
  NotChecked: "not_checked",
  Answered: "answered",
  NoAnswer: "no_answer",
  NoModelList: "no_model_list",
  Refused: "refused",
  KeyNotSent: "key_not_sent",
} as const;

/**
 * **まだ照会していない面の文面**（作業単位 B-66 / Issue origin#94 のコメント1）。
 *
 * 着手前の画面は、照会していない面にも「読めませんでした」「上流が答えませんでした」「（上流は名乗っていません）」と
 * 言っていた —— どれも「聞いたが駄目だった」と読める。**聞いていないことは、そう言う。**
 *
 * **印・「最後の確認」（`upstreamStatus.ts`）と同じ言い方**（レビュー是正 —— 1周目は「まだ聞いていません」で、
 * 同じ状態を2通りに言っていた）。
 */
export const NOT_ASKED_LABEL = "まだ確かめていません";

/**
 * **届いたが、モデルの一覧が読めなかった面の「上流の答え方」**（`check_result` が `no_model_list` / レビュー是正）。
 *
 * **[`NO_ANSWER_LABEL`]（上流が黙っていた）と別の文面にする** —— 一覧の口を持たない上流は届いていて、会話も通る
 * （モデル一覧は上流に「任意」の能力 / ADR-0038 §決定1）。
 */
export const NO_MODEL_LIST_LABEL = "モデルの一覧を読めませんでした（上流には届いています）";

/**
 * **その面に、まだ照会していないか**（作業単位 B-66）。**答えた読み手があれば、照会したと見る**
 * （古いサーバの応答・ありえない組み合わせで、答えを「聞いていない」と言わない）。
 */
export function notAskedYet(view: UpstreamView): boolean {
  return view.check_result === CHECK_RESULT.NotChecked && view.answered_by === null;
}

/**
 * **上流が名前を名乗らなかったときの文面**（`answered_name` が `null` / 作業単位 B-45）。
 *
 * **モデルの綴りに見える文字列にしないこと。**
 * 「名乗らなかった」と「この名前だった」は、まるで違う事実である。
 */
export const UNNAMED_ANSWER_LABEL = "（上流は名乗っていません）";

/**
 * **一覧は読めたが、どれが答えたかを決められなかったときの文面**
 * （`answered_name` が `null` で、`answered_by` に値がある / レビュー是正 / 2026-09-17）。
 *
 * # なぜ [`UNNAMED_ANSWER_LABEL`] と分けるのか
 *
 * **`answered_name` の `null` は、2つの別の事実を潰している。**
 *
 * | `answered_by` | 起きていること |
 * |---|---|
 * | `null` | **往復そのものが失敗した／一覧が空。** 上流は黙っている |
 * | 値がある | **一覧は読めたが、2件以上あって一致も無い。** 決められないのはこちら |
 *
 * **後者で「上流は名乗っていません」と出すのは偽である**
 * ——上流は現に一覧を出している。**調べる先がまるで違う**
 * （前者は「上流が生きているか」、後者は「依頼する綴りが一覧の綴りと合っているか」）。
 *
 * **モデルの綴りに見える文字列にしないこと**（[`UNNAMED_ANSWER_LABEL`] と同じ理由）。
 */
export const UNDECIDED_ANSWER_LABEL =
  "上流は一覧を出しましたが、どれが答えたかは決められません";

/**
 * **頼んだ名前と答えた名前が食い違っているときの一言**（作業単位 B-45）。
 *
 * # なぜ画面で言うのか
 *
 * **`llama serve` は `model` の欄を無視して、いま載っているモデルで答える**
 * （`C-18` が実測 / 2026-09-17）。**2つの綴りを並べて出すだけでは、
 * 読み手は「別名で同じものを指しているのだろう」と読む。**
 *
 * **⚠ 食い違いは異常ではない**ので、**警告の文面にしないこと。**
 * 言うのは**起きている事実**だけである。
 */
export const ANSWERED_BY_ANOTHER_NOTE = "頼んだ名前と違うものが答えています";

/**
 * 上流の答え方の綴り（`src/llm/response.rs` の `AnsweredBy::label` の写し）。
 *
 * # ⚠ この4つは、**欄名の見張り番では守られない**
 *
 * `tests/api-fields.test.ts` が見ているのは **`serde` の欄名**（`answered_by` という綴り）で、
 * **その欄に入る値の綴りではない。**
 * **利用者が実際に読む文を決めているのは、こちらの写しのほうである。**
 *
 * **実測**（`m19` / 作業単位 D-9 のレビュー / 2026-09-08）:
 * `OPENAI_SOLE_ENTRY` を `"openai-single-entry"` にずらすと **714 全緑・exit 0** だった。
 * ずれると [`answeredByLabel`] はどの腕にも当たらず
 * **「知らない綴りはそのまま出す」枝へ落ち、画面には生の `openai-sole-entry` が出る**
 * ——**下に書いた「推論が1つ入っている」という区別が、そのまま画面から消える。**
 *
 * **→ 見張り番を置いた**（`tests/upstream-labels.test.ts` の
 * 「`answered_by` の綴りは、写し元と一致する」）。
 * **写し元の `.rs` を実際に読み、その綴りが生のまま画面に出ないことを見る。**
 * **一覧をもう1つ写す形にはしていない**（写しが増えると、それが古びたときに誰も見ない）。
 *
 * **是正後に同じ変異を当て直した**（作業者の実測 / 複製ツリー）:
 * **1 赤・exit 1**（「`openai-sole-entry` は、生の綴りのまま画面に出ない」）。
 * **その見張り番だけを外すと 726 全緑・exit 0**（`m19_noguard`）
 * ——**赤にしているのはその1本である。**
 */
const NATIVE_API = "native-api";
/** 同上。OpenAI 互換の一覧に、依頼した綴りがそのまま在った。 */
const OPENAI_EXACT_ID = "openai-exact-id";
/** 同上。OpenAI 互換の一覧が1件だけだったので、それを読んだ。 */
const OPENAI_SOLE_ENTRY = "openai-sole-entry";
/** 同上。綴りの一覧は読めたが、長さの行を同定できなかった。 */
const OPENAI_CATALOGUE_ONLY = "openai-catalogue-only";

/**
 * **どの読み手が答えたかの見出し。** `null` は「上流が黙っている」。
 *
 * # 4つを別の文字列にする
 *
 * **危険の度合いが違う。**
 * `openai-sole-entry` には**推論が1つ入っている**（「答えられるモデルが1つなら、
 * 答えるのはそれ」）ので、`openai-exact-id` と同じ見出しにすると、
 * **推測で読んだ値を、上流が名乗った値として読むことになる。**
 */
export function answeredByLabel(answeredBy: string | null): string {
  if (answeredBy === null) {
    return NO_ANSWER_LABEL;
  }
  if (answeredBy === NATIVE_API) {
    return "上流の独自 API が答えました";
  }
  if (answeredBy === OPENAI_EXACT_ID) {
    return "OpenAI 互換の一覧に、依頼した綴りがそのまま在りました";
  }
  if (answeredBy === OPENAI_SOLE_ENTRY) {
    return "OpenAI 互換の一覧が1件だけだったので、それを読みました";
  }
  if (answeredBy === OPENAI_CATALOGUE_ONLY) {
    return "OpenAI 互換の一覧は読めましたが、長さは読めませんでした";
  }
  return answeredBy;
}

/**
 * **上流が申告したコンテキスト長の見出し。** `null` は「読めなかった」。
 *
 * **⚠ これは「上流が申告した窓」であって「personacore が採用した予算」ではない。**
 * そう読ませないための注記は画面の側（`UpstreamPanel.svelte`）が持つ。
 */
export function contextTokensLabel(tokens: number | null): string {
  if (tokens === null) {
    return UNREAD_CONTEXT_LABEL;
  }
  return `${tokens} トークン`;
}

/**
 * **上流が名乗った名前の見出し**（作業単位 B-45）。
 *
 * # 4つの状態を、4つとも別の見え方にする
 *
 * | `answered_name` | `answered_by` | 出すもの |
 * |---|---|---|
 * | `null` | `null` | [`UNNAMED_ANSWER_LABEL`]（**綴りに見える文字列を出さない**） |
 * | `null` | 値がある | [`UNDECIDED_ANSWER_LABEL`]（**上流は名乗っている。決められないのはこちら**） |
 * | `model` と同じ | どちらでも | **その綴りだけ** |
 * | `model` と違う | どちらでも | **その綴り ＋ [`ANSWERED_BY_ANOTHER_NOTE`]** |
 *
 * # ⚠ `answered_by` を受けるのは、`null` を割るためだけ
 *
 * **名乗った面の見え方は、`answered_by` で1バイトも変わらない。**
 * **答え方そのものは別の行が出す**（[`answeredByLabel`]）ので、ここで混ぜない。
 *
 * # ⚠ 一言は「その値を持つ要素」に書く
 *
 * **面ごとに違う事実**である（本文だけが別物に答えられている状態は普通に起きる）。
 * 表の外に1つ置くと、**どちらの面の話なのかが画面から消える**
 * ——このリポジトリが繰り返し落としているのは「載っているか」ではなく**「どれのものか」**。
 */
export function answeredNameLabel(
  answeredName: string | null,
  model: string,
  answeredBy: string | null,
): string {
  if (answeredName === null) {
    // **上流は答えた**（どの読み手が答えたかは分かっている）。
    // **名指しできないのはこちらの側**なので、「名乗っていません」とは言わない。
    return answeredBy === null
      ? UNNAMED_ANSWER_LABEL
      : UNDECIDED_ANSWER_LABEL;
  }
  if (answeredName === model) {
    return answeredName;
  }
  return `${answeredName}（${ANSWERED_BY_ANOTHER_NOTE}）`;
}

/** 突き合わせの行数の見出し。**単位を付ける**（裸の数だと何の数か読めない）。 */
function rowCountLabel(rows: number): string {
  return `${rows} 行`;
}

/** 表の1行。**見出しと、面ごとの値。** */
export interface UpstreamRow {
  /** 行の見出し（表の左端）。**重複させないこと。** */
  readonly label: string;
  /** 本文生成の上流の値。 */
  readonly main: string;
  /** 裏方（抽出・変容）の上流の値。 */
  readonly sub: string;
}

/**
 * **観測を、表の行に組む。**
 *
 * # ⚠ 面を引くのはここ1箇所
 *
 * `main` の列に入るのは `report.main`、`sub` の列に入るのは `report.sub`。
 * **入れ替えると、いちばん上の「応答が名乗った面」の行にそのまま現れる**
 * ——サーバは**観測を引くのに使った面そのもの**から `face` を作っているので、
 * 鍵と中身の食い違いは応答の中で自分を名乗る。
 *
 * # 内訳を1つに潰さない
 *
 * 突き合わせの5つは**別々の行**にする。足すと、
 * **「別のモデルが載っている」と「一覧に無い」の調査先の違いが画面から消える。**
 */
export function reportRows(report: UpstreamReport): readonly UpstreamRow[] {
  const row = (
    label: string,
    of: (view: UpstreamView) => string,
  ): UpstreamRow => ({
    label,
    main: of(report.main),
    sub: of(report.sub),
  });

  return [
    row("応答が名乗った面", (view) => view.face),
    row("宛先", (view) => view.endpoint),
    row("依頼するモデルの綴り", (view) => view.model),
    // **頼んだ名前の隣に、答えた名前**（作業単位 B-45）。
    // **2つを1つの行に畳まないこと**——畳むと、食い違いが読めなくなる。
    // **まだ照会していない面は、3行とも「まだ確かめていません」**（作業単位 B-66 —— 聞いて失敗したとは言わない）。
    row("上流が答えたモデルの綴り", (view) =>
      notAskedYet(view)
        ? NOT_ASKED_LABEL
        : // **`answered_by` も渡す**——`answered_name` の `null` は2つの事実を潰していて、
          // **どちらなのかはこの欄からしか分からない**（レビュー是正 / 2026-09-17）。
          answeredNameLabel(view.answered_name, view.model, view.answered_by),
    ),
    row("上流が申告したコンテキスト長", (view) =>
      notAskedYet(view) ? NOT_ASKED_LABEL : contextTokensLabel(view.queried_context_tokens),
    ),
    row("上流の答え方", (view) => {
      if (notAskedYet(view)) {
        return NOT_ASKED_LABEL;
      }
      // **届いたが一覧が読めない面は、黙っていたとは言わない**（レビュー是正）。
      if (view.answered_by === null && view.check_result === CHECK_RESULT.NoModelList) {
        return NO_MODEL_LIST_LABEL;
      }
      return answeredByLabel(view.answered_by);
    }),
    row("突き合わせ: 見た行", (view) => rowCountLabel(view.measured.checked)),
    row("突き合わせ: 一致", (view) => rowCountLabel(view.measured.matched)),
    row("突き合わせ: 別のモデルが載っている", (view) =>
      rowCountLabel(view.measured.drifted_other),
    ),
    row("突き合わせ: 依頼する綴りが一覧に無い", (view) =>
      rowCountLabel(view.measured.drifted_absent),
    ),
    row("突き合わせ: 聞けなかった", (view) =>
      rowCountLabel(view.measured.unanswered),
    ),
  ];
}
