/**
 * **壊れた行（`unreadable`）を修復する**（作業単位 D-6 / ADR-0016 §3・§4）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `updateCharacter`、
 * 画面は `CharacterRepairForm.svelte`、入口は `CharacterList.svelte`。
 *
 * # なぜ D-2（`characterEdit.ts`）では作れなかったのか
 *
 * **ADR-0016 §4 の核心は「更新は、既存の行を `parse` せずに書けること」**である。
 * ところが **D-2 の編集は差分だけを送る**（`buildUpdate`）ので、
 * **差分を取る元の `CharacterView` が要る。**
 * **壊れた行にはそれが無い** —— `UnreadableCharacterView` は
 * `id` と `handle` と**読めなかった列名**しか持たず、
 * **格納値そのものは1つも返ってこない**（`admin.rs` の `UnreadableCharacterView` の doc）。
 *
 * **だからここは「全部の欄を送る」第3のモードになる。**
 *
 * # ⚠ **壊れていない列も、ここに書いた値で上書きされる**
 *
 * **承知のうえで選んでいる。** 元の値は**読み出す手段が無い**ので、
 * 「壊れていない列だけ残す」を選んでも**残った値を確かめる方法が無い。**
 * 修復の目的は**その行を既知の状態へ戻すこと**なので、全部を書き直す。
 * **画面は `invalid_columns` を出す**ので、どの列が実際に壊れていたかは分かる。
 *
 * **例外は変容の走らせ方だけ**（作業単位 D-15 のレビューの是正）——
 * 列が壊れていない行では「変えない」から始め、触らなければキーを省く
 * （`RepairDraft.stat_drift_mode` の doc）。**他の列に同じ扱いを広げないこと** ——
 * 他の列は、壊れていなくても**元の値を画面に出せない**ので「変えない」を選ばせても
 * 何が残るのかを利用者が確かめられない（走らせ方は4つの値しか無く、
 * 残る値はどれでも正当である）。
 *
 * # ⚠ 直せない列は**識別子ただ1つ**
 *
 * `CharacterColumn` は5つ（`handle` / `display_name` / `base_persona` /
 * `output_format_instruction` / `stat_scale`）で、
 * **`PATCH` の要求型に口があるのは後ろの4つだけ**（識別子は不変 / ADR-0016 §1）。
 *
 * **`handle` が `null` の行 ⟺ 識別子そのものが `parse` を通らない行**なので、
 * **その行はどの API からも指せない**（`domain::UnreadableCharacter` の `handle` の doc
 * が「**この行を直す手段は `sqlite3` だけ**」と書いている）。
 * **だから修復の入口を出さない** —— **無い操作を案内しない**（D-7 と同じ立場）。
 *
 * # 上限は写さない（`characterEdit.ts` のものをそのまま使う）
 *
 * 写しを2つ持つと片方だけが古びる。**写し元を読む見張り番は
 * `tests/character-edit.test.ts`**（`src/domain/character.rs` を実際に読む）。
 */

import { ApiRequestError } from "./api";
import type { UnreadableCharacterView, UpdateCharacterRequest } from "./api";
import {
  labelOf,
  overLimitFields,
  overLimitMessage,
  readScale,
  SCALE_MESSAGE,
  type LimitedField,
} from "./characterEdit";
import {
  DRIFT_MODE_FIELD_LABEL,
  FOLLOW_DEFAULT,
  driftModeChoices,
  driftModeWireOf,
  type DriftModeChoice,
} from "./driftModeChoice";

/**
 * **修復で書き直せる列**（`CharacterColumn::as_str` の綴りの写し）。
 *
 * # ⚠ **値の写しなので、ずれると静かに外れる**
 *
 * 綴りが1文字でも食い違えば、**壊れている欄に印が付かなくなる。**
 * 画面は「読めなかった列」を並べたまま、**どの欄を直せばよいかを言わなくなる**のに、
 * **型検査もテストも緑のまま。**
 *
 * **したがって写し元を実際に読む見張り番を置いた** ——
 * `tests/character-repair.test.ts` の
 * **「`src/domain/character.rs` の `CharacterColumn::as_str` と一致する」**。
 * **先例は `characterEdit.ts` の `LIMITS` と `characterDelete.ts` の `code`。**
 *
 * **並び順は画面の欄の並び。** 送る本文のキーとも同じ綴りにしてある。
 */
export const REPAIR_COLUMNS = [
  "display_name",
  "base_persona",
  "output_format_instruction",
  "stat_scale",
  // **変容の走らせ方**（ADR-0045 / 作業単位 D-15）。**`PATCH` に口がある**ので直せる。
  "stat_drift_mode",
] as const;

