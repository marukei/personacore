<script lang="ts">
  /**
   * **壊れた行を書き直す**（作業単位 D-6 / ADR-0016 §3・§4）。
   *
   * # `CharacterForm.svelte` と別の部品にしてある理由
   *
   * あちらは**作成（`POST`）と編集（`PATCH` の差分）**の2つを、
   * **`character` が `null` かどうか**だけで切り替えている。
   * **修復はその軸に乗らない** —— 「元の行はある。しかし値が1つも読めない」ので、
   * 同じ部品に第3の状態を足すと、**`current === null` が
   * 「まだ作っていない」と「読めない」の2つを意味する**ことになる。
   * **その混同こそが、この画面が直そうとしている壊れ方と同じ形。**
   *
   * # ⚠ **壊れていない列も、ここに書いた値で上書きされる**
   *
   * 元の値は API から返ってこない（`UnreadableCharacterView` は値を持たない）ので、
   * **確かめようが無い。** 承知のうえで全部を書き直す（`characterRepair.ts` の doc）。
   * **どの列が実際に壊れていたかは `invalid_columns` として出す。**
   *
   * # 推測で埋めない
   *
   * 初期値は空（`emptyRepairDraft`）。**`MemoryList` の
   * 「読み出せなかった行」の訂正と同じ立場**で、
   * 推測した値が「いま正しい設定」として保存されるのを避ける。
   */

  import { untrack } from "svelte";

  import { updateCharacter, type CharacterView } from "./api";
  import {
    LIMITS,
    countChars,
    isOverLimit,
    labelOf,
  } from "./characterEdit";
  import {
    buildRepair,
    columnLabel,
    describeRepairFailure,
    emptyRepairDraft,
    repairDriftModeChoices,
    type RepairColumn,
    type RepairDraft,
    type RepairFailure,
    type RepairLimitedField,
    type RepairableRow,
  } from "./characterRepair";
  import { DRIFT_MODE_FIELD_LABEL } from "./driftModeChoice";

  interface Props {
    /** 直す行。**`handle` が読めた行しか渡せない**（`repairTargetOf`）。 */
    row: RepairableRow;
    /**
     * 保存が通ったとき。**読み直せなかった回も呼ぶ。**
     *
     * 読み直せていなくても**更新は起きている**ので、一覧は取り直す必要がある
     * （`unreadable` の側が変わっているかもしれない）。
     * **`CharacterForm` の `onSaved` と意味が違う**ので、名前も分けてある。
     */
    onRepaired?: (character: CharacterView | null) => void;
    /** 閉じる。**渡さなければ「閉じる」ボタンを出さない。** */
    onClose?: () => void;
  }

  let { row, onRepaired, onClose }: Props = $props();

  /**
   * 下書き。**初期値は空。**
   *
   * **⚠ 後から `row` が別の行に差し替わっても追随しない**（`untrack` はそれを明示する）。
   * **別の行を直させたい親は `{#key}` で作り直すこと**（`CharacterList.svelte`）
   * ——追随させると、打ちかけの本文が親の再描画で黙って消える。
   */
  let draft = $state<RepairDraft>(
    untrack(() => emptyRepairDraft(broken("stat_drift_mode"))),
  );

  /**
   * **走らせ方の選択肢**（作業単位 D-15）。**列が壊れていない行では「変えない」から始まる**
   * （`characterRepair.ts` の `RepairDraft.stat_drift_mode` の doc）。
   *
   * **「既定に従う」に括弧は付けない** ——
   * この行は読めないので、変容の応答（既定を言う唯一の口）も開かない
   * （`GET .../drift` は `find_character` を通り、壊れた行では失敗する）。
   */
  const driftChoices = untrack(() => repairDriftModeChoices(broken("stat_drift_mode")));

  let saving = $state(false);
  let failure = $state<RepairFailure | null>(null);
  let notice = $state<string | null>(null);

  /** 送る本文。**組み立ては純粋関数**（`characterRepair.ts`）。 */
  const built = $derived(buildRepair(draft));

  const canSubmit = $derived(built.ok && !saving);

  /**
   * **その列が、この行で読めなかったか。**
   *
   * **`row.invalid_columns` を引く。** 決め打つと、
   * **どの欄を直せばよいかを画面が嘘で言う**ことになる。
   * 見張り番は `tests/character-repair-form.test.ts::壊れていた欄にだけ印が付く`
   * （**4列とも、印が付く腕と付かない腕の両方を通る**）。
   *
   * # ⚠ 引数を `string` にしないこと
   *
   * **`string` だと、テンプレートに書いた綴りが
   * `REPAIR_COLUMNS`（写し元を読む見張り番つき）から独立した第2の写しになる。**
   * **D-6 のレビューで実測**: `broken("output_format_instruction")` を
   * `broken("output_format")` に変えても **262 全緑・`pnpm check` も 0 エラー**だった
   * ——**このファイルの `REPAIR_COLUMNS` の doc が書いている状況そのもの。**
   *
   * **`RepairColumn` にすると、綴りを崩した時点でコンパイルが通らなくなる。**
   */
  function broken(column: RepairColumn): boolean {
    return row.invalid_columns.includes(column);
  }

  /** **その欄が、その欄の上限を超えているか**（`CharacterForm` と同じ形）。 */
  function over(field: RepairLimitedField): boolean {
    return isOverLimit(field, draft[field]);
  }

  /** いま何文字か。**サーバと同じ数え方**（`countChars`）。 */
  function counterOf(field: RepairLimitedField): string {
    return `${countChars(draft[field])} / ${LIMITS[field]}`;
  }

  async function save(): Promise<void> {
    const request = built;
    if (!canSubmit || !request.ok) {
      return;
    }
    saving = true;
    failure = null;
    notice = null;

    try {
      // **送り先は、この行の識別子。** 一覧の行と送り先がここで結び付く。
      const updated = await updateCharacter(row.handle, request.body);
      if (updated.character === null) {
        // **失敗ではない**（`admin.rs` の `UpdatedCharacterView`）。
        // 更新は通っていて、読み直せなかっただけ。**下書きは触らない**
        // ——送った値がそのまま残るので、続けて直せる。
        notice =
          "更新しました。ただし更新後の姿を読み直せていません（「再読み込み」で一覧を取り直してください）。";
      } else {
        notice = `修復しました。${updated.character.handle} は読み出せるようになりました。`;
      }
      onRepaired?.(updated.character);
    } catch (error) {
      failure = describeRepairFailure(error);
    } finally {
      saving = false;
    }
  }
