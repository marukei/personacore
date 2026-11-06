/**
 * **取り決め（約束・決まり）の画面の純粋な部分**（作業単位 A-12a / ADR-0051 §決定4・5）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `createAgreement` / `endAgreement` /
 * `adoptAgreement` / `rejectAgreement`、画面は `AgreementsPanel.svelte`、入口はキャラクター一覧の行。
 * **提案の問いかけ**（作業単位 A-12c-2）は会話の画面の `ProposalPrompt.svelte`、直す欄は両方の画面で
 * `ProposalEditor.svelte` を使う。
 *
 * # 何をする画面か
 *
 * * **有効な取り決めは、毎ターンのプロンプトに載る**（削れない枠の次の「優先の枠」）。
 *   **入りきらないときは古いものから落ち、それを伝える**（一覧の口の `last_turn` → [`droppedNotice`]）。
 *   終わったものは載らない。
 * * 手で書く（有効で始まる）・手で終わらせる（果たした・破った・取り消した）。**消す口は無い。**
 * * **裏方が会話から提案したもの**（提案中）を、採る・直して採る・捨てる（作業単位 A-12c-2）。
 *   提案中は本文に載らず、上限にも数えない。捨てたもの（却下）は記録の側にだけ出る。
 * * **有効なものが上限に達したら書く欄を止め、古いものを終わらせるよう促す**（黙って古いものを落とさない）。
 *
 * # 写しているもの（**増やさないこと**）
 *
 * | 写している | 写し元 | 見張り番 |
 * |---|---|---|
 * | 中身の上限（60 字） | `src/domain/agreement.rs` の `AgreementContent::MAX_CHARS` | `tests/agreements.test.ts` |
 * | 種類・守る側・終わらせ方の綴り | 同 `label` | 同 |
 * | `code` の綴り（6つ） | `src/server/error.rs` | 同 |
 *
 * **有効な件数の上限は写していない** —— 応答の `active_limit` を読む（`DriftView.recent_limit` と同じ立場）。
 * **履歴の上限も写していない**（応答の `transitions_limit`）。
 */

import { ApiRequestError } from "./api";
import type {
  AdoptAgreementRequest,
  AgreementEnding,
  AgreementKeeper,
  AgreementKind,
  AgreementTransitionView,
  AgreementTransitionsView,
  AgreementView,
  AgreementsView,
  CreateAgreementRequest,
} from "./api";
// **文字数の数え方は1箇所**（符号位置で数える理由はあちらの doc）。
import { countChars } from "./characterEdit";

/** 中身の上限（**`src/domain/agreement.rs` の `AgreementContent::MAX_CHARS` の写し**）。 */
export const AGREEMENT_CONTENT_MAX_CHARS = 60;

/** 種類（**`AgreementKind::label` の写し**）。選択肢の並びでもある。 */
export const AGREEMENT_KINDS: readonly AgreementKind[] = ["promise", "rule"];

/** 守る側（**`AgreementKeeper::label` の写し**）。選択肢の並びでもある（本文の小見出しの順）。 */
export const AGREEMENT_KEEPERS: readonly AgreementKeeper[] = [
  "user",
  "character",
  "both",
];

/** 終わらせ方（**`AgreementState::label` のうち終わった3つ**）。ボタンの並びでもある。 */
export const AGREEMENT_ENDINGS: readonly AgreementEnding[] = [
  "fulfilled",
  "broken",
  "withdrawn",
];

/**
 * 種類の見出し。**知らない綴りはそのまま返す**（サーバが値を足した回に `undefined` を出さない）。
 */
export function agreementKindLabel(kind: string): string {
  switch (kind) {
    case "promise":
      return "約束";
    case "rule":
      return "決まり";
    default:
      return kind;
  }
}

/**
 * 守る側の見出し。**知らない綴りはそのまま返す。**
 *
 * **3つとも別の綴り**（取り違えると、誰が守るのかを画面が嘘で言う）。
 */
