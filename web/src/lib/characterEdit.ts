/**
 * 作成・編集の**下書きから、送る本文を組み立てる**（作業単位 D-2）。
 *
 * **I/O を持たない。** 送るのは `api.ts`、画面は `CharacterForm.svelte`。
 * ここに寄せてあるのは、**「何を送るか」だけを単体で見張れるようにする**ため
 * （`PATCH` の「キーの有無が意味を持つ」は、画面越しには観測しづらい）。
 *
 * # ⚠ **上限は `src/domain/character.rs` の写しである**
 *
 * `api.ts` の型と同じで**生成していない**（ADR-0031 §6）。ただし型と違い、
 * **ずれると画面が黙って嘘の数を出す**（「600 文字まで」と数えておきながら
 * サーバが 400 を返す、あるいはその逆）ので、**ここだけは見張り番を置いた** ——
 * `tests/character-edit.test.ts` が **`src/domain/character.rs` を実際に読んで
 * 突き合わせる。**
 *
 * **⚠ それでも守れていないものがある。**
 * **`Handle` と `DisplayName` はどちらも 50 なので、この2つの取り違えは
 * 突き合わせでは捕まらない**（写し元で同値）。
 *
 * # 写していないもの（**増やさないこと**）
 *
 * | 写していない | 代わりに |
 * |---|---|
 * | `Handle::ALLOWED_CHARACTERS` | **ここでは判定しない。** 打たせて、サーバの理由を見せる |
 * | `StatScale::MIN` / `MAX` | 同上 |
 * | `domain::text::TRIMMABLE` | `countChars` の JS の `trim()`（**上位集合**。下記） |
 *
 * > **⚠ `ALLOWED_CHARACTERS` と `StatScale::DEFAULT` は、画面の説明文に写してある**
 * > （`CharacterForm.svelte` / 作業単位 D-14）。**判定には使わない、読ませるだけの写し**で、
 * > 写し元を読む見張り番は `tests/character-form-examples.test.ts`。
 */

import type {
  CharacterView,
  CreateCharacterRequest,
  UpdateCharacterRequest,
} from "./api";
import {
  FOLLOW_DEFAULT,
  driftModeDraftOf,
  driftModeWireOf,
} from "./driftModeChoice";

/**
 * 画面が持つ下書き。**全部が文字列**（`<input>` の値そのもの）。
 *
 * 数値へ直すのは送る直前で、**直せなければ送らない**（`buildCreate` / `buildUpdate`）。
 */
export interface CharacterDraft {
  /** **作成のときだけ意味がある。** 編集では変えられない（ADR-0016 §1）。 */
  handle: string;
  /**
   * **空の意味が、作成と編集で違う。**
   *
   * | | 空にしたとき |
   * |---|---|
   * | 作成 | **キーごと省略**され、サーバが `handle` の値で埋める |
   * | 編集 | **`""` を送る**ので、サーバが「表示名が空です」で 400 |
   *
   * **揃えていないのは意図的。** 編集で空を「変えない」に丸めると、
   * **消したつもりの利用者が 200 を受け取って、値は元のまま**になる。
   * この列は NOT NULL なので「消す」は元から表現できない
   * （`output_format_instruction` だけが3値なのはそのため）。
   */
  display_name: string;
  base_persona: string;
  /** **空文字は「未設定」。** 編集では「消す」になる。 */
  output_format_instruction: string;
  /** **空文字は「省略」。** 作成ではサーバの既定、編集では「変えない」。 */
  stat_scale: string;
  /**
   * **変容の走らせ方**（作業単位 D-15 / `driftModeChoice.ts`）。
   * **空文字は「既定に従う」**（作成ではキーごと省く / 編集では `null` を送る）。
   *
   * **編集で元の値と同じなら送らない**（他の欄と同じ差分の約束）。
   */
  stat_drift_mode: string;
}

/**
 * 上限（**`src/domain/character.rs` の `MAX_CHARS` の写し**）。
 *
 * キーは**送る本文のキーと同じ綴り**にしてある（欄と上限の対応を1箇所に保つため）。
 */
