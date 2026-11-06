/**
 * **キャラクター属性の置き換えを組み立てる**（作業単位 D-12a / ADR-0009 / ADR-0028）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `replaceAttributes`、
 * 画面は `AttributesPanel.svelte`、入口はキャラクター一覧の行。
 *
 * # ⚠ **これは4つ目のモードである**
 *
 * | モード | 送るもの | どこ |
 * |---|---|---|
 * | 作成 | 空の欄はキーごと落とす | `characterEdit.ts` の `buildCreate` |
 * | 編集 | **差分だけ**（キーの有無が「変えない」） | 同 `buildUpdate` |
 * | 修復 | **全部の欄**（元の値が読めないので差分を取れない） | `characterRepair.ts` |
 * | 置き換え（性格値） | **全軸**（送らなかった軸は消える） | `statsEdit.ts` |
 * | **置き換え（属性）** | **全件**（**送らなかった属性は消える**） | **ここ** |
 *
 * **`PUT .../attributes` は丸ごと置き換え**（`admin.rs` の `replace_attributes`）。
 * **「変えない」を表す手段が無い**ので、**差分モードの作法を持ち込むと
 * 送らなかった属性が黙って消える。**
 *
 * # **属性を消す方法は「送らないこと」しかない**
 *
 * 削除の API は無い（`PUT` ただ1本）。したがって
 * **「外す」＝「本文に載せない」**であり、**それが破壊的な操作そのもの**である。
 * **画面はそれを押す前に言う**（[`replaceSummary`]）。
 *
 * # 要約の数え方は、サーバが履歴に書く差分と同じ
 *
 * 写し元は **`src/domain/attribute.rs` の `attribute_changes`** ——
 * **`(subject, key)` の組で突き合わせ、値まで同じなら記録しない。**
 * **キーだけで突き合わせないこと。** `年齢` を利用者とキャラクターの両方が持つとき、
 * **持ち主をまたいだ差分**が出る（ADR-0021 が 6% 実測した主語の取り違えを、
 * 画面の側で作り込む形になる）。
 *
 * # 写しているもの（**増やさないこと**）
 *
 * | 写している | 写し元 | 見張り番 |
 * |---|---|---|
 * | 上限（キー 12 / 値 20 / 件数 8） | **`src/domain/attribute.rs`** | `tests/attributes-edit.test.ts` |
 * | `code` の綴り（4つ） | `src/server/error.rs` | 同 |
 * | 主語の綴りと見出し | **写していない** ——`memoryRepair.ts` の `SUBJECTS` /
 *   `subjectLabel` をそのまま使う（[`attributeSubjectLabel`]） | —— |
 *
 * **履歴の上限（`ATTRIBUTE_REVISIONS_LIMIT`）は写していない** ——
 * 応答の `revisions_limit` を出す（`DriftView.recent_limit` と同じ立場）。
 */

import { ApiRequestError } from "./api";
import type {
  AttributeRevisionsView,
  AttributeSubject,
  AttributeView,
  AttributesView,
  ReplaceAttributesRequest,
} from "./api";
// **文字数の数え方は1箇所**（符号位置で数える理由はあちらの doc）。
import { countChars } from "./characterEdit";
// **主語の綴りと見出しは1箇所**（写しを2つ持たない）。
import { SUBJECTS, subjectLabel } from "./memoryRepair";

/**
 * 上限（**`src/domain/attribute.rs` の `MAX_CHARS` の写し**）。
 *
 * キーは**送る本文のキーと同じ綴り**にしてある（欄と上限の対応を1箇所に保つため）。
 *
 * **⚠ 写し元は `src/domain/character.rs` ではない。**
 * あちら（`characterEdit.ts` の `LIMITS` の写し元）に属性の上限は1つも無い。
 */
export const ATTRIBUTE_LIMITS = {
  /** `AttributeKey::MAX_CHARS`。 */
  key: 12,
  /** `AttributeValue::MAX_CHARS`。 */
  value: 20,
} as const;