export function agreementKeeperLabel(keeper: string): string {
  switch (keeper) {
    case "user":
      return "利用者が守る";
    case "character":
      return "キャラクターが守る";
    case "both":
      return "両方が守る";
    default:
      return keeper;
  }
}

/** 状態の見出し。**知らない綴りはそのまま返す。** 終わらせるボタンの文言もこれを使う。 */
export function agreementStateLabel(state: string): string {
  switch (state) {
    // **提案中・却下は作業単位 A-12c-2 で見出しを持った**（それまでは綴りのまま出ていた）。
    case "proposed":
      return "提案中";
    case "rejected":
      return "却下";
    case "active":
      return "有効";
    case "fulfilled":
      return "果たした";
    case "broken":
      return "破った";
    case "withdrawn":
      return "取り消した";
    default:
      return state;
  }
}

/** 有効な取り決め（**書いた順のまま**）。本文に載るのはこれだけ。 */
export function activeAgreements(view: AgreementsView): AgreementView[] {
  return view.agreements.filter((agreement) => agreement.state === "active");
}

/**
 * **提案中の取り決め**（**書かれた順のまま** / 作業単位 A-12c-2）。本文には載らず、上限にも数えない。
 * 約束・決まりの画面の「提案」の欄と、会話の画面の問いかけに出す。
 */
export function proposedAgreements(view: AgreementsView): AgreementView[] {
  return view.agreements.filter((agreement) => agreement.state === "proposed");
}

/**
 * 終わった取り決め（**書いた順のまま**）。本文には載らない。
 *
 * **提案中・却下は入れない**（作業単位 A-12c-2）—— 提案中は「提案」の欄に、却下は記録の側にだけ出す。
 * かつては「有効でないもの全部」だったので、A-12c-1 で増えた2つが「終わったもの」に綴りのまま出ていた。
 * **知らない綴りはこちらに入れる**（サーバが状態を足した回に、行を黙って画面から消さない）。
 */
export function endedAgreements(view: AgreementsView): AgreementView[] {
  return view.agreements.filter(
    (agreement) =>
      agreement.state !== "active" &&
      agreement.state !== "proposed" &&
      agreement.state !== "rejected",
  );
}

/** **有効なものが上限に達しているか**（上限は応答の `active_limit`）。 */
export function isAtAgreementLimit(view: AgreementsView): boolean {
  return activeAgreements(view).length >= view.active_limit;
}

/**
 * 上限の案内。**上限に達したら、次の一手（古いものを終わらせる）を言う。**
 *
 * **黙って古いものを落とさない**（ADR-0051 §決定4）—— 落とすのはサーバでも画面でもなく、利用者。
 */
export function agreementLimitNotice(view: AgreementsView): string {
  const active = activeAgreements(view).length;
  if (isAtAgreementLimit(view)) {
    return (
      `有効な約束・決まりが上限の ${view.active_limit} 件に達しています。` +
      "新しく書く前に、古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。"
    );
  }
  return `有効な約束・決まりは ${view.active_limit} 件までです（いま ${active} 件）。`;
}

/**
 * **直近のターンで本文に載らなかった取り決めがあるか**の案内（作業単位 A-12a の決め直し）。
 *
 * 取り決めは予算に入りきらないときだけ**古い側から**落ちる。**黙って落とさない**ので、
 * 落ちた回は画面がそれを言う。**落ちていない・まだターンが無いなら `null`**（何も出さない）。
 *
 * **時刻を名乗る** —— その後に書いた・終わらせた取り決めは反映されていない。
 */
export function droppedNotice(view: AgreementsView, zoneLabel: (at: string) => string): string | null {
  const turn = view.last_turn;
  if (turn === null || turn.dropped === 0) {
    return null;
  }
  return (
    `直近のターン（${zoneLabel(turn.at)}）では、有効な約束・決まりのうち古い ${turn.dropped} 件が` +
    `本文に載りませんでした（予算に入りきらなかったため。載ったのは新しい ${turn.included} 件）。` +
    "古いものを終わらせると、残りが載りやすくなります。"
  );
}