export const LIMITS = {
  /** `Handle::MAX_CHARS`。 */
  handle: 50,
  /** `DisplayName::MAX_CHARS`。 */
  display_name: 50,
  /** `Persona::MAX_CHARS`。 */
  base_persona: 600,
  /** `OutputFormat::MAX_CHARS`。 */
  output_format_instruction: 150,
} as const;

/** 上限を持つ欄。 */
export type LimitedField = keyof typeof LIMITS;

/** 欄の見出し。**画面とメッセージで同じ綴りを使う**（写しを増やさない）。 */
export function labelOf(field: LimitedField): string {
  switch (field) {
    case "handle":
      return "識別子";
    case "display_name":
      return "表示名";
    case "base_persona":
      return "ペルソナ本文";
    case "output_format_instruction":
      return "出力形式指示";
  }
}

/** 空の下書き（新規作成の初期値）。 */
export function emptyDraft(): CharacterDraft {
  return {
    handle: "",
    display_name: "",
    base_persona: "",
    output_format_instruction: "",
    stat_scale: "",
    stat_drift_mode: FOLLOW_DEFAULT,
  };
}

/** 既存のキャラクターを下書きに写す（編集の初期値）。 */
export function draftOf(character: CharacterView): CharacterDraft {
  return {
    handle: character.handle,
    display_name: character.display_name,
    base_persona: character.base_persona,
    // **未設定は空文字。** `null` を画面へ出さない。
    output_format_instruction: character.output_format_instruction ?? "",
    stat_scale: String(character.stat_scale),
    stat_drift_mode: driftModeDraftOf(character.stat_drift_mode),
  };
}

/**
 * サーバと同じ数え方で数える。**符号位置**（Rust の `chars().count()`）。
 *
 * **`.length` を使わないこと。** UTF-16 の個数なので `🐈` が 2 になり、
 * **絵文字を含むペルソナだけ、画面の数がサーバの数より多くなる。**
 *
 * # 前後の空白を落とすのは、サーバがそうするから
 *
 * `Persona::parse` などは**正規化してから数える**ので、
 * 末尾の改行だけで上限を超えたことにはならない。
 *
 * **⚠ JS の `trim()` は `domain::text::TRIMMABLE` の上位集合**
 * （U+2028 / U+2029 / U+FEFF を余分に落とす）。**`TRIMMABLE` を写さない**代わりに、
 * **外れる向きが「こちらが少なく数える」に固定される** ——
 * つまり**こちらが早まって弾くことはなく**、サーバだけが弾く回は
 * サーバのメッセージが画面に出る。見張り番は
 * `tests/character-edit.test.ts::サーバより多く数えることはない`。
 */
export function countChars(value: string): number {
  return [...value.trim()].length;
}

/** その欄が上限を超えているか。 */
export function isOverLimit(field: LimitedField, value: string): boolean {
  return countChars(value) > LIMITS[field];
}

/**
 * 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。**
 *
 * `kind` が**送り先そのもの**（`create` = `POST` / `update` = `PATCH`）。
 * 画面はこの値で分岐するので、**「どちらへ送るか」を画面が別に持たない。**
 */
export type BuiltRequest =
  | { ok: true; kind: "create"; body: CreateCharacterRequest }
  | { ok: true; kind: "update"; body: UpdateCharacterRequest }
  | { ok: false; message: string };

/** 作成のときに数える欄（`handle` を含む）。 */
const CREATE_FIELDS: readonly LimitedField[] = [
  "handle",
  "display_name",
  "base_persona",
  "output_format_instruction",
];

/**
 * 編集のときに数える欄。**`handle` を含まない**（変えられないため）。
 *
 * 含めると、**保存済みの識別子が上限を超えている行**（＝上限を下げた後の DB）で
 * 編集そのものができなくなる。直す入口を塞ぐのが最も避けたい壊れ方。
 */
const UPDATE_FIELDS: readonly LimitedField[] = [
  "display_name",
  "base_persona",
  "output_format_instruction",
];