/** 修復で書き直せる列名。 */
export type RepairColumn = (typeof REPAIR_COLUMNS)[number];

/**
 * **この画面から直せない列**（同上の写し）。
 *
 * 識別子は不変（ADR-0016 §1）。`PATCH` の要求型に口が無いので、
 * **この列が壊れている行は `sqlite3` でしか直せない。**
 */
export const UNREPAIRABLE_COLUMN = "handle";

/**
 * **サーバが指し示しうるが、この画面にまだ欄が無い列**（同上の写し）。
 *
 * `axis_discovery_slots`（見いだす枠の数 / ADR-0046 §決定1 / 作業単位 A-9a）は
 * **`PATCH` に口がある**（`UpdateCharacterRequest.axis_discovery_slots`）が、
 * **画面は ADR-0046 の第2段**で、この修復の画面にも欄を作っていない。
 * **だから修復で書き直す列（`REPAIR_COLUMNS`）には入れない** —— 入れると、
 * 送らない列を「直す」と案内することになる。
 *
 * **この列だけが壊れた行は、この画面で修復しても読めるようにならない**
 * （`PATCH` を直接送るか、第2段で欄を作る）。見出しは `columnLabel` の
 * 「知らない綴りはそのまま返す」に落ちる（**壊れている列が画面から消えない**）。
 *
 * `archived_at`（しまった時刻 / ADR-0049 §決定1 / 作業単位 A-10）も**修復の下書きには欄が無い**。
 * 直す口は `PATCH` の `archived: false`（時刻を消す＝戻す。`true` は壊れた値を残す）で、
 * **画面の入口は、読み出せなかった行の「しまった時刻を消す」**（[`archivedAtFixOf`]）。
 * 全欄モードの修復に混ぜないのは、時刻だけが壊れた行で、読めている他の欄まで打ち直させないため。
 */
export const COLUMNS_WITHOUT_A_FIELD = [
  "axis_discovery_slots",
  "archived_at",
] as const;

/** しまった時刻の列名（**`src/domain/character.rs` の写し**。上の一覧と同じ見張り番が見る）。 */
export const ARCHIVED_AT_COLUMN: (typeof COLUMNS_WITHOUT_A_FIELD)[1] = "archived_at";

/**
 * **しまった時刻だけを直せる行か**（作業単位 A-10）。直せるならその識別子、直せないなら `null`。
 *
 * 時刻が読めない行は、しまった一覧にも上の一覧にも出ない（読み出せなかった行に出る）ので、
 * 「戻す」の釦が届かない。**直す手段が `PATCH {"archived": false}` しか無い状態を、画面に出す。**
 *
 * **識別子が読めない行は `null`**（`repairTargetOf` と同じ —— `PATCH` で指せない）。
 */
export function archivedAtFixOf(broken: UnreadableCharacterView): string | null {
  if (broken.handle === null || !broken.invalid_columns.includes(ARCHIVED_AT_COLUMN)) {
    return null;
  }
  return broken.handle;
}

/** その列名が、この画面から直せる列か。 */
export function isRepairColumn(name: string): name is RepairColumn {
  return (REPAIR_COLUMNS as readonly string[]).includes(name);
}

/** 尺度の欄の見出し。**`LIMITS` に無い**（文字数の上限を持たない列）ので `labelOf` に無い。 */
const SCALE_LABEL = "性格ステータスの尺度";

/**
 * 列名の見出し。
 *
 * **知らない綴りは、そのまま返す。** サーバが列を増やした日に
 * **壊れている列が画面から消える**のがいちばん質の悪い壊れ方なので、
 * 訳せなくても落とさない（`MemoryList` が生の主語を併記しているのと同じ立場）。
 */
export function columnLabel(name: string): string {
  if (name === "stat_scale") {
    return SCALE_LABEL;
  }
  // **`LIMITS` に無い**（文字数の上限を持たない列）ので `labelOf` には無い。
  // ここで拾わないと `labelOf` の網羅から外れて `undefined` が画面に出る。
  if (name === "stat_drift_mode") {
    return DRIFT_MODE_FIELD_LABEL;
  }
  if (name === UNREPAIRABLE_COLUMN || isRepairColumn(name)) {
    return labelOf(name as LimitedField);
  }
  return name;
}