/**
 * 書く下書き。**種類と守る側の空文字は「まだ選んでいない」**（既定を作らない）。
 *
 * **守る側に既定を作らないのは意図的**（属性の主語と同じ）。勝手に決めると、
 * 押し間違いで「誰が守るのか」が入れ替わった取り決めが、以後の全ターンに載る。
 */
export interface AgreementDraft {
  content: string;
  kind: string;
  keeper: string;
}

/** 空の下書き。 */
export function emptyAgreementDraft(): AgreementDraft {
  return { content: "", kind: "", keeper: "" };
}

/** 中身が上限を超えているか（**符号位置で数える**）。 */
export function isOverAgreementLimit(content: string): boolean {
  return countChars(content) > AGREEMENT_CONTENT_MAX_CHARS;
}

/** 文字数の欄（`いま / 上限`）。 */
export function agreementCounter(content: string): string {
  return `${countChars(content)} / ${AGREEMENT_CONTENT_MAX_CHARS}`;
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltCreateAgreement =
  | { ok: true; body: CreateAgreementRequest }
  | { ok: false; message: string };

/** 綴りを型で narrow する（`as` で押し込まない）。 */
function knownKind(raw: string): AgreementKind | null {
  return AGREEMENT_KINDS.find((kind) => kind === raw) ?? null;
}

/** 綴りを型で narrow する（`as` で押し込まない）。 */
function knownKeeper(raw: string): AgreementKeeper | null {
  return AGREEMENT_KEEPERS.find((keeper) => keeper === raw) ?? null;
}

/**
 * `POST .../agreements` の本文を組み立てる。**中身は正規化しない**（サーバの `parse` が行う）。
 *
 * 見る順は 中身（空・長さ）→ 種類 → 守る側。空かどうかの判定にだけ `trim()` を使う。
 */
export function buildCreateAgreement(
  draft: AgreementDraft,
): BuiltCreateAgreement {
  if (draft.content.trim() === "") {
    return { ok: false, message: "中身が空です。" };
  }
  if (isOverAgreementLimit(draft.content)) {
    return {
      ok: false,
      message: `中身が長すぎます（上限 ${AGREEMENT_CONTENT_MAX_CHARS} 文字）。`,
    };
  }
  const kind = knownKind(draft.kind);
  if (kind === null) {
    return { ok: false, message: "約束か決まりかを選んでください。" };
  }
  const keeper = knownKeeper(draft.keeper);
  if (keeper === null) {
    return { ok: false, message: "誰が守るのかを選んでください。" };
  }
  return { ok: true, body: { content: draft.content, kind, keeper } };
}

// ----------------------------------------------------------------------
// 提案（作業単位 A-12c-2 / ADR-0051 §決定2）
// ----------------------------------------------------------------------

/**
 * **会話の画面の問いかけの見出し**（ADR-0051 の決め直しの「今こんな約束した？」）。
 *
 * 綴りは配布物の README が「」で名指す（`tests/distribution.rs` が画面の文言とまるごと突き合わせる）。
 */
export const PROPOSAL_QUESTION = "今こんな約束した？";

/** 提案の表の「種類」の欄。**提案の種類は判定していない**（サーバはいつも `rule` で保存する）。 */
export const PROPOSAL_KIND_UNDECIDED = "未判定";

/** 採った後の報せ。**採ったものがいつから効くか**を言う。 */
export const PROPOSAL_ADOPTED_NOTICE = "提案を採りました。次のターンから本文に載ります。";

/** 捨てた後の報せ。**行は消えない**（一覧には出なくなるが、記録には残る）。 */
export const PROPOSAL_REJECTED_NOTICE = "提案を捨てました（記録には残ります）。";

/**
 * **種類の注記**（作業単位 A-12c-2）。**判定していないことを言い、そのまま採ったら何になるか**を言う。
 *
 * そのまま採ったときの種類は**応答の `kind` から言う**（「決まり」と写さない —— サーバが提案の
 * 既定を変えた回に、画面が嘘を言わないため）。
 */
export function proposalKindNote(proposal: AgreementView): string {
  return (
    `種類は${PROPOSAL_KIND_UNDECIDED}です（そのまま採ると「${agreementKindLabel(proposal.kind)}」になります。` +
    "違うなら「直して採る」で選べます）。"
  );
}

/** 問いかけの「何件目か」（`（提案 2 / 3 件）`）。`index` は 0 始まり。 */
export function proposalPosition(index: number, total: number): string {
  return `（提案 ${index + 1} / ${total} 件）`;
}

/**
 * **提案を採れない（上限に達している）ときの案内。達していなければ `null`**（何も出さない）。
 *
 * **送らせてから 409 で断らない**（書く欄と同じ）。終わらせるのは利用者（ADR-0051 §決定4）。
 */
export function proposalLimitNotice(view: AgreementsView): string | null {
  if (!isAtAgreementLimit(view)) {
    return null;
  }
  return (
    `有効な約束・決まりが上限の ${view.active_limit} 件に達しているので、いまは採れません。` +
    "古いものを終わらせてください。"
  );
}

/**
 * **直して採る下書き**。中身と守る側は提案から写し、**種類は空（まだ選んでいない）で始める。**
 *
 * **種類を写さないのは意図的** —— 提案の種類は判定していない（いつも `rule`）。写すと、
 * 利用者が選んでいない「決まり」を、選んだかのように送る。
 */
export function adoptDraftOf(proposal: AgreementView): AgreementDraft {
  return { content: proposal.content, kind: "", keeper: proposal.keeper };
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltAdoptAgreement =
  | { ok: true; body: AdoptAgreementRequest }
  | { ok: false; message: string };

/**
 * 直して採るときの `POST .../adoption` の本文。**3つとも送る**（直した欄だけを送り分けない ——
 * 画面は3つとも見せて選ばせているので、見せたものをそのまま送る）。
 *
 * **検査は書くときと同じ**（[`buildCreateAgreement`] —— サーバも同じ `parse` を通す）。
 * **そのまま採るときは、これを通さずに `{}` を送る。**
 */
export function buildAdoptAgreement(draft: AgreementDraft): BuiltAdoptAgreement {
  const built = buildCreateAgreement(draft);
  if (!built.ok) {
    return built;
  }
  return { ok: true, body: { ...built.body } };
}

/** 履歴の「前」の見出し。**`null` は「書かれた」**（無かった → 後）。 */
export const WRITTEN_LABEL = "（書いた）";

/**
 * 履歴の「前」の見出し。**無かった → 提案中**（裏方が提案として書いた / 作業単位 A-12c-2）。
 * **「書いた」と言わない** —— 利用者が書いたものではない。
 */
export const PROPOSED_LABEL = "（提案された）";

/**
 * 履歴の「前」の欄。**綴りを決めるのはここだけ。**
 *
 * `after` は「前」が無い（`null`）ときだけ見る —— 無かった → 提案中は [`PROPOSED_LABEL`]、それ以外は [`WRITTEN_LABEL`]。
 */
export function transitionBeforeLabel(before: string | null, after?: string): string {
  if (before !== null) {
    return agreementStateLabel(before);
  }
  return after === "proposed" ? PROPOSED_LABEL : WRITTEN_LABEL;
}

/**
 * 記録の「種類」の欄（作業単位 A-12c-2）。**その取り決めが、いま提案中か却下なら「未判定」**
 * —— 提案の種類は判定していない（保存の `rule` は判定ではない）。採った提案は、採った後の種類で出す。
 *
 * いまの状態は一覧（`agreements`）から `agreement_id` で引く。**一覧が読めていない（`null`）・
 * 一覧に無いなら、記録の種類のまま**出す（推測で「未判定」にしない）。
 */
export function transitionKindLabel(
  transition: AgreementTransitionView,
  agreements: readonly AgreementView[] | null,
): string {
  const now = agreements?.find((agreement) => agreement.id === transition.agreement_id);
  if (now !== undefined && (now.state === "proposed" || now.state === "rejected")) {
    return PROPOSAL_KIND_UNDECIDED;
  }
  return agreementKindLabel(transition.kind);
}

/** **総数を必ず言う**（上限を名乗るだけでは「たまたまその数」と「切られた」を区別できない）。 */
export function transitionCountNotice(view: AgreementTransitionsView): string {
  if (view.total_transitions === 0) {
    return "まだ記録はありません。";
  }
  const hidden = Math.max(0, view.total_transitions - view.transitions.length);
  if (hidden > 0) {
    return (
      `全 ${view.total_transitions} 件の記録のうち、新しい ${view.transitions.length} 件だけを出しています` +
      `（1回に返る上限は ${view.transitions_limit} 件 / 残り ${hidden} 件はここに出ていません）。`
    );
  }
  return `全 ${view.total_transitions} 件の記録を、すべて出しています（1回に返る上限は ${view.transitions_limit} 件）。`;
}

/**
 * サーバが返す `code`（**`src/server/error.rs` の綴りの写し**）。
 *
 * ずれるとどの枝にも入らず `unknown` に落ち、案内が黙って消える。
 * **写し元を読む見張り番は `tests/agreements.test.ts`。**
 */
export const AGREEMENT_LIMIT_REACHED_CODE = "agreement_limit_reached";
/** 同上（`ApiError::agreement_not_active`）。 */
export const AGREEMENT_NOT_ACTIVE_CODE = "agreement_not_active";
/** 同上（`ApiError::agreement_not_found`）。 */
export const AGREEMENT_NOT_FOUND_CODE = "agreement_not_found";
/** 同上（`ApiError::agreement_not_proposed` / 作業単位 A-12c-1 で増えた）。 */
export const AGREEMENT_NOT_PROPOSED_CODE = "agreement_not_proposed";
/** 同上（`ApiError::invalid_agreement_content(AgreementContentError::Control)`）。 */
export const AGREEMENT_CONTENT_CONTROL_CODE = "agreement_content_control";
/** 同上（`ApiError::character_not_found`）。 */
export const CHARACTER_NOT_FOUND_CODE = "character_not_found";

/** 失敗の種別。**`unknown` は「分類できなかった」であって「サーバの障害」ではない。** */
export type AgreementFailureReason =
  | "limit"
  | "control"
  | "ended"
  // **提案がもう提案中でない**（先に採った・捨てた / 作業単位 A-12c-2）。
  | "decided"
  | "not-found"
  | "unknown";

/** 画面に出すもの。 */
export interface AgreementFailure {
  reason: AgreementFailureReason;
  /** **サーバが言ったこと。** 読めなかったときだけこちらの定型文になる。 */
  message: string;
  /** **次に取れる行動。** 分からないなら `null`（推測で埋めない）。 */
  guidance: string | null;
}

/** 上限に達していたときの一手。**実在する操作だけを指す。** */
const LIMIT_GUIDANCE =
  "「有効な約束・決まり」の一覧から、古いものを終わらせてから、もう一度書いてください。" +
  "（書こうとした取り決めは保存されていません。）";

/** 見えない文字が混ざったときの一手。 */
const CONTROL_GUIDANCE =
  "中身に、改行や制御文字のような見えない文字が混ざっています。" +
  "貼り付けた文字を消して、手で打ち直してください。";

/** 取り決めがもう終わっていたときの一手。 */
const ENDED_GUIDANCE =
  "別の画面で先に終わらせたのかもしれません。「再読み込み」で取り直してください。";

/** 取り決め（またはキャラクター）が見つからないときの一手。 */
const NOT_FOUND_GUIDANCE = "「再読み込み」で取り直してください。";

/** 失敗を、画面に出せる形へ。**`ApiRequestError` 以外は中身を画面へ流さない。** */
export function describeAgreementFailure(failure: unknown): AgreementFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { reason: "unknown", message: "予期しない失敗です", guidance: null };
  }
  switch (failure.code) {
    case AGREEMENT_LIMIT_REACHED_CODE:
      return { reason: "limit", message: failure.message, guidance: LIMIT_GUIDANCE };
    case AGREEMENT_CONTENT_CONTROL_CODE:
      return {
        reason: "control",
        message: failure.message,
        guidance: CONTROL_GUIDANCE,
      };
    case AGREEMENT_NOT_ACTIVE_CODE:
      return { reason: "ended", message: failure.message, guidance: ENDED_GUIDANCE };
    case AGREEMENT_NOT_FOUND_CODE:
    case CHARACTER_NOT_FOUND_CODE:
      return {
        reason: "not-found",
        message: failure.message,
        guidance: NOT_FOUND_GUIDANCE,
      };
    default:
      return { reason: "unknown", message: failure.message, guidance: null };
  }
}

