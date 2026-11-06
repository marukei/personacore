<script lang="ts">
  /**
   * **いまの値が読めないときに、性格ステータスを書き直す**（作業単位 D-8）。
   *
   * # `StatsPanel` の中のフォームと別の部品にしてある理由
   *
   * あちらは **`GET .../stats` の応答から下書きを作る**（`draftOfStats`）ことが
   * 前提で、**保存済みの軸名を変えられない**という約束
   * （`{#if row.stored}` の1行 / `removedAxes` がそれに寄りかかっている）まで含めて成り立っている。
   *
   * **ここにはその応答が無い。** 同じ部品に第3の状態を足すと、
   * **`draft === null` が「まだ読んでいない」と「読めなかった」の2つを意味する**ことになり、
   * **その混同こそ、この画面が直そうとしている壊れ方と同じ形**になる
   * （`CharacterRepairForm.svelte` と同じ判断 / D-6）。
   *
   * # ⚠ **送る本文の組み立ては写していない**
   *
   * `statsEdit.ts` の `buildReplace` / `keptRows` / `replaceSummary` /
   * `describeReplaceFailure` を**そのまま使う。**
   * 写すと、「全軸を送る」という意味論の見張り番が片方にしか効かなくなる。
   *
   * # ⚠ **何が消えるかは言えない**
   *
   * いま保存されている軸を1本も読み出せないので、
   * **`replaceSummary` が言えるのは「何が残るか」だけ**である。
   * **残りは `UNKNOWN_LOSS_NOTICE` が「言えない」と明言する**
   * ——できないことを、できるふりをしない（D-3c）。
   */

  import { untrack } from "svelte";

  import { replaceStats, type StatsView } from "./api";
  import {
    buildReplace,
    describeReplaceFailure,
    emptyRow,
    replaceSummary,
    type ReplaceFailure,
    type StatsDraft,
  } from "./statsEdit";
  import {
    UNKNOWN_LOSS_NOTICE,
    emptyRepairDraft,
    type RepairScale,
  } from "./statsRepair";

  interface Props {
    /** 送り先。**この画面が見ているキャラクター。** */
    handle: string;
    /**
     * 値の上端（`statsRepair.ts` の `RepairScale`）。
     *
     * **既定値で埋めないこと**（`StatScale::DEFAULT` を写さない）。
     * 埋めると、**別の尺度のキャラクターに他人の上端を見せる**ことになる。
     *
     * **⚠ `number | null` ではない。** 読めなかった回は**理由の文を連れてくる**ので、
     * **「上端は分かっているのに理由も出ている」という状態が作れない**
     * （`RepairScale` の doc）。
     */
    scale: RepairScale;
    /** 保存が通ったとき。**親がいまの値を読み直す。** */
    onRepaired?: (stats: StatsView) => void;
  }

  let { handle, scale, onRepaired }: Props = $props();

  /**
   * 下書き。**空の行が1本から始まる**（推測で埋めない / `emptyRepairDraft`）。
   *
   * **⚠ 後から `handle` が変わっても追随しない**（`untrack` はそれを明示する）。
   * 別のキャラクターを直させたい親は `{#key}` で作り直すこと
   * ——追随させると、打ちかけが親の再描画で黙って消える。
   */
  let draft = $state<StatsDraft>(untrack(() => emptyRepairDraft()));

  let saving = $state(false);
  let failure = $state<ReplaceFailure | null>(null);

  /** 送る本文。**組み立ては純粋関数**（`statsEdit.ts` の `buildReplace`）。 */
  const built = $derived(buildReplace(draft));
  const canSubmit = $derived(built.ok && !saving);
  /** **何が残るか。** 何が消えるかは言えない（上の ⚠）。 */
  const summary = $derived(replaceSummary(draft));

  function addRow(): void {
    draft.rows.push(emptyRow());
  }

  /**
   * その行を「外す」／「戻す」。
   *
   * **押しただけでは何も起きない。** 外した行は本文に載らなくなるだけで、
   * 送るのは保存のときである（`keptRows`）。
   */
  function toggleRemoved(index: number): void {
    const row = draft.rows[index];
    if (row === undefined) {
      return;
    }
    row.removed = !row.removed;
  }

  /** 軸名の欄。**`bind:` を使わない** —— 索引の型を絞る場所を1つに保つ。 */
  function setAxis(index: number, axis: string): void {
    const row = draft.rows[index];
    if (row === undefined) {
      return;
    }
    row.axis = axis;
  }

  /** 値の欄。 */
  function setValue(index: number, value: string): void {
    const row = draft.rows[index];
    if (row === undefined) {
      return;
    }
    row.value = value;
  }

  async function save(): Promise<void> {
    const request = built;
    if (!canSubmit || !request.ok) {
      return;
    }
    saving = true;
    failure = null;
    try {
      // **送り先は、この画面が見ているキャラクター。**
      const stored = await replaceStats(handle, request.body);
      onRepaired?.(stored);
    } catch (error) {
      failure = describeReplaceFailure(error);
    } finally {
      saving = false;
    }
  }
