<script lang="ts">
  /**
   * **会話の画面の、約束・決まりの提案の問いかけ**（作業単位 A-12c-2 / ADR-0051 §決定2 の決め直し）。
   *
   * 裏方が会話から見つけた提案（提案中）を、「今こんな約束した？ [採る] [直して採る] [違う]」の形で
   * 小さく問いかける。**外れた提案は「違う」を1回押せば済む**（ADR-0051 の線「タイミングよく出て、うるさくない」）。
   *
   * # 本文の邪魔をしない
   *
   * * **入力・送信を塞がない**（この部品は会話の欄に何も渡さない。送信中でも押せる）。
   * * **告げる囲み（`aria-live`）を持たない** —— 会話の本文の告げる囲みは1つだけ（`tests/live-region.test.ts`）。
   * * **1件ずつ見せる。** 何件あるかは「（提案 1 / 3 件）」で言い、「次へ」で回す。
   * * **提案が無ければ何も出さない**（失敗・報せがあるときだけ、それを出す）。
   *
   * # いつ取りに行くか（`GET .../agreements`）
   *
   * 提案は**本文の応答を返し切った後に、裏方の鎖（抽出の後ろ）で作られる**。その往復で見つけ、
   * **次の往復の後に確かめて**から提案中になるので、生まれた往復から1〜2往復遅れて出る
   * （1往復あたり約 10 秒 / ADR-0051 §軽い形の模索の結果）。
   *
   * | 時機 | 何回 |
   * |---|---|
   * | 開いたとき（相手を変えたときも —— 部品ごと作り直される） | 1回 |
   * | **保存されたターンが終わったとき**（`turnsEnded` が増えたとき） | すぐ1回 ＋ `recheckIntervalMs` ごとに `rechecks` 回 |
   *
   * * **ターンの直後だけでは足りない** —— その時点では、その往復の裏方はまだ走り始めたばかり。
   *   読み直さないと、次のターンを送るまで出ない（さらに1往復遅れる）。
   * * **間隔を空けた読み直しは、ターンの後の決まった回数だけ**にした。**ターンが無ければ、提案は増えない**
   *   （裏方はターンが起こす）ので、開いているだけで口を叩き続ける理由が無い。
   *   外のクライアントが同じキャラクターと話した分は、次のターンの後か、開き直したときに出る。
   * * **値（30 秒 × 3 回）は実測の線ではない**（ADR-0036）。外しても「出るのが遅れる」か
   *   「少し多く読む」だけで、何も壊れない。裏方が混んで 90 秒より遅れても、次のターンの後に出る。
   * * **画面を離れたら止める**（予約を消し、飛んでいる読みを落とす）。
   *
   * # 上限
   *
   * **上限は応答の `active_limit`**（写さない）。達していると分かっているなら、採る2つを止めて
   * 「古いものを終わらせてください」と言い、約束・決まりの画面へ案内する。**「違う」は押せる。**
   * 古い一覧で押して 409 `agreement_limit_reached` が返ったときも、同じことを言う（黙って失敗しない）。
   */

  import { untrack } from "svelte";

  import {
    adoptAgreement,
    listAgreements,
    rejectAgreement,
    type AdoptAgreementRequest,
    type AgreementView,
    type AgreementsView,
  } from "./api";
  import {
    PROPOSAL_ADOPTED_NOTICE,
    PROPOSAL_QUESTION,
    PROPOSAL_REJECTED_NOTICE,
    agreementKeeperLabel,
    describeProposalFailure,
    proposalKindNote,
    proposalLimitNotice,
    proposalPosition,
    proposedAgreements,
    type AgreementFailure,
  } from "./agreements";
  import ProposalEditor from "./ProposalEditor.svelte";
  import { hrefFor, labelFor } from "./router";

  interface Props {
    /** どのキャラクターの提案か。**変えるときは部品ごと作り直す**（`ChatView.svelte` の `{#key}`）。 */
    handle: string;
    /**
     * **この画面で保存まで届いたターンの数**（`ChatView.svelte` が数える）。増えたら取りに行く。
     * **数そのものに意味は無い**（増えたことだけを見る）。
     */
    turnsEnded: number;
    /** ターンの後の読み直しの間隔（ミリ秒）。上の doc の表。テストは短くして回す。 */
    recheckIntervalMs: number;
    /** ターンの後の読み直しの回数。 */
    rechecks: number;
  }

  let { handle, turnsEnded, recheckIntervalMs, rechecks }: Props = $props();

  /** **この部品が生きている間**の中断ハンドル。破棄されたら落とす。 */
  const lifetime = new AbortController();
  /** いちばん新しい読みの番号（**古い読みが新しい読みを上書きしない**ように）。描画には使わない。 */
  let ticket = 0;

  /** 最後に読めた一覧。**読めていなければ `null`**（何も出さない）。 */
  let view = $state<AgreementsView | null>(null);
  /** 読めなかった理由（**「読めなかった」を「提案が無い」に見せない**）。 */
  let loadError = $state<string | null>(null);
  /** いま見せている提案（`null` なら先頭）。**位置でなく id で持つ**（読み直しで並びが変わっても、同じものを見せ続ける）。 */
  let focusId = $state<number | null>(null);
  /** 直している提案の id（`null` なら直していない）。 */
  let editingId = $state<number | null>(null);
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let failure = $state<AgreementFailure | null>(null);

  const proposals = $derived(view === null ? [] : proposedAgreements(view));
  const currentIndex = $derived.by(() => {
    const at = proposals.findIndex((proposal) => proposal.id === focusId);
    return at < 0 ? 0 : at;
  });
  const current = $derived<AgreementView | undefined>(proposals[currentIndex]);
  const full = $derived(view === null ? null : proposalLimitNotice(view));

  function messageOf(error: unknown): string {
    return describeProposalFailure(error).message;
  }

  /** 一覧を読む。**画面を離れた後・古い読みの結果は書かない。** */
  async function refresh(): Promise<void> {
    ticket += 1;
    const mine = ticket;
    try {
      const next = await listAgreements(handle, lifetime.signal);
      if (lifetime.signal.aborted || mine !== ticket) {
        return;
      }
      // **形の崩れた応答で会話の画面ごと止めない**（`api.ts` は返り値の形を検証しない）。
      // この部品は会話の脇にある —— `agreements` が配列でなければ、`$derived` の中の `filter` が投げて、
      // 本文の画面まで巻き込む。**読めなかったとして扱う**（前に読めた一覧は捨てない）。
      if (!Array.isArray(next.agreements)) {
        loadError = "応答の形が想定と違いました";
        return;
      }
      view = next;
      loadError = null;
    } catch (error) {
      if (lifetime.signal.aborted || mine !== ticket) {
        return;
      }
      loadError = messageOf(error);
    }
  }

  // **開いたとき1回。**
  void refresh();

  /**
   * **ターンが終わったら、すぐ1回と、間隔を空けて決まった回数だけ読む**（上の doc の表）。
   *
   * **開いた時点の数は「もう見た」とする** —— 相手を変えて作り直された部品が、前の相手のターンの数で
   * 読み直しを始めないように。**新しいターンが来たら予約を消して数え直す**（重ねない）。
   */
  let seenTurns = untrack(() => turnsEnded);
  $effect(() => {
    const ended = turnsEnded;
    if (ended === seenTurns) {
      return;
    }
    seenTurns = ended;
    // **報せも失敗も居座らせない**（前のターンで採った・捨てた話は、もう古い）。
    // 失敗を残すと、提案が0件に戻った後も失敗の段落だけが問いかけの囲みを出し続ける。
    untrack(() => {
      notice = null;
      failure = null;
      void refresh();
    });
    let left = rechecks;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const again = (): void => {
      if (left <= 0) {
        return;
      }
      timer = setTimeout(() => {
        left -= 1;
        void refresh();
        again();
      }, recheckIntervalMs);
    };
    again();
    return () => {
      if (timer !== null) {
        clearTimeout(timer);
      }
    };
  });

  // **画面を離れたら、飛んでいる読み・送りを落とす。**
  $effect(() => {
    return () => {
      lifetime.abort();
    };
  });

  /** 次の提案へ（最後の次は最初）。直している途中なら閉じる。 */
  function next(): void {
    if (proposals.length === 0) {
      return;
    }
    const following = proposals[(currentIndex + 1) % proposals.length];
    focusId = following?.id ?? null;
    editingId = null;
  }

  /**
   * 採る（`body` が `{}` ならそのまま）。**成否にかかわらず読み直す**（失敗の一手が「取り直しました」と言う）。
   *
   * **読み直しが返るまで押させない**（`busy` を降ろすのは読み直しの後）—— 古い一覧の上で、
   * もう採った提案をもう一度押させない（押すと 409 `agreement_not_proposed` になる）。
   */
  async function adopt(proposal: AgreementView, body: AdoptAgreementRequest): Promise<void> {
    busy = true;
    notice = null;
    failure = null;
    try {
      await adoptAgreement(handle, proposal.id, body, lifetime.signal);
      notice = PROPOSAL_ADOPTED_NOTICE;
      editingId = null;
    } catch (error) {
      if (lifetime.signal.aborted) {
        return;
      }
      failure = describeProposalFailure(error);
    }
    try {
      await refresh();
    } finally {
      busy = false;
    }
  }

  /** 違う（捨てる）。**読み直しが返るまで押させない**（`adopt` と同じ）。 */
  async function reject(proposal: AgreementView): Promise<void> {
    busy = true;
    notice = null;
    failure = null;
    try {
      await rejectAgreement(handle, proposal.id, lifetime.signal);
      notice = PROPOSAL_REJECTED_NOTICE;
      editingId = null;
    } catch (error) {
      if (lifetime.signal.aborted) {
        return;
      }
      failure = describeProposalFailure(error);
    }
    try {
      await refresh();
    } finally {
      busy = false;
    }
  }