</script>

<form
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <h3 data-repair-row={row.id}>
    id {row.id}（<code>{row.handle}</code>）を修復
  </h3>

  <!--
    **生の列名を落とさない**（DB を直接見に行くときの手掛かり）。
    **見出しも併記する** —— 知らない綴りは `columnLabel` がそのまま返すので、
    サーバが列を増やしても画面から消えない。
  -->
  <p class="hint" data-invalid-columns={row.invalid_columns.join(" ")}>
    読めなかった列:
    {#each row.invalid_columns as column, index (column)}
      {#if index > 0},&nbsp;{/if}<code>{column}</code>（{columnLabel(column)}）
    {/each}
  </p>

  <!--
    **代償を隠さない。** 壊れていない列も、ここに書いた値で上書きされる
    （元の値を読み出す手段が無いので、確かめようが無い）。

    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
    **識別子が不変であることの決定は ADR-0016 §1**——
    **その番号は、ここ（コメント）と `characterRepair.ts` にだけ残す。**
    **画面は利用者に向けて書く**（見張り番は `tests/screen-wording.test.ts` と
    `tests/character-repair-form.test.ts` の「画面が利用者の言葉で話す」）。
  -->
  <p class="hint">
    この行は値を1つも読み出せません。<strong>推測で埋めないでください。</strong>
    保存すると、<strong>壊れていなかった列もここに書いた値で上書きされます</strong>
    （変容の走らせ方だけは、壊れていなければ変えずに残せます）。
    識別子（<code>{row.handle}</code>）は変えられません。
  </p>

  <label>
    <span>{labelOf("display_name")}</span>
    <input
      name="display_name"
      bind:value={draft.display_name}
      disabled={saving}
    />
    <span
      class="counter"
      data-counter="display_name"
      class:over={over("display_name")}
    >
      {counterOf("display_name")}
    </span>
  </label>
  {#if broken("display_name")}
    <p class="broken" data-broken-field="display_name">この列が読めませんでした。</p>
  {/if}

  <label>
    <span>{labelOf("base_persona")}</span>
    <textarea
      name="base_persona"
      rows="6"
      bind:value={draft.base_persona}
      disabled={saving}
    ></textarea>
    <span
      class="counter"
      data-counter="base_persona"
      class:over={over("base_persona")}
    >
      {counterOf("base_persona")}
    </span>
  </label>
  {#if broken("base_persona")}
    <p class="broken" data-broken-field="base_persona">この列が読めませんでした。</p>
  {/if}

  <label>
    <span>{labelOf("output_format_instruction")}</span>
    <textarea
      name="output_format_instruction"
      rows="3"
      bind:value={draft.output_format_instruction}
      disabled={saving}
    ></textarea>
    <span
      class="counter"
      data-counter="output_format_instruction"
      class:over={over("output_format_instruction")}
    >
      {counterOf("output_format_instruction")}
    </span>
  </label>
  {#if broken("output_format_instruction")}
    <p class="broken" data-broken-field="output_format_instruction">
      この列が読めませんでした。
    </p>
  {/if}
  <p class="hint">空のままなら未設定に戻します（この節はプロンプトから落ちます）。</p>

  <label>
    <span>性格ステータスの尺度</span>
    <!--
      **上下限を書かない**（`StatScale::MIN` / `MAX` を写さない）。
      範囲外はサーバが理由付きで返す。
    -->
    <input name="stat_scale" bind:value={draft.stat_scale} disabled={saving} />
  </label>
  {#if broken("stat_scale")}
    <p class="broken" data-broken-field="stat_scale">この列が読めませんでした。</p>
  {/if}
  <p class="hint">
    保存されている値は読み出せないので、<strong>必ず入力してください。</strong>
  </p>

  <label>
    <span>{DRIFT_MODE_FIELD_LABEL}</span>
    <select name="stat_drift_mode" bind:value={draft.stat_drift_mode} disabled={saving}>
      {#each driftChoices as choice (choice.value)}
        <option value={choice.value}>{choice.label}</option>
      {/each}
    </select>
  </label>
  {#if broken("stat_drift_mode")}
    <p class="broken" data-broken-field="stat_drift_mode">この列が読めませんでした。</p>
  {/if}
  <!--
    **列が壊れているかで、触らなかったときの扱いが違う**（作業単位 D-15 のレビューの是正）。
    壊れていない列の値は正しいので、触らなければ送らない（サーバはその列を残す）。
  -->
  {#if broken("stat_drift_mode")}
    <p class="hint" data-drift-mode-hint="broken">
      この列は読めなかったので、選ばなければ既定に従う（サーバの起動時の設定に従う）として保存します。
    </p>
  {:else}
    <p class="hint" data-drift-mode-hint="kept">
      この列は壊れていないので、選ばなければいまの設定のまま変えません。
    </p>
  {/if}

  {#if !built.ok}
    <!-- **送る前に止めた理由**（サーバの拒否とは別物なので、別の印を付ける）。 -->
    <p class="error" role="alert" data-repair-blocked="1">{built.message}</p>
  {/if}

  {#if failure !== null}
    <!--
      **サーバが言ったことをそのまま出す**（`characterDelete.ts` と同じ立場 ——
      こちらから足さない）。**種別は `code` で決まる。**
    -->
    <p class="error" role="alert" data-repair-reason={failure.reason}>{failure.message}</p>
    {#if failure.guidance !== null}
      <!-- **次に取れる行動。** 分類できなかった失敗には出ない（**推測で埋めない**）。 -->
      <p class="guidance" data-repair-guidance="1">{failure.guidance}</p>
    {/if}
  {/if}

  {#if notice !== null}
    <p class="notice" role="status" data-repair-notice="1">{notice}</p>
  {/if}

  <div class="actions">
    <button type="submit" disabled={!canSubmit}>修復を保存する</button>
    {#if onClose !== undefined}
      <button type="button" onclick={onClose} disabled={saving}>閉じる</button>
    {/if}
  </div>
</form>

<style>
  /* **枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /* **この枠は「壊れた行を直す」ためのもの。** 枠が壊れている側の色を持つ。 */
  form {
    border-color: var(--color-warn);
  }

  .hint {
    margin: 0 0 0.4rem;
  }

  /* **どの列が読めなかったか。** 見出しの列の幅にそろえる。 */
  .broken {
    margin: 0 0 0.4rem 10.5rem;
    color: var(--color-warn);
    font-size: 0.9em;
  }
</style>