</script>

<form
  data-stats-repair="1"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <h3>性格ステータスを書き直す</h3>

  <!--
    **代償を隠さない。** いま保存されている軸は読み出せないので、
    **何が消えるかは押す前に言えない**（`UNKNOWN_LOSS_NOTICE`）。
  -->
  <p class="hint" data-unknown-loss="1">{UNKNOWN_LOSS_NOTICE}</p>

  {#if scale.kind === "unreadable"}
    <!--
      **知らないことを、知らないと言う**（既定値で埋めない）。
      **文は `scale` が連れてくる** —— 「引けなかった」と「読めない形だった」で
      別の文になる（`statsRepair.ts` の `RepairScale`）。
      **ここで文を選ばない。** 選ぶと、判定の写しが2つになる。
    -->
    <p class="hint" data-repair-scale="unknown">{scale.notice}</p>
  {:else}
    <!--
      **尺度は一覧から引いた、このキャラクターのもの**（`scaleFromListing`）。
      **`GET .../drift` から取らないこと** —— そちらも一緒に落ちている。
    -->
    <p class="hint" data-repair-scale="known">
      このキャラクターの尺度は <strong>{scale.scale}</strong> です（値の上端）。
    </p>
  {/if}

  <table>
    <thead>
      <tr>
        <th>軸</th>
        <th>値</th>
        <th></th>
      </tr>
    </thead>
    <tbody data-section="stats-repair">
      <!--
        **索引で鍵を付ける。** 軸名は空にも重複にもなりうるので、
        軸名を鍵にすると**行が入れ替わる**（帰属の壊れ方）。
      -->
      {#each draft.rows as row, index (index)}
        <tr
          data-repair-row={index}
          data-removed={row.removed ? "true" : "false"}
          class:removed={row.removed}
        >
          <td>
            <input
              name="axis"
              value={row.axis}
              disabled={saving || row.removed}
              oninput={(event) => setAxis(index, event.currentTarget.value)}
            />
          </td>
          <td>
            <input
              name="value"
              value={row.value}
              disabled={saving || row.removed}
              oninput={(event) => setValue(index, event.currentTarget.value)}
            />
            {#if scale.kind === "known"}
              <span class="muted">/ {scale.scale}</span>
            {/if}
          </td>
          <td>
            <button
              type="button"
              disabled={saving}
              onclick={() => toggleRemoved(index)}
            >
              {row.removed ? "戻す" : "外す"}
            </button>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>

  <div class="actions">
    <button type="button" onclick={addRow} disabled={saving}>軸を1本足す</button>
  </div>

  <!-- **何が残るかは言える**（`replaceSummary`）。 -->
  <p class="summary" data-repair-summary="1">{summary}</p>

  {#if !built.ok}
    <!-- **送る前に止めた理由**（サーバの拒否とは別物なので、別の印を付ける）。 -->
    <p class="error" role="alert" data-repair-blocked="1">{built.message}</p>
  {/if}

  {#if failure !== null}
    <!-- **サーバが言ったことを、そのまま出す**（こちらから足さない）。 -->
    <p class="error" role="alert" data-repair-reason={failure.reason}>{failure.message}</p>
    {#if failure.guidance !== null}
      <!-- **次に取れる行動。** 分類できなかった失敗には出ない（**推測で埋めない**）。 -->
      <p class="guidance" data-repair-guidance="1">{failure.guidance}</p>
    {/if}
  {/if}

  <div class="actions">
    <button type="submit" disabled={!canSubmit}>この内容で作り直す</button>
  </div>
</form>

<style>
  /* **表・枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /* **この枠は「壊れた行を直す」ためのもの。** 枠が壊れている側の色を持つ。 */
  form {
    border-color: var(--color-warn);
  }

  .hint {
    margin: 0 0 0.4rem;
  }
</style>