</script>

{#if current !== undefined || notice !== null || failure !== null || loadError !== null}
  <!--
    **本文の邪魔をしない**（上の doc）。**送信の欄の後ろに置く** —— 送信の欄より前に `<select>` や
    `<form>` を置くと、会話の画面の測定が引く「最初の欄」が入れ替わる（この部品は `<form>` を持たない）。
  -->
  <aside class="proposal" aria-label="約束・決まりの提案" data-proposal-prompt="1">
    {#if current !== undefined}
      <p class="proposal-heading">
        <strong class="proposal-question">{PROPOSAL_QUESTION}</strong>
        <span class="muted proposal-position">{proposalPosition(currentIndex, proposals.length)}</span>
      </p>
      <p class="proposal-content">{current.content}</p>
      <p class="note proposal-note">{agreementKeeperLabel(current.keeper)}。{proposalKindNote(current)}</p>
      {#if full !== null}
        <!--
          **採れない理由と次の一手を、押す前に言う**（送らせてから 409 で断らない）。
          **割り込まない**（`role="status"`）—— 状態の案内であって、いま起きた失敗ではない。
          0件から提案が現れるたびに、本文の読み上げへ割り込ませない。
        -->
        <p class="note" role="status" data-proposal-full="1">
          {full}<a href={hrefFor({ route: "agreements", handle })}
            >{labelFor({ route: "agreements", handle })}の画面</a
          >で終わらせられます。
        </p>
      {/if}
      <div class="actions proposal-actions">
        <button
          type="button"
          disabled={busy || full !== null}
          onclick={() => void adopt(current, {})}
        >
          採る
        </button>
        <button
          type="button"
          disabled={busy || full !== null}
          onclick={() => (editingId = current.id)}
        >
          直して採る
        </button>
        <button type="button" disabled={busy} onclick={() => void reject(current)}>違う</button>
        {#if proposals.length > 1}
          <button type="button" disabled={busy} onclick={next}>次へ</button>
        {/if}
      </div>
      {#if editingId === current.id}
        {#key current.id}
          <ProposalEditor
            proposal={current}
            {busy}
            blocked={full !== null}
            onAdopt={(body) => void adopt(current, body)}
            onCancel={() => (editingId = null)}
          />
        {/key}
      {/if}
    {/if}

    {#if notice !== null}
      <p class="notice" role="status">{notice}</p>
    {/if}
    {#if failure !== null}
      <!-- **サーバが言ったことを、そのまま出す**（こちらから足さない）。一手は下の段落。 -->
      <p class="error" role="alert" data-proposal-reason={failure.reason}>{failure.message}</p>
      {#if failure.guidance !== null}
        <p class="guidance" data-proposal-guidance="1">
          {failure.guidance}{#if failure.reason === "limit"}<a
              href={hrefFor({ route: "agreements", handle })}
              >{labelFor({ route: "agreements", handle })}の画面</a
            >で終わらせられます。{/if}
        </p>
      {/if}
    {/if}
    {#if loadError !== null}
      <!-- **「読めなかった」を「提案が無い」に見せない。** 次のターンの後に読み直す。 -->
      <p class="error" role="alert" data-proposal-load-error="1">
        約束・決まりの提案を読めませんでした（次のターンの後に読み直します）: {loadError}
      </p>
    {/if}
  </aside>
{/if}

<style>
  /* **枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /*
    **小さく、本文と分けて**置く。**色は使わない**（accent は「測っている」か「壊れている」に読める）。
    罫で囲むだけ。
  */
  .proposal {
    border: 1px solid var(--color-rule);
    padding: 0.4rem 0.6rem;
    margin: 0.6rem 0;
    font-size: 0.9rem;
  }

  .proposal p {
    margin: 0.2rem 0;
  }

  .proposal-position {
    margin-left: 0.5rem;
  }

  .proposal-content {
    white-space: pre-wrap;
  }
</style>
