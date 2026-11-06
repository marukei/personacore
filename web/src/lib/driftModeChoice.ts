/**
 * **変容の走らせ方の選択**（作業単位 D-15 / ADR-0045 §決定3）。
 *
 * **I/O を持たない。** 作成・編集（`characterEdit.ts` / `CharacterForm.svelte`）と
 * 修復（`characterRepair.ts` / `CharacterRepairForm.svelte`）が**同じ4択**を使う。
 * **写しを2つ持たないためにここへ置いた**（片方だけ文言や並びが古びる）。
 *
 * # 4つの選択と、送る値
 *
 * | 表示 | 下書きの値 | 送る値 |
 * |---|---|---|
 * | 既定に従う（いまは: …） | `""` | **作成: キーごと省く / 編集・修復: `null`** |
 * | 止める | `off` | `"off"` |
 * | 記録だけ（値は動かさない） | `shadow` | `"shadow"` |
 * | 動かす | `on` | `"on"` |
 *
 * **下書きは文字列**（`<select>` の値そのもの / 他の欄と同じ「下書きは全部が文字列」）。
 * **`""` が「既定に従う」**で、送る直前に `null` へ直す（`driftModeWireOf`）。
 *
 * # 綴りは写さない
 *
 * `off` / `shadow` / `on` は `statsEdit.ts` の `RUN_MODE_*` から取る
 * （**写し元 `src/domain/drift.rs` の `StatDriftMode::label` を読む見張り番**が
 * `tests/stats-edit.test.ts` にある）。「既定に従う」の括弧の表示は `runModeLabel`。
 *
 * # 「いまの既定」は画面が知らないことがある
 *
 * 既定は変容の応答（`GET .../drift` の `default_run_mode`）からしか分からない。
 * **新規作成ではキャラクターがまだ無いので引けない**し、読み込み中や読めなかった回もある。
 * **そのときは括弧を出さない**（推測で「停止」と書かない）。
 */

import { RUN_MODE_OFF, RUN_MODE_ON, RUN_MODE_SHADOW, runModeLabel } from "./statsEdit";

/** 欄の見出し。**作成・編集・修復・修復の列名の見出しで同じ綴り。** */
export const DRIFT_MODE_FIELD_LABEL = "変容の走らせ方";

/** 下書きで「既定に従う」を表す値（送るときは `null` / 作成ではキーごと省く）。 */
export const FOLLOW_DEFAULT = "";

/** 選択肢1つ。 */
export interface DriftModeChoice {
  /** 下書きの値（`<option value>`）。 */
  value: string;
  /** 表示。 */
  label: string;
}

/**
 * 「既定に従う」の表示。**括弧の中は、渡された既定から言う。**
 *
 * `defaultRunMode` は**変容の応答の `default_run_mode`**（起動時の既定）。
 * **キャラクターの値や `run_mode`（効いている値）を渡さないこと** ——
 * キャラクターが値を持っていると `run_mode` はその値になり、
 * **「既定に従うと、いまは何になるか」を言えなくなる。**
 */
export function followDefaultLabel(defaultRunMode: string | null): string {
  if (defaultRunMode === null) {
    return "既定に従う";
  }
  return `既定に従う（いまは: ${runModeLabel(defaultRunMode)}）`;
}

/** 4つの選択肢（**この順で並べる**）。 */
export function driftModeChoices(defaultRunMode: string | null): DriftModeChoice[] {
  return [
    { value: FOLLOW_DEFAULT, label: followDefaultLabel(defaultRunMode) },
    { value: RUN_MODE_OFF, label: "止める" },
    { value: RUN_MODE_SHADOW, label: "記録だけ（値は動かさない）" },
    { value: RUN_MODE_ON, label: "動かす" },
  ];
}

/**
 * 保存されている値を下書きへ。**未設定（`null`）は「既定に従う」。**
 *
 * **知らない綴りは、そのまま持つ。** 4つのどれかに丸めると、
 * **触っていないのに差分として本文に載り、サーバの値を踏み潰す。**
 */
export function driftModeDraftOf(stored: string | null): string {
  return stored ?? FOLLOW_DEFAULT;
}

/** 下書きを送る値へ。**「既定に従う」は `null`。** */
export function driftModeWireOf(draft: string): string | null {
  return draft === FOLLOW_DEFAULT ? null : draft;
}

/** その値が4つの選択肢のどれかか（知らない綴りを画面に出すかの判定）。 */
export function isKnownDriftModeChoice(value: string): boolean {
  return driftModeChoices(null).some((choice) => choice.value === value);
}