/**
 * 上限を超えている欄を全部挙げる。**1つ目で止めない**（直し漏れが出る）。
 *
 * **修復モード（`characterRepair.ts` / 作業単位 D-6）もここを通る。**
 * 判定を2つ持つと、片方だけが上限の扱いを取りこぼす形で古びる。
 *
 * **下書きの型ではなく「その欄を持っていること」だけを要求する**
 * （`Record<F, string>`）。修復の下書きには識別子の欄が無いので、
 * `CharacterDraft` を要求すると通らない ——
 * **通すために `handle: ""` を持たせるのが、いちばん避けたい形**
 * （変えられない欄の空値が、下書きの中に生まれる）。
 */
export function overLimitFields<F extends LimitedField>(
  draft: Record<F, string>,
  fields: readonly F[],
): F[] {
  return fields.filter((field) => isOverLimit(field, draft[field]));
}

/**
 * 上限超過のメッセージ。**超えている欄を全部名乗る。**
 *
 * **⚠ 「その欄の上限」を出すこと自体が、取り違えうる。**
 * `LIMITS[field]` を `LIMITS.base_persona` に決め打っても、
 * **止まる／止まらないは1文字も変わらない**（判定は `overLimitFields` の側）ので、
 * **画面だけが「識別子（上限 600 文字）が長すぎます」と嘘を言う。**
 *
 * **見張り番は `tests/character-edit.test.ts::`
 * `メッセージは、その欄の上限を出す`** ——
 * **自分の上限が出ることと、他の欄の上限が出ないことを両方見る。**
 */
export function overLimitMessage(fields: readonly LimitedField[]): string {
  const named = fields
    .map((field) => `${labelOf(field)}（上限 ${LIMITS[field]} 文字）`)
    .join("、");
  return `${named}が長すぎます。減らしてから保存してください。`;
}

/**
 * 尺度の読み取り結果。
 *
 * **`omitted` の意味は、モードごとに違う。** 作成ではサーバの既定、
 * 編集では「変えない」、**修復では「まだ入力していない」**（送れない /
 * `characterRepair.ts` の `SCALE_REQUIRED_MESSAGE`）。
 * **読み取りは1つ、意味付けは呼び出し側**という分け方にしてある。
 */
export type Scale =
  | { kind: "omitted" }
  | { kind: "value"; value: number }
  | { kind: "invalid" };

/**
 * 尺度を整数として読む。**空は「省略」。**
 *
 * **上下限は見ない**（`StatScale::MIN` / `MAX` を写さない）。
 * 範囲外はサーバが理由付きで返す（`stat_scale_too_small` / `_too_large`）。
 */
export function readScale(raw: string): Scale {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { kind: "omitted" };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { kind: "invalid" };
  }
  const value = Number.parseInt(trimmed, 10);
  return Number.isSafeInteger(value) ? { kind: "value", value } : { kind: "invalid" };
}

/**
 * 尺度が読めなかったときの文言。
 *
 * **範囲を書かない**（`StatScale::MIN` / `MAX` を写さない）。
 * ここが見ているのは「JSON の数として送れるか」だけで、**範囲はサーバが判定する。**
 */
export const SCALE_MESSAGE =
  "尺度は整数で入力してください（使える範囲はサーバが判定します）。";

/**
 * `POST /admin/characters` の本文を組み立てる。
 *
 * **空の欄はキーごと落とす。** 空文字を送るとサーバは「空です」で 400 にする
 * （未設定は `None` で表す、という `OutputFormat` の約束）。
 * **落とせばサーバの既定に載る**ので、意味が変わらない。
 */