/** 上限を持つ欄。 */
export type LimitedAttributeField = keyof typeof ATTRIBUTE_LIMITS;

/**
 * 1キャラクターが持てる属性の件数（**`AttributeSet::MAX_LEN` の写し**）。
 *
 * **画面でも数える理由は上限の欄と同じ** ——**送る前に止める。**
 * 越えた要求は `too_many_attributes` で必ず落ちるので、
 * 押させると1件も保存されないまま置き換えが取り消される。
 */
export const MAX_ATTRIBUTES = 8;

/** 欄の見出し。**画面とメッセージで同じ綴りを使う**（写しを増やさない）。 */
export function attributeFieldLabel(field: LimitedAttributeField): string {
  switch (field) {
    case "key":
      return "キー";
    case "value":
      return "値";
  }
}

/**
 * 主語の見出し。**綴りの表は `memoryRepair.ts` のものを使う。**
 *
 * # ⚠ 知らない綴りは、そのまま返す
 *
 * `subjectLabel` は2値の型を取るので、**サーバが3つ目の綴りを返した回に
 * `undefined` が画面へ出る。** 属性は**削れない枠に毎ターン載る**ので、
 * **画面から主語の欄が消えるのがいちばん質の悪い壊れ方**である
 * （`statsEdit.ts` の `runModeLabel` / `deltaLabel` と同じ立場）。
 */
export function attributeSubjectLabel(subject: string): string {
  const known = knownAttributeSubject(subject);
  return known === null ? subject : subjectLabel(known);
}

/**
 * その綴りが、線に乗せられる主語か。**知らない綴りは `null`。**
 *
 * **`as` で押し込まないこと。** 押し込むと、
 * **サーバが3つ目の綴りを返した回に、その綴りをそのまま送り返す**
 * ——サーバは `attribute_subject_unknown`（400）で断るので、
 * **置き換えが「なぜか通らない」状態**になる。
 */
export function knownAttributeSubject(
  subject: string,
): AttributeSubject | null {
  return SUBJECTS.find((candidate) => candidate === subject) ?? null;
}

/**
 * 置き換えの下書きの1行。
 *
 * **`subject` は `string`。空文字は「まだ選んでいない」** ——
 * **既定を作らないのは意図的**（`memoryRepair.ts` の `CorrectionDraft.subject` と同じ）。
 * 勝手に決めると、**押し間違いで持ち主の入れ替わった属性ができる**うえ、
 * 属性は毎ターン必ず読まれるので**そのキャラクターの以後の全ターンがそれを前提に喋る。**
 */
export interface AttributeRow {
  /** 誰についての属性か。**空文字は未選択で、送れない。** */
  subject: string;
  /** キー。**空白だけなら送れない。** */
  key: string;
  /** 値。**空白だけなら送れない**（サーバも `AttributeValue::parse` で拒む）。 */
  value: string;
  /** **本文に載せない＝この属性は消える。** */
  removed: boolean;
  /** **サーバから来た行か。** この画面で足した行は `false`。 */
  stored: boolean;
}

/** 置き換えの下書き。 */
export interface AttributesDraft {
  rows: AttributeRow[];
}

/** いまの値を下書きに写す。**1件も落とさない。** */
export function draftOfAttributes(view: AttributesView): AttributesDraft {
  return {
    rows: view.attributes.map((attribute) => ({
      subject: attribute.subject,
      key: attribute.key,
      value: attribute.value,
      removed: false,
      stored: true,
    })),
  };
}

/**
 * 足す行の初期値。**何も埋めない**（主語も選ばない / [`AttributeRow`] の doc）。
 *
 * **`stored` が `false`** —— この行を外しても「属性が消える」ではない
 * （元から保存されていない）。[`replacePlan`] がその区別を使う。
 */
export function emptyAttributeRow(): AttributeRow {
  return { subject: "", key: "", value: "", removed: false, stored: false };
}

