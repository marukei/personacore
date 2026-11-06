<script lang="ts">
  /**
   * **約束・決まりの画面**（作業単位 A-12a / ADR-0051 §決定4・5）。
   *
   * # なぜこの画面が要るのか
   *
   * **有効な約束・決まりは、毎ターンのプロンプトに載る**（削れない枠の次の「優先の枠」。
   * 記憶と履歴より先に取るが、入りきらないときは古いものから落ち、その件数を一覧の口の
   * `last_turn` が伝える —— 画面はそれを出す）。
   * 会話の中で交わした約束を覚えておく置き場所で、**利用者が手で書き、手で終わらせる。**
   * **裏方が会話から見つけた提案**（提案中）も出す（作業単位 A-12c-2 / ADR-0051 §決定2）——
   * 「提案」の欄で採る・直して採る・捨てる。**提案中は本文に載らず、上限にも数えない。**
   * 捨てたもの（却下）は記録の側にだけ出る（「終わった約束・決まり」には出さない）。
   * 会話の画面にも同じ提案を問いかける（`ProposalPrompt.svelte`）。
   *
   * # 3つの読みは別々（**片方が落ちても、もう片方は出す**）
   *
   * 一覧と履歴は別の要求。**失敗も別々に持つ** —— 履歴が読めないだけで書く入口まで消さない。
   *
   * # 消す操作は無い
   *
   * **終わらせても行は残る**（一覧の「終わった約束・決まり」と履歴に残る）。
   * **消す・書き換える操作をこの画面に生やさないこと。** サーバにも経路が無い。
   *
   * # 上限
   *
   * **上限は応答の `active_limit`**（写しを持たない）。達したら書く欄を止め、
   * 古いものを終わらせるよう促す（**黙って古いものを落とさない**）。
   */

  import {
    ApiRequestError,
    adoptAgreement,
    createAgreement,
    endAgreement,
    listAgreementTransitions,
    listAgreements,
    rejectAgreement,
    type AdoptAgreementRequest,
    type AgreementEnding,
    type AgreementTransitionsView,
    type AgreementView,
    type AgreementsView,
  } from "./api";
  import {
    AGREEMENT_CONTENT_MAX_CHARS,
    AGREEMENT_ENDINGS,
    AGREEMENT_KEEPERS,
    AGREEMENT_KINDS,
    PROPOSAL_ADOPTED_NOTICE,
    PROPOSAL_KIND_UNDECIDED,
    PROPOSAL_REJECTED_NOTICE,
    activeAgreements,
    agreementCounter,
    agreementKeeperLabel,
    agreementKindLabel,
    agreementLimitNotice,
    agreementStateLabel,
    buildCreateAgreement,
    describeAgreementFailure,
    describeProposalFailure,
    droppedNotice,
    emptyAgreementDraft,
    endedAgreements,
    isAtAgreementLimit,
    isOverAgreementLimit,
    proposalLimitNotice,
    proposedAgreements,
    transitionBeforeLabel,
    transitionCountNotice,
    transitionKindLabel,
    type AgreementDraft,
    type AgreementFailure,
  } from "./agreements";
  import ProposalEditor from "./ProposalEditor.svelte";
  import { hrefFor, labelFor } from "./router";
  // **時刻の綴りは1箇所**（作業単位 F-7 / 記憶の一覧と同じ関数）。
  import { browserZone, timeLabel } from "./timestamp";

  interface Props {
    /** どのキャラクターのものか。**経路が運ぶ**（`router.ts` の `Screen`）。 */
    handle: string;
  }

  let { handle }: Props = $props();

  /** **読み手の時間帯**（`AttributesPanel` と同じ理由で、ここで解決して渡す）。 */
  const zone = browserZone();

  /** 取得の結果。**`null` を「失敗」と「まだ」に使い回さない。** */
  type Loaded<T> =
    | { kind: "loading" }
    | { kind: "ready"; value: T }
    | { kind: "failed"; message: string };

  let agreementsState = $state<Loaded<AgreementsView>>({ kind: "loading" });
  let transitionsState = $state<Loaded<AgreementTransitionsView>>({
    kind: "loading",
  });

  let draft = $state<AgreementDraft>(emptyAgreementDraft());
  let busy = $state(false);
  let notice = $state<string | null>(null);
  /** 書く・終わらせるの失敗。**どちらの操作の失敗かを持つ。** */
  let failure = $state<{ action: "write" | "end"; detail: AgreementFailure } | null>(
    null,
  );
  /**
   * **確かめている終わらせ方**（`null` なら確かめていない）。
   *
   * 終わらせると戻せない（有効に戻す口は無い）ので、**押してすぐには送らない。**
   */
  let confirming = $state<{ id: number; ending: AgreementEnding } | null>(null);
  /** **直している提案の id**（`null` なら直していない / 作業単位 A-12c-2）。 */
  let editingProposal = $state<number | null>(null);
  /**
   * 提案を採る・捨てるの失敗（作業単位 A-12c-2）。**書く・終わらせるの失敗（`failure`）と分ける** ——
   * 読み分けが違う（`describeProposalFailure`）うえ、出す場所が「提案」の欄の中。
   */
  let proposalFailure = $state<AgreementFailure | null>(null);

  const built = $derived(buildCreateAgreement(draft));
  const atLimit = $derived(
    agreementsState.kind === "ready" && isAtAgreementLimit(agreementsState.value),
  );
  const canWrite = $derived(
    agreementsState.kind === "ready" && !atLimit && built.ok && !busy,
  );

  function messageOf(error: unknown): string {
    if (error instanceof ApiRequestError) {
      return error.message;
    }
    return "予期しない失敗です";
  }

  async function load(): Promise<void> {
    agreementsState = { kind: "loading" };
    transitionsState = { kind: "loading" };
    await Promise.all([loadAgreements(), loadTransitions()]);
  }

  async function loadAgreements(): Promise<void> {
    try {
      agreementsState = { kind: "ready", value: await listAgreements(handle) };
    } catch (error) {
      agreementsState = { kind: "failed", message: messageOf(error) };
    }
  }

  async function loadTransitions(): Promise<void> {
    try {
      transitionsState = {
        kind: "ready",
        value: await listAgreementTransitions(handle),
      };
    } catch (error) {
      transitionsState = { kind: "failed", message: messageOf(error) };
    }
  }

  async function write(): Promise<void> {
    const request = built;
    if (!canWrite || !request.ok) {
      return;
    }
    busy = true;
    notice = null;
    failure = null;
    try {
      await createAgreement(handle, request.body);
      notice = "約束・決まりを書きました。次のターンから本文に載ります。";
      draft = emptyAgreementDraft();
      await load();
    } catch (error) {
      failure = { action: "write", detail: describeAgreementFailure(error) };
    } finally {
      busy = false;
    }
  }

  async function end(id: number, ending: AgreementEnding): Promise<void> {
    busy = true;
    notice = null;
    failure = null;
    try {
      await endAgreement(handle, id, { state: ending });
      notice = `「${agreementStateLabel(ending)}」として終わらせました。次のターンから本文に載りません。`;
      confirming = null;
      await load();
    } catch (error) {
      failure = { action: "end", detail: describeAgreementFailure(error) };
    } finally {
      busy = false;
    }
  }

  /**
   * 提案を採る（`body` が `{}` ならそのまま / 作業単位 A-12c-2）。
   *
   * **成否にかかわらず読み直す** —— 失敗の一手が「取り直しました」と言う（`describeProposalFailure`）。
   */
  async function adopt(proposal: AgreementView, body: AdoptAgreementRequest): Promise<void> {
    busy = true;
    notice = null;
    failure = null;
    proposalFailure = null;
    try {
      await adoptAgreement(handle, proposal.id, body);
      notice = PROPOSAL_ADOPTED_NOTICE;
      // **閉じるのは、採った提案の欄だけ**（別の提案を直しかけていたら、その字を消さない）。
      if (editingProposal === proposal.id) {
        editingProposal = null;
      }
    } catch (error) {
      proposalFailure = describeProposalFailure(error);
    }
    // **読み直しが返るまで押させない**（古い一覧の上で、もう採った提案をもう一度押させない）。
    try {
      await reloadQuietly();
    } finally {
      busy = false;
    }
  }

  /**
   * **表を畳まずに読み直す**（作業単位 A-12c-2）。`load` は表を一度畳むので、
   * 直しかけの欄（`ProposalEditor` の下書き）まで消える —— 上限で断られた後に打ち直させない。
   */
  async function reloadQuietly(): Promise<void> {
    await Promise.all([loadAgreements(), loadTransitions()]);
  }

  /** 提案を捨てる（作業単位 A-12c-2）。**行は消えない**（記録に残る）。 */
  async function reject(proposal: AgreementView): Promise<void> {
    busy = true;
    notice = null;
    failure = null;
    proposalFailure = null;
    try {
      await rejectAgreement(handle, proposal.id);
      notice = PROPOSAL_REJECTED_NOTICE;
      if (editingProposal === proposal.id) {
        editingProposal = null;
      }
    } catch (error) {
      proposalFailure = describeProposalFailure(error);
    }
    // **読み直しが返るまで押させない**（`adopt` と同じ）。
    try {
      await reloadQuietly();
    } finally {
      busy = false;
    }
  }

  void load();
