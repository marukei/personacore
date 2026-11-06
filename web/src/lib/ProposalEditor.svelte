<script lang="ts">
  /**
   * **提案を直して採る欄**（作業単位 A-12c-2 / ADR-0051 §決定2「直してから採る」）。
   *
   * **会話の画面の問いかけ（`ProposalPrompt.svelte`）と、約束・決まりの画面（`AgreementsPanel.svelte`）の
   * 両方が使う。** 写しを2つ持つと、片方だけが検査を変えたときに、同じ提案が画面ごとに違う形で採られる。
   *
   * # 何を直せるか
   *
   * 中身・種類・守る側（`POST .../adoption` が受ける3つ）。**中身と守る側は提案から写し、
   * 種類は選んでいない状態で始める**（`agreements.ts` の `adoptDraftOf` —— 提案の種類は判定していない）。
   *
   * # 送るのは呼び出し側
   *
   * この部品は `fetch` しない。組み立てた本文を `onAdopt` に渡すだけ（失敗の出し方は画面ごとに違う）。
   *
   * **⚠ `<form>` にしない** —— 会話の画面の測定は「画面の最初の `<form>`」を送信の欄として引く。
   * 欄の名前（`proposal-…`）も、約束・決まりの画面の書く欄（`content` / `kind` / `keeper`）と分けてある。
   */

  import { untrack } from "svelte";

  import type { AdoptAgreementRequest, AgreementView } from "./api";
  import {
    AGREEMENT_CONTENT_MAX_CHARS,
    AGREEMENT_KEEPERS,
    AGREEMENT_KINDS,
    adoptDraftOf,
    agreementCounter,
    agreementKeeperLabel,
    agreementKindLabel,
    buildAdoptAgreement,
    isOverAgreementLimit,
    type AgreementDraft,
  } from "./agreements";

  interface Props {
    /** 直す提案。 */
    proposal: AgreementView;
    /** 送っている間は押させない。 */
    busy: boolean;
    /**
     * **上限に達していて採れない**（作業単位 A-12c-2 のレビュー）。欄を開いた後に上限へ達したときも、
     * 「この内容で採る」を止める（行の「採る」と同じ）。**欄は閉じない**（打った字を消さない）。
     */
    blocked: boolean;
    /** 組み立てた本文を渡す（送るのは呼び出し側）。 */
    onAdopt: (body: AdoptAgreementRequest) => void;
    /** 「やめる」。 */
    onCancel: () => void;
  }

  let { proposal, busy, blocked, onAdopt, onCancel }: Props = $props();

  // **開いた時点の提案から写す**（追随させない —— 直している途中で一覧を読み直しても、打った字を消さない）。
  // 別の提案を直すときは、呼び出し側が部品ごと作り直す（`{#key}`）。
  let draft = $state<AgreementDraft>(untrack(() => adoptDraftOf(proposal)));

  const built = $derived(buildAdoptAgreement(draft));
</script>

<div class="proposal-editor" data-proposal-editor={proposal.id}>
  <label>
    中身
    <input
      name="proposal-content"
      value={draft.content}
      oninput={(event) => (draft.content = event.currentTarget.value)}
    />
  </label>
  <span class="counter" class:over={isOverAgreementLimit(draft.content)}>
    {agreementCounter(draft.content)}
  </span>
  <label>
    種類
    <select
      name="proposal-kind"
      value={draft.kind}
      onchange={(event) => (draft.kind = event.currentTarget.value)}
    >
      <!-- **既定を作らない**（提案の種類は判定していない。約束か決まりかは、採る人が決める）。 -->
      <option value="">選んでください</option>
      {#each AGREEMENT_KINDS as kind (kind)}
        <option value={kind}>{agreementKindLabel(kind)}</option>
      {/each}
    </select>
  </label>
  <label>
    守る側
    <select
      name="proposal-keeper"
      value={draft.keeper}
      onchange={(event) => (draft.keeper = event.currentTarget.value)}
    >
      <option value="">選んでください</option>
      {#each AGREEMENT_KEEPERS as keeper (keeper)}
        <option value={keeper}>{agreementKeeperLabel(keeper)}</option>
      {/each}
    </select>
  </label>
  <p class="note">
    中身は {AGREEMENT_CONTENT_MAX_CHARS} 文字まで。「誰が・何を」が分かる短い平叙文で書きます。
  </p>
  {#if !built.ok}
    <!-- **送れない理由を、開いた時点から言う**（種類は必ず選ぶので、最初は「選んでください」）。 -->
    <p class="note" data-proposal-build-error="1">{built.message}</p>
  {/if}
  <div class="actions">
    <button
      type="button"
      disabled={!built.ok || busy || blocked}
      onclick={() => {
        if (built.ok && !blocked) {
          onAdopt(built.body);
        }
      }}
    >
      この内容で採る
    </button>
    <button type="button" disabled={busy} onclick={onCancel}>やめる</button>
  </div>
</div>

<style>
  /* **表・枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  label {
    margin-right: 1rem;
  }

  .proposal-editor {
    margin: 0.4rem 0;
  }
</style>