/**
 * **修復できる壊れた行。**
 *
 * # ⚠ **型では塞げていない**
 *
 * `handle` が `string` なので**「指せない行はこの型を作れない」と読めるが、
 * 実際にはそうではない。** `handle: broken.handle ?? ""` と書けば作れてしまい、
 * **`pnpm run check` は 0 エラーで通る**（D-6 のレビューで実測）。
 *
 * **守っているのは `repairTargetOf` の `null` ガードと、それを見るテスト**
 * （`tests/character-repair.test.ts::識別子が読めない行には、入口が無い` ほか
 * 計4本。ガードを外す変異で実際に赤になる）。
 *
 * **`ValidTag` と同じ強さに読める文言をここに書かないこと**（CLAUDE.md / `0012b`）。
 * そう読まれると「型が守るから見張り番は要らない」という経路ができる。
 */
export interface RepairableRow {
  /** 主キー。**この行を指す唯一の確実な手掛かり。** */
  id: number;
  /** 送り先。**`Handle::parse` を通った綴り**なので、パスに貼れる。 */
  handle: string;
  /** 読めなかった列（**DB の列名と同じ綴り**）。 */
  invalid_columns: string[];
}

/**
 * 壊れた行を、**修復できるものだけ**に絞る。
 *
 * **`handle` が `null` なら `null`。** その行は識別子そのものが壊れていて、
 * `PATCH` でも `DELETE` でも指せない（上の ⚠）。
 * **画面はこの `null` を見て、入口を出さない。**
 */
export function repairTargetOf(
  broken: UnreadableCharacterView,
): RepairableRow | null {
  if (broken.handle === null) {
    return null;
  }
  return {
    id: broken.id,
    handle: broken.handle,
    invalid_columns: broken.invalid_columns,
  };
}

/**
 * 修復の下書き。**全部が文字列**（`<input>` の値そのもの）。
 *
 * **識別子の欄は無い**（不変 / ADR-0016 §1）。
 *
 * **`CharacterDraft` と別の型にしてある。** まとめると
 * **「元の値から始まる下書き」と「元の値が無い下書き」が同じ表現**になり、
 * **`draftOf` で埋め忘れた欄と、意図して空のままの欄が区別できなくなる。**
 */
export interface RepairDraft {
  display_name: string;
  base_persona: string;
  /** **空文字は「未設定に戻す」**（`null` を送る）。 */
  output_format_instruction: string;
  /** **空文字は「まだ入力していない」。** 省略ではない（下の `buildRepair`）。 */
  stat_scale: string;
  /**
   * **変容の走らせ方**（作業単位 D-15）。
   *
   * | 値 | 送るもの |
   * |---|---|
   * | [`KEEP_DRIFT_MODE`]（**列が壊れていない行の初期値**） | **キーを省く**（サーバは省いた列を残す） |
   * | `""`（既定に従う / **列が壊れている行の初期値**） | `null`（未設定に戻す） |
   * | `off` / `shadow` / `on` | その値 |
   *
   * # 列が壊れていない行では「変えない」から始める（D-15 のレビューの是正）
   *
   * **他の列は元の値が読めないので全部送るが、この列が壊れていないことは
   * `invalid_columns` から分かる**（その行の他の列が壊れていても、この列の値は正しい）。
   * 初期値を「既定に従う」にしていたころは、**正しい値を未設定で上書きしていた。**
   *
   * **列が壊れている行では「変えない」を出さない**（壊れた値を残す選択になる）。
   */
  stat_drift_mode: string;
}

/**
 * **「変えない」を表す番兵**（修復の下書きだけで使う / 作業単位 D-15）。
 *
 * **どの走らせ方の綴りとも、「既定に従う」（`""`）とも違う値。**
 * サーバの綴りは小文字の英字なので、括弧を含むこの値とは重ならない。
 */
export const KEEP_DRIFT_MODE = "(keep)";

/**
 * **修復で出す走らせ方の選択肢**（作業単位 D-15）。
 *
 * * **列が壊れている行** —— 4つ（`driftModeChoices`）。**括弧は付けない**
 *   （この行は読めないので、既定を言う変容の応答も開かない）。
 * * **壊れていない行** —— 先頭に「変えない（いまの設定のまま）」を足した5つ。
 */
export function repairDriftModeChoices(driftModeBroken: boolean): DriftModeChoice[] {
  const four = driftModeChoices(null);
  if (driftModeBroken) {
    return four;
  }
  return [{ value: KEEP_DRIFT_MODE, label: "変えない（いまの設定のまま）" }, ...four];
}