/** 送る行（外していない行）。 */
export function keptAttributeRows(draft: AttributesDraft): AttributeRow[] {
  return draft.rows.filter((row) => !row.removed);
}

/** その欄が上限を超えているか。 */
export function isOverAttributeLimit(
  field: LimitedAttributeField,
  value: string,
): boolean {
  return countChars(value) > ATTRIBUTE_LIMITS[field];
}

/** 文字数の欄（`いま / 上限`）。**自分の欄の上限だけを出す。** */
export function attributeCounter(
  field: LimitedAttributeField,
  value: string,
): string {
  return `${countChars(value)} / ${ATTRIBUTE_LIMITS[field]}`;
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltReplaceAttributes =
  | { ok: true; body: ReplaceAttributesRequest }
  | { ok: false; message: string };

/** 主語が選ばれていないときの文言。**「外す」はこの画面に実在する操作。** */
const SUBJECT_REQUIRED_MESSAGE =
  "主語の選ばれていない行があります。主語を選ぶか、その行を「外す」でください。";

/**
 * **知らない主語の綴り**が保存されているときの文言。
 *
 * **「主語を選んでください」と言わないこと** ——
 * `<select>` は2つしか出さないので、**この状態は選ぶ操作では直らない。**
 * **綴りをそのまま出して、何が起きているかを言う**
 * （`statsEdit.ts` の `runModeLabel` が知らない綴りを落とさないのと同じ立場）。
 */
function unknownSubjectMessage(subject: string): string {
  return (
    `保存されている主語の綴り（${subject}）を、このサーバは受け付けません。` +
    "この行を「外す」で送らないようにするか、DB を直してください。"
  );
}

/** キーが空のときの文言。 */
const KEY_REQUIRED_MESSAGE =
  "キーの空いている行があります。キーを入れるか、その行を「外す」でください。";

/** 値が空のときの文言。**どのキーの話かを名乗る。** */
function valueRequiredMessage(key: string): string {
  return `${key} の値が空です。値を入れるか、その行を「外す」でください。`;
}

/** 上限を超えたときの文言。**その欄の上限だけを名乗る。** */
function overLimitMessage(field: LimitedAttributeField, key: string): string {
  const named = key.trim() === "" ? "" : `${key.trim()} の`;
  return `${named}${attributeFieldLabel(field)}が長すぎます（上限 ${ATTRIBUTE_LIMITS[field]} 文字）。`;
}

/** 件数が上限を超えたときの文言。**上限といまの件数を両方言う。** */
function tooManyMessage(count: number): string {
  return (
    `属性は ${MAX_ATTRIBUTES} 件までです（いま ${count} 件あります）。` +
    "どれかの行を「外す」でください。"
  );
}

/**
 * `PUT /admin/characters/{handle}/attributes` の本文を組み立てる。**全件を送る。**
 *
 * # ⚠ **ここで「変わった行だけ」に絞らないこと**
 *
 * 絞った瞬間、**絞られた属性は消える。**
 * `characterEdit.ts` の `buildUpdate`（差分だけ送る）とは**正反対の意味論**で、
 * 見た目が似ているぶん取り違えやすい。
 *
 * # 外した行だけは載せない（**それが「消す」の唯一の表現**）
 *
 * 外した行の中身が壊れていても、組み立ては止めない ——**送らない行だから。**
 *
 * # 値は生のまま送る
 *
 * キーも値も**正規化しない**（`AttributeKey::parse` / `AttributeValue::parse` が
 * サーバ側で行う）。空かどうかの判定にだけ `trim()` を使う。
 */
export function buildReplaceAttributes(
  draft: AttributesDraft,
): BuiltReplaceAttributes {
  const attributes: ReplaceAttributesRequest["attributes"] = [];
  for (const row of keptAttributeRows(draft)) {
    if (row.subject.trim() === "") {
      return { ok: false, message: SUBJECT_REQUIRED_MESSAGE };
    }
    // **綴りを型で narrow する**（`as` で押し込まない / [`knownAttributeSubject`]）。
    const subject = knownAttributeSubject(row.subject);
    if (subject === null) {
      return { ok: false, message: unknownSubjectMessage(row.subject) };
    }
    if (row.key.trim() === "") {
      return { ok: false, message: KEY_REQUIRED_MESSAGE };
    }
    if (isOverAttributeLimit("key", row.key)) {
      return { ok: false, message: overLimitMessage("key", row.key) };
    }
    if (row.value.trim() === "") {
      return { ok: false, message: valueRequiredMessage(row.key.trim()) };
    }
    if (isOverAttributeLimit("value", row.value)) {
      return { ok: false, message: overLimitMessage("value", row.key) };
    }
    attributes.push({ subject, key: row.key, value: row.value });
  }
  if (attributes.length > MAX_ATTRIBUTES) {
    return { ok: false, message: tooManyMessage(attributes.length) };
  }
  return { ok: true, body: { attributes } };
}

/**
 * **この置き換えで、履歴に何が残るか。**
 *
 * 写し元は `src/domain/attribute.rs` の `attribute_changes`（このファイルの冒頭）。
 * **`(subject, key)` の組で突き合わせ、値まで同じなら数えない。**
 *
 * **⚠ 前後の空白を落として比べる。** サーバは
 * `AttributeValue::parse`（`trim_blank`）を通してから比べるので、
 * **空白を足しただけの行を「変更」と数えると、要約が嘘になる。**
 */
export interface ReplacePlan {
  /** 保存後に残る件数（＝送る件数）。 */
  readonly kept: number;
  /** **無かった `(subject, key)` が増える**件数。 */
  readonly added: number;
  /** **あった `(subject, key)` の値が変わる**件数。 */
  readonly changed: number;
  /**
   * **消える行**（保存済みで、送られないもの）。**順は保存されている順。**
   *
   * 件数ではなく行そのものを返す ——**画面が名前を出すため。**
   */
  readonly removed: readonly AttributeView[];
}

/** 比べるための綴り（前後の空白を落とす / [`ReplacePlan`] の ⚠）。 */
function normalized(value: string): string {
  return value.trim();
}

/** その行が指す属性の同一性（**`(subject, key)` の組**）。 */
function sameAttribute(
  left: { subject: string; key: string },
  right: { subject: string; key: string },
): boolean {
  return (
    normalized(left.subject) === normalized(right.subject) &&
    normalized(left.key) === normalized(right.key)
  );
}

/** 保存すると履歴に何が残るか。**組み立ては純粋関数。** */
export function replacePlan(
  stored: readonly AttributeView[],
  draft: AttributesDraft,
): ReplacePlan {
  const kept = keptAttributeRows(draft);
  let added = 0;
  let changed = 0;
  for (const row of kept) {
    const held = stored.find((attribute) => sameAttribute(attribute, row));
    if (held === undefined) {
      added += 1;
      continue;
    }
    if (normalized(held.value) !== normalized(row.value)) {
      changed += 1;
    }
  }
  const removed = stored.filter(
    (attribute) => !kept.some((row) => sameAttribute(attribute, row)),
  );
  return { kept: kept.length, added, changed, removed };
}

/** 消える行の名前。**`「」` で囲まないこと**（操作名として読まれる / `statsEdit.ts` の ⚠）。 */
function removedLabel(attribute: AttributeView): string {
  return `『${attributeSubjectLabel(attribute.subject)}の${attribute.key}』`;
}

/**
 * **保存すると何が起きるかを、押す前に言う文。**
 *
 * **丸ごと置き換えであることを、画面の側で表す唯一の場所。**
 * **残る件数・履歴に残る3つの数・消える行の名前**を全部言う ——
 * 数だけだと「どれが消えるのか」が読めず、
 * 名前だけだと「静かに変わったことに気付く」ための記録が何件増えるのかが読めない。
 */
export function replaceSummary(
  stored: readonly AttributeView[],
  draft: AttributesDraft,
): string {
  const plan = replacePlan(stored, draft);
  const head =
    plan.kept === 0
      ? "保存すると、この属性は1件も残りません。"
      : `保存すると、この属性は ${plan.kept} 件になります。`;
  const counts =
    plan.added + plan.changed + plan.removed.length === 0
      ? "いまの内容と同じなので、変更の記録は増えません。"
      : `変更の記録には 追加 ${plan.added} 件 / 変更 ${plan.changed} 件 / 削除 ${plan.removed.length} 件 が残ります。`;
  const names =
    plan.removed.length === 0
      ? ""
      : `消えるのは${plan.removed.map(removedLabel).join("")}です` +
        "（置き換えなので、送らない行は消えます）。";
  return `${head}${counts}${names}`;
}

/**
 * **履歴の「変更前」の見出し。** `null` は**その属性が無かった**こと。
 *
 * **[`NO_AFTER_LABEL`] と同じ綴りにしないこと** ——
 * 同じにすると、**追加と削除が画面で同じ行に見える。**
 */
export const NO_BEFORE_LABEL = "（無かった）";

/** **履歴の「変更後」の見出し。** `null` は**消えた**こと。 */
export const NO_AFTER_LABEL = "（消えた）";

/** 変更前の欄。**綴りを決めるのはここだけ。** */
export function beforeLabel(before: string | null): string {
  return before === null ? NO_BEFORE_LABEL : before;
}

/** 変更後の欄。**綴りを決めるのはここだけ。** */
export function afterLabel(after: string | null): string {
  return after === null ? NO_AFTER_LABEL : after;
}

/** 見えていない履歴の件数。**総数と、出した件数の差。** */
export function hiddenRevisions(view: AttributeRevisionsView): number {
  return Math.max(0, view.total_revisions - view.revisions.length);
}

/**
 * **総数を必ず言う**（`AttributeRevisionsView::total_revisions` の doc ——
 * 上限を名乗るだけでは「たまたまその数」と「切られた」を区別できない）。
 *
 * **上限も応答のもの**（こちらに写しを持たない）。
 */
export function revisionCountNotice(view: AttributeRevisionsView): string {
  if (view.total_revisions === 0) {
    return "まだ変更はありません（この属性は、記録が残り始めてから一度も変わっていません）。";
  }
  const hidden = hiddenRevisions(view);
  if (hidden > 0) {
    return (
      `全 ${view.total_revisions} 件の変更のうち、新しい ${view.revisions.length} 件だけを出しています` +
      `（1回に返る上限は ${view.revisions_limit} 件 / 残り ${hidden} 件はここに出ていません）。`
    );
  }
  return `全 ${view.total_revisions} 件の変更を、すべて出しています（1回に返る上限は ${view.revisions_limit} 件）。`;
}

/**
 * サーバが返す `code`（**`src/server/error.rs` の綴りの写し**）。
 *
 * # ⚠ **値の写しなので、ずれると案内が黙って消える**
 *
 * 綴りが1文字でも食い違えば、**どの枝にも入らず `unknown` に落ちる。**
 * **画面は次の一手を言わなくなるのに、型検査もテストも緑のまま**
 * （`statsEdit.ts` / `characterDelete.ts` と同じ性質）。
 *
 * **写し元を読む見張り番は `tests/attributes-edit.test.ts`**（`error.rs` を実際に読む）。
 */
export const DUPLICATE_ATTRIBUTE_CODE = "duplicate_attribute";
/** 同上（`ApiError::invalid_attribute_key(AttributeKeyError::Control)`）。 */
export const KEY_CONTROL_CODE = "attribute_key_control";
/** 同上（`ApiError::invalid_attribute_value(AttributeValueError::Control)`）。 */
export const VALUE_CONTROL_CODE = "attribute_value_control";
/** 同上（`ApiError::character_not_found`）。 */
export const NOT_FOUND_CODE = "character_not_found";

/**
 * 失敗の種別。**`unknown` は「分類できなかった」であって「サーバの障害」ではない。**
 *
 * # なぜこの3つだけなのか
 *
 * **画面が止められない拒否だけ**を分ける。
 * 空・長すぎ・件数超過は[`buildReplaceAttributes`]が送る前に止めるので、
 * ここへは（上限が食い違っていない限り）来ない。
 *
 * | `code` | 状態 | 直す相手 |
 * |---|---|---|
 * | `duplicate_attribute` | 400 | **同じ主語の同じキーの行が2つある** |
 * | `attribute_key_control` / `attribute_value_control` | 400 | **見えない文字が混ざっている** |
 * | `character_not_found` | 404 | **そもそも居ない** |
 *
 * **長さ・空・件数の拒否は `unknown` に落とす。**
 * サーバの文言が**壊れている欄を自分で名乗る**ので、
 * こちらから言い換えると情報が減るだけになる（`statsEdit.ts` と同じ判断）。
 */
export type ReplaceAttributesFailureReason =
  | "duplicate"
  | "control"
  | "not-found"
  | "unknown";

/** 画面に出すもの。 */
export interface ReplaceAttributesFailure {
  reason: ReplaceAttributesFailureReason;
  /** **サーバが言ったこと。** 読めなかったときだけこちらの定型文になる。 */
  message: string;
  /**
   * **次に取れる行動。** 分からないなら `null`。
   *
   * **`null` を埋めないこと。** 分類できていない失敗に一手を書くと、
   * それは推測であり、外れたときに利用者を遠回りさせる。
   */
  guidance: string | null;
}

/**
 * **同じ属性を2回指定した**ときの一手。
 *
 * **⚠ 実在する導線だけを指す**（`characterDelete.ts` の ⚠ / B-15）。
 */
const DUPLICATE_GUIDANCE =
  "同じ主語の同じキーの行が2つあります。片方を「外す」か、キーを直してください。" +
  "（置き換え全体が取り消されているので、保存済みの値は変わっていません。）";

/**
 * **見えない文字が混ざった**ときの一手。
 *
 * **貼り付け事故がここへ来る** —— 画面の欄では見えないので、
 * **打ち直すしかない**ことを言う。
 */
const CONTROL_GUIDANCE =
  "キーか値に、改行や制御文字のような見えない文字が混ざっています。" +
  "貼り付けた文字を消して、手で打ち直してください。" +
  "（置き換え全体が取り消されているので、保存済みの値は変わっていません。）";

/** **居ない**と言われたときの一手。**取り直し**で通る。 */
const NOT_FOUND_GUIDANCE =
  "すでに消えているのかもしれません。「キャラクターの一覧へ」で取り直してください。";

/**
 * 失敗を、画面に出せる形へ。
 *
 * **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約:
 * エラーに内部の詳細を載せない）。
 */
export function describeReplaceAttributesFailure(
  failure: unknown,
): ReplaceAttributesFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { reason: "unknown", message: "予期しない失敗です", guidance: null };
  }
  switch (failure.code) {
    case DUPLICATE_ATTRIBUTE_CODE:
      return {
        reason: "duplicate",
        message: failure.message,
        guidance: DUPLICATE_GUIDANCE,
      };
    case KEY_CONTROL_CODE:
    case VALUE_CONTROL_CODE:
      return {
        reason: "control",
        message: failure.message,
        guidance: CONTROL_GUIDANCE,
      };
    case NOT_FOUND_CODE:
      return {
        reason: "not-found",
        message: failure.message,
        guidance: NOT_FOUND_GUIDANCE,
      };
    default:
      return { reason: "unknown", message: failure.message, guidance: null };
  }
}