/**
 * 提案を採ろうとして上限に達していたときの一手（作業単位 A-12c-2）。**実在する操作だけを指す。**
 *
 * **書くときの一手（[`LIMIT_GUIDANCE`]）と分けたのは、言うことが違うから** ——
 * 書くときは「書こうとしたものは保存されていない」、採るときは「提案は残っている」。
 */
const PROPOSAL_LIMIT_GUIDANCE =
  "有効な約束・決まりが上限に達しているので、採れませんでした。" +
  "古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。" +
  "終わらせてから、もう一度採れます（提案はそのまま残っています）。";

/**
 * もう提案中でなかったときの一手。**画面は失敗の後に一覧を取り直す**（だから「取り直しました」と言える）。
 */
const PROPOSAL_DECIDED_GUIDANCE =
  "別の画面で先に採ったか、捨てたのかもしれません。一覧を取り直しました。";

/** 提案（またはキャラクター）が見つからないときの一手。**画面は失敗の後に一覧を取り直す。** */
const PROPOSAL_NOT_FOUND_GUIDANCE = "一覧を取り直しました。";

/**
 * **提案を採る・捨てるときの失敗**を、画面に出せる形へ（作業単位 A-12c-2）。
 *
 * **[`describeAgreementFailure`] と分けた** —— 採る口が返す 409 は `agreement_not_proposed` で、
 * 書く・終わらせる口の `agreement_not_active` は返らない。上限の一手も言うことが違う（上の doc）。
 * **会話の画面には「再読み込み」が無い**ので、一手は「再読み込み」を名指さない。
 */
export function describeProposalFailure(failure: unknown): AgreementFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { reason: "unknown", message: "予期しない失敗です", guidance: null };
  }
  switch (failure.code) {
    case AGREEMENT_LIMIT_REACHED_CODE:
      return {
        reason: "limit",
        message: failure.message,
        guidance: PROPOSAL_LIMIT_GUIDANCE,
      };
    case AGREEMENT_CONTENT_CONTROL_CODE:
      return {
        reason: "control",
        message: failure.message,
        guidance: CONTROL_GUIDANCE,
      };
    case AGREEMENT_NOT_PROPOSED_CODE:
      return {
        reason: "decided",
        message: failure.message,
        guidance: PROPOSAL_DECIDED_GUIDANCE,
      };
    case AGREEMENT_NOT_FOUND_CODE:
    case CHARACTER_NOT_FOUND_CODE:
      return {
        reason: "not-found",
        message: failure.message,
        guidance: PROPOSAL_NOT_FOUND_GUIDANCE,
      };
    default:
      return { reason: "unknown", message: failure.message, guidance: null };
  }
}