/**
 * 空の下書き。**何も埋めない。**
 *
 * 元の値は API から返ってこない（`UnreadableCharacterView` は
 * `id` と `handle` と壊れた列名しか持たない）。
 * **推測で埋めない** —— `memoryRepair.ts` の `draftForUnreadable` と同じ立場で、
 * 推測した値が「いま正しい設定」として保存される。
 *
 * **`driftModeBroken`（走らせ方の列が壊れているか）は必須**（作業単位 D-15）——
 * 初期値がそれで変わる（`RepairDraft.stat_drift_mode` の doc）。
 * **省略できる形にしない**：省略の既定がどちらでも、呼び出し側が決め忘れた行で
 * 正しい値を上書きするか、壊れた値を残すかのどちらかになる。
 */
export function emptyRepairDraft(driftModeBroken: boolean): RepairDraft {
  return {
    display_name: "",
    base_persona: "",
    output_format_instruction: "",
    stat_scale: "",
    stat_drift_mode: driftModeBroken ? FOLLOW_DEFAULT : KEEP_DRIFT_MODE,
  };
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltRepair =
  | { ok: true; body: UpdateCharacterRequest }
  | { ok: false; message: string };

/**
 * 文字数を数える欄。**`handle` を含まない**（修復では変えられないうえ、
 * **上限を下げた後の DB で、識別子が長すぎる行の修復を塞ぐ**ため /
 * `characterEdit.ts` の `UPDATE_FIELDS` と同じ理由）。
 *
 * **`stat_scale` も含まない** —— 文字数の上限を持たない列（`LIMITS` に無い）。
 */
export type RepairLimitedField = Exclude<LimitedField, "handle">;

/** 同上（並びは画面の欄の並び）。 */
const REPAIR_LIMITED_FIELDS: readonly RepairLimitedField[] = [
  "display_name",
  "base_persona",
  "output_format_instruction",
];

/**
 * **尺度を空のまま送れない理由。**
 *
 * この列は NOT NULL で、**保存されている値も読み出せない**
 * （`UnreadableCharacterView` は値を返さない）。
 * キーごと落とすとサーバは「変えない」と読むので、
 * **壊れている尺度がそのまま残り、行はいつまでも読めない。**
 *
 * **範囲は書かない**（`StatScale::MIN` / `MAX` を写さない / `characterEdit.ts` と同じ）。
 */
const SCALE_REQUIRED_MESSAGE =
  "尺度は省略できません（この行は保存されている値を読み出せないため、必ず入力してください）。";

/**
 * `PATCH /admin/characters/{handle}` の本文を組み立てる。**全部の欄を送る。**
 *
 * # 差分モード（`buildUpdate`）と何が違うのか
 *
 * **キーを1つも落とさない。** サーバは「キーの有無」で「変えない」を表すので、
 * **落とした列は壊れたまま残る。**
 * 元の値が読めない以上「変えていない欄」を決めようがないので、
 * **4つとも必ず載せる**のがこのモードの定義そのものである。
 *
 * # 空欄は空のまま送る
 *
 * **こちらで別の言い方をしない**（`buildCreate` と同じ立場）——
 * `DisplayName::parse("")` は「表示名が空です」で 400 を返すので、
 * **サーバが名指しした理由がそのまま画面に出る。**
 * 形式指示だけは3値で、**空は `null`（未設定に戻す）**という正当な指示になる。
 */
export function buildRepair(draft: RepairDraft): BuiltRepair {
  const over = overLimitFields(draft, REPAIR_LIMITED_FIELDS);
  if (over.length > 0) {
    return { ok: false, message: overLimitMessage(over) };
  }

  const scale = readScale(draft.stat_scale);
  if (scale.kind === "omitted") {
    return { ok: false, message: SCALE_REQUIRED_MESSAGE };
  }
  if (scale.kind === "invalid") {
    return { ok: false, message: SCALE_MESSAGE };
  }

  // **⚠ ここでキーを条件付きにしないこと。** 条件を1つ足した瞬間、
  // このモードは差分モードに戻り、**その列は壊れたまま残る。**
  // 見張り番は `tests/character-repair.test.ts::`
  // `同じ内容でも、差分モードはキーを落とし、修復モードは落とさない`。
  const body: UpdateCharacterRequest = {
    display_name: draft.display_name,
    base_persona: draft.base_persona,
    output_format_instruction:
      draft.output_format_instruction.trim() === ""
        ? null
        : draft.output_format_instruction,
    stat_scale: scale.value,
  };
  // **走らせ方だけは「変えない」を持てる**（作業単位 D-15 のレビューの是正）——
  // 列が壊れていないことが `invalid_columns` から分かる行だけ。
  // **「既定に従う」は `null`＝未設定に戻す**（変えないとは別の表現）。
  if (draft.stat_drift_mode !== KEEP_DRIFT_MODE) {
    body.stat_drift_mode = driftModeWireOf(draft.stat_drift_mode);
  }
  return { ok: true, body };
}

/**
 * サーバが返す `code`（**`src/server/error.rs` の綴りの写し**）。
 *
 * **`characterDelete.ts` の `HAS_SESSIONS_CODE` と同じ性質の写し**である ——
 * 綴りがずれると**どちらの枝にも入らず `unknown` に落ち、案内が黙って消える。**
 * **写し元を読む見張り番は `tests/character-repair.test.ts`。**
 */
export const SCALE_BELOW_STORED_CODE = "stat_scale_below_stored_value";
/** 同上（`ApiError::character_not_found`）。 */
export const NOT_FOUND_CODE = "character_not_found";

/**
 * 失敗の種別。**`unknown` は「分類できなかった」であって「サーバの障害」ではない。**
 *
 * # なぜこの2つだけなのか
 *
 * **`src/server/admin.rs` の `classify_update` が分けているのがこの2つだけ**だから ——
 * `UnknownCharacter` → 404 / `StatScaleBelowStoredValue` → 409 /
 * **それ以外は全部 500**（`internal`）。
 * 500 に次の一手は無いので、`unknown` のまま文言だけ出す。
 *
 * **400 も `unknown` に落とす。** `character_update` の `parse` が返す拒否は
 * **サーバが壊れている欄を名乗る**（「ペルソナ本文が長すぎます」等）ので、
 * こちらから言い換えると情報が減るだけになる。
 */
export type RepairFailureReason = "scale-below-stored" | "not-found" | "unknown";

/** 画面に出すもの。 */
export interface RepairFailure {
  reason: RepairFailureReason;
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
 * **尺度が保存済みの値より小さい**ときの一手。
 *
 * **⚠ 「先に性格値を下げてください」と書かないこと。**
 * 書くと**利用者は永久に見つからないボタンを探すことになる**
 * ——`characterDelete.ts` の ⚠ と同じ穴。
 *
 * # ⚠ **理由が変わった**（2026-08-28 / 作業単位 D-4）
 *
 * **性格値を変える画面はできた**（`#/stats/{識別子}` / `StatsPanel.svelte`）。
 * **それでも、この案内からそこへ導いてはならない。**
 *
 * **`GET /admin/characters/{handle}/stats` は `find_character` を通り**（`src/server/admin.rs`）、
 * その先の **`Store::find_character_by_handle` は全フィールドを `parse` する。**
 * `src/store/character.rs` の `update_character` の doc がそのものを書いている ——
 * **「直したい行ほど読めない。修復の対象はどれも `InvalidStoredValue` になる行であり、
 * 読んでから書く実装は修復の入口として機能しない」。**
 *
 * **つまり、この画面が相手にしている行では、性格値の画面が開かない。**
 * 案内したら**開かない扉を指すことになる。**
 *
 * **⚠ 「画面ができたら足せる」と読まないこと。できても足せない。**
 * **足せる条件は「読めない行でも開ける読み出しがサーバにできたとき」**であり、
 * **それは画面側の作業単位ではない。**
 *
 * **この画面で実際に取れるのは「もっと大きい数を入れて押し直す」だけ**なので、
 * それだけを言い、**無いものは無いと言う。**
 */
const SCALE_BELOW_STORED_GUIDANCE =
  "入力した尺度より大きい性格ステータスの値が、この行に保存されています。" +
  "もっと大きい尺度を入れて、もう一度保存してください。" +
  "（性格ステータスの値そのものを変える入口は、この画面にはありません。）";

/**
 * **その識別子が居ない**ときの一手。
 *
 * 対処が正反対（`classify_delete` の doc と同じ話）——こちらは**取り直し**で通る。
 * 実在するのは一覧の**「再読み込み」**。
 */
const NOT_FOUND_GUIDANCE =
  "すでに直っている（あるいは消えている）のかもしれません。" +
  "「再読み込み」で一覧を取り直してください。";

/**
 * 失敗を、画面に出せる形へ。
 *
 * **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約:
 * エラーに内部の詳細を載せない）。
 */
export function describeRepairFailure(failure: unknown): RepairFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { reason: "unknown", message: "予期しない失敗です", guidance: null };
  }
  switch (failure.code) {
    case SCALE_BELOW_STORED_CODE:
      return {
        reason: "scale-below-stored",
        message: failure.message,
        guidance: SCALE_BELOW_STORED_GUIDANCE,
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