export function buildCreate(draft: CharacterDraft): BuiltRequest {
  const over = overLimitFields(draft, CREATE_FIELDS);
  if (over.length > 0) {
    return { ok: false, message: overLimitMessage(over) };
  }

  const scale = readScale(draft.stat_scale);
  if (scale.kind === "invalid") {
    return { ok: false, message: SCALE_MESSAGE };
  }

  // **`handle` と `base_persona` は落とさない**（省略できない）。
  // 空のまま送るとサーバが「空です」と理由を返す——こちらで別の言い方をしない。
  //
  // **⚠ `handle` を `trim` しない**（ADR-0017）。`Handle::parse` は
  // **直さずに拒否する**——前後に空白の付いた識別子を**表現できなくする**のが要点で、
  // **こちらで黙って直すと、貼り付け事故が「気付かないまま別の綴りで作られる」形になる。**
  // 表示名・ペルソナは逆で、**サーバが正規化する**ので生のまま送る。
  const body: CreateCharacterRequest = {
    handle: draft.handle,
    base_persona: draft.base_persona,
  };
  if (draft.display_name.trim() !== "") {
    body.display_name = draft.display_name;
  }
  if (draft.output_format_instruction.trim() !== "") {
    body.output_format_instruction = draft.output_format_instruction;
  }
  if (scale.kind === "value") {
    body.stat_scale = scale.value;
  }
  // **「既定に従う」はキーごと省く**（サーバは省略を未設定と読む / 作業単位 D-15）。
  const driftMode = driftModeWireOf(draft.stat_drift_mode);
  if (driftMode !== null) {
    body.stat_drift_mode = driftMode;
  }
  return { ok: true, kind: "create", body };
}

/**
 * `PATCH /admin/characters/{handle}` の本文を組み立てる。**差分だけ。**
 *
 * # 変えていない欄をキーごと落とす理由
 *
 * サーバは「キーの有無」で「変えない」を表す（`admin.rs` の
 * `UpdateCharacterRequest`）。**同じ値で送っても結果は同じ**に見えるが、
 * **別の窓や `curl` で先に直された値を、こちらの古い写しで踏み潰す**ことになる。
 * **「変えない」を実際に「触らない」で表すのが、この関数の仕事。**
 *
 * # 識別子は絶対に載せない
 *
 * 不変（ADR-0016 §1）。しかもサーバは `deny_unknown_fields` なので、
 * 混ぜると**1列も書かれない 400** になる。
 */
export function buildUpdate(
  original: CharacterView,
  draft: CharacterDraft,
): BuiltRequest {
  const over = overLimitFields(draft, UPDATE_FIELDS);
  if (over.length > 0) {
    return { ok: false, message: overLimitMessage(over) };
  }

  const scale = readScale(draft.stat_scale);
  if (scale.kind === "invalid") {
    return { ok: false, message: SCALE_MESSAGE };
  }

  const body: UpdateCharacterRequest = {};
  if (draft.display_name !== original.display_name) {
    body.display_name = draft.display_name;
  }
  if (draft.base_persona !== original.base_persona) {
    body.base_persona = draft.base_persona;
  }

  // **形式指示だけが3値。** 空にしたら「消す」（`null`）、
  // 元から未設定で空のままなら「変えない」（キーごと落とす）。
  const format = draft.output_format_instruction;
  const cleared = format.trim() === "";
  if (cleared) {
    if (original.output_format_instruction !== null) {
      body.output_format_instruction = null;
    }
  } else if (format !== original.output_format_instruction) {
    body.output_format_instruction = format;
  }

  // **空欄は「変えない」。** この列は NOT NULL なので「消す」に意味が無い。
  if (scale.kind === "value" && scale.value !== original.stat_scale) {
    body.stat_scale = scale.value;
  }

  // **走らせ方も3値**（作業単位 D-15）。**元の値と同じならキーごと落とす**
  // ——触っていないのに `null` を送ると、「変えない」が「未設定に戻す」に化ける。
  const driftMode = driftModeWireOf(draft.stat_drift_mode);
  if (driftMode !== original.stat_drift_mode) {
    body.stat_drift_mode = driftMode;
  }

  return { ok: true, kind: "update", body };
}

/** 送るものが1つも無いか（編集で「保存」を押させないため）。 */
export function isEmptyUpdate(body: UpdateCharacterRequest): boolean {
  return Object.keys(body).length === 0;
}