</script>

<section>
  <header>
    <h1>{labelFor({ route: "agreements", handle })}</h1>
    <button type="button" onclick={() => void load()}>再読み込み</button>
    <a href={hrefFor({ route: "characters" })}>キャラクターの一覧へ</a>
  </header>

  <!--
    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8 / `tests/screen-wording.test.ts`）。
  -->
  <p class="hint">
    <strong>有効な約束・決まりは、毎ターンのプロンプトに載ります。</strong>
    入りきらないときは古いものから載らなくなり、そのときはここでお知らせします。
    果たした・破った・取り消したものは載りません（一覧と記録には残ります）。
  </p>

  {#if notice !== null}
    <p class="notice" role="status">{notice}</p>
  {/if}
  {#if failure !== null}
    <!-- **サーバが言ったことを、そのまま出す**（こちらから足さない）。 -->
    <p
      class="error"
      role="alert"
      data-agreement-failure={failure.action}
      data-agreement-reason={failure.detail.reason}
    >
      {failure.detail.message}
    </p>
    {#if failure.detail.guidance !== null}
      <p class="guidance" data-agreement-guidance="1">{failure.detail.guidance}</p>
    {/if}
  {/if}

  {#if agreementsState.kind === "failed"}
    <!-- **「読めなかった」を「1件も無い」に見せない。** -->
    <p class="error" role="alert" data-agreements-error="1">
      約束・決まりを取得できませんでした: {agreementsState.message}
    </p>
  {:else if agreementsState.kind === "loading"}
    <p role="status">読み込み中…</p>
  {:else}
    {@const view = agreementsState.value}
    {@const active = activeAgreements(view)}
    {@const ended = endedAgreements(view)}
    {@const dropped = droppedNotice(view, (at) => timeLabel(at, zone))}
    {@const proposals = proposedAgreements(view)}
    {@const proposalFull = proposalLimitNotice(view)}

    <!--
      **提案**（作業単位 A-12c-2 / ADR-0051 §決定2）。裏方が会話から見つけたもの。**採るまで本文に載らない。**
      **捨てたもの（却下）はここにも「終わった約束・決まり」にも出さない**（記録の側にだけ出る）。
    -->
    <h2>提案</h2>

    <p class="muted">
      会話から見つかった約束・決まりの候補です。採るまで本文には載りません。
      種類は判定していないので、約束か決まりかは「直して採る」で選べます。
    </p>

    {#if proposalFull !== null && proposals.length > 0}
      <!--
        **採れない理由と次の一手を、押す前に言う**（送らせてから 409 で断らない）。
        **割り込まない**（`role="status"`）—— いま起きた失敗ではなく、状態の案内。提案が現れるたびに
        読み上げを止めない。
      -->
      <p class="note" role="status" data-proposal-full="1">{proposalFull}</p>
    {/if}

    {#if proposalFailure !== null}
      <!-- **サーバが言ったことを、そのまま出す**（こちらから足さない）。一手は下の段落。 -->
      <p class="error" role="alert" data-proposal-reason={proposalFailure.reason}>
        {proposalFailure.message}
      </p>
      {#if proposalFailure.guidance !== null}
        <p class="guidance" data-proposal-guidance="1">{proposalFailure.guidance}</p>
      {/if}
    {/if}

    {#if proposals.length === 0}
      <p class="muted" data-no-proposals="1">いま提案はありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>守る側</th>
            <th>種類</th>
            <th>中身</th>
            <th>見つけた日時</th>
            <th>採るか</th>
          </tr>
        </thead>
        <tbody data-section="proposed-agreements">
          {#each proposals as proposal (proposal.id)}
            <tr
              data-agreement-id={proposal.id}
              data-agreement-keeper={proposal.keeper}
              data-agreement-state={proposal.state}
            >
              <td>{agreementKeeperLabel(proposal.keeper)}</td>
              <!-- **種類は判定していない**（サーバはいつも「決まり」で保存する）。 -->
              <td>{PROPOSAL_KIND_UNDECIDED}</td>
              <td>{proposal.content}</td>
              <td title={proposal.created_at}>{timeLabel(proposal.created_at, zone)}</td>
              <td>
                <button
                  type="button"
                  disabled={busy || proposalFull !== null}
                  onclick={() => void adopt(proposal, {})}
                >
                  採る
                </button>
                <button
                  type="button"
                  disabled={busy || proposalFull !== null}
                  onclick={() => (editingProposal = proposal.id)}
                >
                  直して採る
                </button>
                <button type="button" disabled={busy} onclick={() => void reject(proposal)}>
                  捨てる
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#each proposals as proposal (proposal.id)}
        {#if editingProposal === proposal.id}
          <h3>直して採る</h3>
          <ProposalEditor
            {proposal}
            {busy}
            blocked={proposalFull !== null}
            onAdopt={(body) => void adopt(proposal, body)}
            onCancel={() => (editingProposal = null)}
          />
        {/if}
      {/each}
    {/if}

    <h2>有効な約束・決まり</h2>

    <p class="muted" data-agreement-limit={view.active_limit}>
      {agreementLimitNotice(view)}
    </p>

    {#if dropped !== null}
      <!--
        **載らなかった取り決めがあることを言う**（作業単位 A-12a の決め直し —— 黙って落とさない）。
        **「壊れている」側の色**（良い報せではない）。
      -->
      <p class="error" role="alert" data-agreement-dropped={view.last_turn?.dropped ?? 0}>
        {dropped}
      </p>
    {/if}

    {#if active.length === 0}
      <p class="muted">有効な約束・決まりはありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>守る側</th>
            <th>種類</th>
            <th>中身</th>
            <th>書いた日時</th>
            <th>終わらせる</th>
          </tr>
        </thead>
        <tbody data-section="active-agreements">
          {#each active as agreement (agreement.id)}
            <tr
              data-agreement-id={agreement.id}
              data-agreement-keeper={agreement.keeper}
              data-agreement-state={agreement.state}
            >
              <td>{agreementKeeperLabel(agreement.keeper)}</td>
              <td>{agreementKindLabel(agreement.kind)}</td>
              <td>{agreement.content}</td>
              <td title={agreement.created_at}>{timeLabel(agreement.created_at, zone)}</td>
              <td>
                {#if confirming !== null && confirming.id === agreement.id}
                  {@const ending = confirming.ending}
                  <!--
                    **戻せないことを、送る前に言う**（有効に戻す口は無い。書き直すなら新しく書く）。
                  -->
                  <span class="ending-confirm" data-agreement-confirm={ending}>
                    「{agreementStateLabel(ending)}」として終わらせます。戻せません。
                  </span>
                  <button
                    type="button"
                    disabled={busy}
                    onclick={() => void end(agreement.id, ending)}
                  >
                    終わらせる
                  </button>
                  <button type="button" disabled={busy} onclick={() => (confirming = null)}>
                    やめる
                  </button>
                {:else}
                  {#each AGREEMENT_ENDINGS as ending (ending)}
                    <button
                      type="button"
                      disabled={busy}
                      data-ending={ending}
                      onclick={() => (confirming = { id: agreement.id, ending })}
                    >
                      {agreementStateLabel(ending)}
                    </button>
                  {/each}
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <h2>書く</h2>

    <form
      onsubmit={(event) => {
        event.preventDefault();
        void write();
      }}
    >
      <!--
        **上限に達したら、書く欄ごと止める**（送らせてから 409 で断らない）。
        **次の一手（古いものを終わらせる）は上の案内が言う。**
      -->
      <fieldset disabled={atLimit || busy} data-agreement-form={atLimit ? "full" : "open"}>
        {#if atLimit}
          <p class="error" role="alert" data-agreement-full="1">
            {agreementLimitNotice(view)}
          </p>
        {/if}
        <label>
          中身
          <input
            name="content"
            value={draft.content}
            oninput={(event) => (draft.content = event.currentTarget.value)}
          />
        </label>
        <span
          class="counter"
          data-counter="content"
          class:over={isOverAgreementLimit(draft.content)}
        >
          {agreementCounter(draft.content)}
        </span>
        <label>
          種類
          <select
            name="kind"
            value={draft.kind}
            onchange={(event) => (draft.kind = event.currentTarget.value)}
          >
            <!-- **既定を作らない**（約束か決まりかは、書く人が決める）。 -->
            <option value="">選んでください</option>
            {#each AGREEMENT_KINDS as kind (kind)}
              <option value={kind}>{agreementKindLabel(kind)}</option>
            {/each}
          </select>
        </label>
        <label>
          守る側
          <select
            name="keeper"
            value={draft.keeper}
            onchange={(event) => (draft.keeper = event.currentTarget.value)}
          >
            <!--
              **既定を作らない** —— 守る側を取り違えると、以後の全ターンに
              「誰が守るのか」が入れ替わった取り決めが載る。
            -->
            <option value="">選んでください</option>
            {#each AGREEMENT_KEEPERS as keeper (keeper)}
              <option value={keeper}>{agreementKeeperLabel(keeper)}</option>
            {/each}
          </select>
        </label>
        <p class="note">
          中身は {AGREEMENT_CONTENT_MAX_CHARS} 文字まで。「誰が・何を」が分かる短い平叙文で書きます。
        </p>
        {#if !built.ok && draft.content !== ""}
          <p class="error" role="alert" data-build-error="1">{built.message}</p>
        {/if}
        <div class="actions">
          <button type="submit" disabled={!canWrite}>書く</button>
        </div>
      </fieldset>
    </form>

    <h2>終わった約束・決まり</h2>

    {#if ended.length === 0}
      <p class="muted">終わった約束・決まりはありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>守る側</th>
            <th>種類</th>
            <th>中身</th>
            <th>状態</th>
            <th>書いた日時</th>
          </tr>
        </thead>
        <tbody data-section="ended-agreements">
          {#each ended as agreement (agreement.id)}
            <tr
              data-agreement-id={agreement.id}
              data-agreement-keeper={agreement.keeper}
              data-agreement-state={agreement.state}
            >
              <td>{agreementKeeperLabel(agreement.keeper)}</td>
              <td>{agreementKindLabel(agreement.kind)}</td>
              <td>{agreement.content}</td>
              <td>{agreementStateLabel(agreement.state)}</td>
              <td title={agreement.created_at}>{timeLabel(agreement.created_at, zone)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}

  <h2>記録</h2>

  {#if transitionsState.kind === "failed"}
    <p class="error" role="alert" data-transitions-error="1">
      記録を取得できませんでした: {transitionsState.message}
    </p>
  {:else if transitionsState.kind === "loading"}
    <p role="status">読み込み中…</p>
  {:else}
    {@const transitions = transitionsState.value}
    <p class="muted" data-transition-total={transitions.total_transitions}>
      {transitionCountNotice(transitions)}
    </p>
    {#if transitions.transitions.length > 0}
      <table>
        <thead>
          <tr>
            <th>日時</th>
            <th>守る側</th>
            <th>種類</th>
            <th>中身</th>
            <th>前</th>
            <th>後</th>
          </tr>
        </thead>
        <tbody data-section="agreement-transitions">
          <!-- **新しい順のまま出す**（並びは `store` の1文が決める）。 -->
          {#each transitions.transitions as transition (transition.id)}
            <tr data-transition-id={transition.id}>
              <td title={transition.created_at}>{timeLabel(transition.created_at, zone)}</td>
              <td>{agreementKeeperLabel(transition.keeper)}</td>
              <!-- **いま提案中・却下の取り決めの行は「未判定」**（`transitionKindLabel` の doc）。 -->
              <td>
                {transitionKindLabel(
                  transition,
                  agreementsState.kind === "ready" ? agreementsState.value.agreements : null,
                )}
              </td>
              <td>{transition.content}</td>
              <td>{transitionBeforeLabel(transition.before, transition.after)}</td>
              <td>{agreementStateLabel(transition.after)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</section>

<style>
  /* **表・枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  fieldset {
    border: none;
    padding: 0;
    margin: 0;
  }

  label {
    margin-right: 1rem;
  }

  .ending-confirm {
    margin-right: 0.5rem;
  }
</style>
