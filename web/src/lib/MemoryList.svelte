<script lang="ts">
  /**
   * **記憶の一覧と修復**（作業単位 D-3 / ADR-0022）。
   *
   * # **これは想起ではない**
   *
   * 想起の入口は `Store::recall_memories` ひとつだけ（ADR-0003）。
   * この画面が読むものは**プロンプトに一切載らない。**
   *
   * # **消す道は無い**（絶対制約 / ADR-0022 §1）
   *
   * `memory` の行は DELETE も UPDATE もされない。**できるのは
   * 「訂正版を足して、古い行にそれを指させる」**だけで、
   * サーバにもその2本しか経路が無い（`GET .../memories` と
   * `POST .../memories/{id}/corrections`）。
   * **画面の文言も「削除」「修正」ではなく「訂正版を足す」で揃えてある。**
   *
   * # ⚠ **訂正済みの行が並ぶ節は2つあり、寿命が違う**
   *
   * | 節 | どこから来るか | 寿命 |
   * |---|---|---|
   * | **この画面で訂正した記憶** | この画面が `POST` を通した記録 | **画面と同じ** |
   * | **上書きされた記憶（DB に残っている記録）** | `GET …?include=superseded` | **DB と同じ** |
   *
   * # **D-3 の但し書きを、事実に合わせて書き直した**（作業単位 D-3c / 2026-08-28）
   *
   * D-3 の時点では **「訂正済み」に出るのはこの画面が `POST` を通した行だけ**で、
   * **記録は DB の側にしか無く、画面からは読めなかった**（読み出しの入口が
   * どれも `superseded_by IS NULL` しか見ていなかった）。
   * **D-3b で `?include=superseded` ができた**ので、
   * **いまは DB の記録を同じ画面で読める**——**再読み込みでも消えない。**
   *
   * **⚠ 何が変わっていないか**（測ったぶんだけ書く）:
   *
   * * **1つめの節は、いまも画面と同じ寿命しか持たない。**
   *   別のキャラクターへ移って戻れば消え（`App.svelte` の `{#key screen.handle}`）、
   *   再読み込みでも消える。**残してあるのは、2つめが答えない回があるから**（次の2つ）。
   * * **一覧の取得に失敗している間は、どちらの節も出ない**（`{#if listError}` の下）。
   *   訂正が通ったことと2つの id は**通知（`p.notice`）にだけ残る。**
   * * **サーバが古い一覧を返した回**は、訂正した行が2つめの節に出てこない
   *   （`superseded_by` がまだ入っていない）。このとき1つめの節だけがそれを言える。
   * * **DB の記録も「全部」ではない。** 上限（`limit`）で切られるので、
   *   **総数との差を必ず出す**——出さないと**画面が「これで全部」と嘘をつく。**
   * * **応答に節が無い回は「0 件」ではない。** 訊いていないのか、
   *   サーバが古いのかの区別が付かないので、**そう書いて出す。**
   * * **「いつ訂正したか」はどこにも無い。** 並んでいるのは
   *   **その記憶が書かれた時刻**である（上書きした時刻は保存していない）。
   * * **持ち主を取り違えた記憶を、正しい持ち主へ移す手段は無いまま**
   *   （CLAUDE.md 1節の ⚠）。**読めるようになっただけである。**
   *
   * # `locked` は出てこない（ADR-0022 §4）
   *
   * 意味が未確定なので API が返さない。**壊れている場合だけ、
   * `invalid_columns` に列名として現れる。**
   */

  import {
    ApiRequestError,
    correctMemory,
    listMemories,
    type MemoryListView,
    type MemorySubject,
    type MemoryView,
    type UnreadableMemoryView,
  } from "./api";
  import {
    SUBJECTS,
    annotateSuperseded,
    buildCorrection,
    chainLabel,
    draftForMemory,
    draftForUnreadable,
    hiddenSuperseded,
    isUnchanged,
    limitLabel,
    shownSuperseded,
    subjectLabel,
    successorLabel,
    totalSuperseded,
    type CorrectionDraft,
  } from "./memoryRepair";
  import { hrefFor } from "./router";
  // **時刻の綴りは1箇所**（作業単位 F-7 / 変容の表と同じ関数）。
  import { browserZone, timeLabel } from "./timestamp";

  interface Props {
    /** どのキャラクターの記憶か。**経路が運ぶ**（`router.ts` の `Screen`）。 */
    handle: string;
  }

  let { handle }: Props = $props();

  /**
   * **読み手の時間帯**（作業単位 F-7b）。
   *
   * **ここで解決して `timeLabel` へ渡す** ——
   * **`timeLabel` に自分で決めさせない。**
   * 決めさせると、**あの関数を見る検査が、走らせた機械の時間帯でしか
   * 成立しなくなる**（F-7 が地域化を避けた2つ目の理由）。
   *
   * **⚠ 綴りを直接書かないこと**（`"Asia/Tokyo"` 等）。
   * 書くと、**開発者の機械では正しく見えて、他の誰の画面でも時刻がずれる。**
   * 見張り番は `tests/timestamp-wiring.test.ts`。
   */
  const zone = browserZone();

  /**
   * 訂正の相手。
   *
   * **読めた行と読めなかった行を1つの型にまとめない。**
   * まとめると、**読めなかった行の主語や本文を「空の値」として持つ**ことになり、
   * **「まだ選んでいない」と「空だと分かっている」が同じ表現**になる。
   */
  type Target =
    | { kind: "memory"; memory: MemoryView }
    | { kind: "unreadable"; broken: UnreadableMemoryView };

  /** **この画面が訂正した行**（上の ⚠）。 */
  interface Corrected {
    target: Target;
    /** 新しく書かれた行の id。**次に直すときはこの id を指す。** */
    correctionId: number;
  }

  function idOf(target: Target): number {
    return target.kind === "memory" ? target.memory.id : target.broken.id;
  }

  let list = $state<MemoryListView | null>(null);
  let listError = $state<string | null>(null);
  let selected = $state<Target | null>(null);
  let draft = $state<CorrectionDraft>(draftForUnreadable());
  let saving = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let corrected = $state<Corrected[]>([]);

  /** この画面が想起から外した id。 */
  const supersededIds = $derived(
    new Set(corrected.map((entry) => idOf(entry.target))),
  );

  /**
   * いま想起される記憶。
   *
   * **訂正した行を落とす。** サーバは訂正の直後から返さなくなるが、
   * **一覧が古いまま届いた回**（`superseded_by` の入っていない応答・
   * 別の窓の古い応答）に、想起から外れた行が
   * 「想起される」側に残り続けるのを防ぐ。
   *
   * **⚠ 「読み直しに失敗した回」の話ではない。** 失敗したときは
   * `list` が `null` になって節ごと出なくなる（下の `{#if listError}`）ので、
   * この絞り込みは効きようがない。**効くのは「古い一覧が届いた回」だけ。**
   * 見張り番は `tests/memory-list.test.ts::`
   * `一覧が古いままでも、訂正した行は想起される側に戻らない`。
   */
  const activeMemories = $derived(
    (list?.memories ?? []).filter((memory) => !supersededIds.has(memory.id)),
  );

  const brokenRows = $derived(
    (list?.unreadable ?? []).filter((broken) => !supersededIds.has(broken.id)),
  );

  /**
   * **サーバが返した、上書きされた記憶の節**（作業単位 D-3c）。
   *
   * **`null` は「1件も無い」ではなく「節が来なかった」。**
   * 1件も無いときは `total: 0` の節が来る（`api.ts` の `MemoryListView` の ⚠）。
   */
  const storedSuperseded = $derived(list?.superseded ?? null);

  /**
   * **いま想起される行の id**（読めた行と読めなかった行の**両方**）。
   *
   * **鎖の先端を判定するために要る。** 読めない行を数に入れないと、
   * **壊れた行に置き換わった記憶が「この一覧には出ていません」に見える。**
   *
   * **⚠ `activeMemories` ではなく応答をそのまま使う。**
   * 鎖は**1つの応答の中だけで閉じている**話なので、
   * この画面が持っている訂正の記録を混ぜると、**同じ応答を2通りに読むことになる。**
   */
  const respondedActiveIds = $derived([
    ...(list?.memories ?? []).map((memory) => memory.id),
    ...(list?.unreadable ?? []).map((broken) => broken.id),
  ]);

  /** 鎖を添えた上書き済みの行。**並べ替えない**（`store` の1文が決めている）。 */
  const supersededChain = $derived(
    storedSuperseded === null
      ? null
      : annotateSuperseded(storedSuperseded, respondedActiveIds),
  );

  /** 送る本文。**組み立ては純粋関数**（`memoryRepair.ts`）。 */
  const built = $derived(buildCorrection(draft));

  /**
   * 元と同じ内容か。**同じなら送らない**（行だけが増える）。
   *
   * **読めなかった行には「元」が無い**ので、常に送れる。
   */
  const unchanged = $derived(
    selected !== null &&
      selected.kind === "memory" &&
      isUnchanged(selected.memory, draft),
  );

  const canSubmit = $derived(built.ok && !unchanged && !saving);

  function messageOf(failure: unknown): string {
    if (failure instanceof ApiRequestError) {
      return failure.message;
    }
    return "予期しない失敗です";
  }

  async function load(): Promise<void> {
    listError = null;
    try {
      // **上書きされた記憶も一緒に読む**（作業単位 D-3c）。
      //
      // **⚠ 型検査が縛っているのは FE 側の手書きの定数だけ**
      // （`MemoryListInclude` は `api.ts` の `MEMORY_LIST_INCLUDES` から作った union）。
      // **サーバの綴りと繋いでいるのは `tests/memory-chain.test.ts` の写し検査**で、
      // あちらが `src/server/admin.rs` を実際に読んでいる。
      // **`vitest run` は型検査をしない。** どちらか片方では足りない。
      list = await listMemories(handle, "superseded");
    } catch (failure) {
      list = null;
      listError = messageOf(failure);
    }
  }

  /**
   * 訂正の下書きを開く。**必ず作り直す。**
   *
   * 持ち越すと、**前の行の本文が別の記憶の訂正版として書き込まれる**
   * （`CharacterList` の `{#key}` と同じ壊れ方）。
   */
  function openCorrection(target: Target): void {
    selected = target;
    draft =
      target.kind === "memory"
        ? draftForMemory(target.memory)
        : draftForUnreadable();
    error = null;
    notice = null;
  }

  function closeCorrection(): void {
    selected = null;
  }

  /** 主語を選ぶ。**知らない綴りは「選んでいない」に落とす。** */
  function selectSubject(raw: string): void {
    const found: MemorySubject | null =
      SUBJECTS.find((subject) => subject === raw) ?? null;
    draft = { ...draft, subject: found };
  }

  async function save(): Promise<void> {
    const request = built;
    const target = selected;
    if (!canSubmit || !request.ok || target === null) {
      return;
    }
    saving = true;
    error = null;
    notice = null;

    try {
      const view = await correctMemory(handle, idOf(target), request.body);
      // **記録してから読み直す。** 読み直しに失敗しても、
      // 想起から外れた行が「想起される」側に残らない。
      corrected = [...corrected, { target, correctionId: view.correction_id }];
      selected = null;
      notice = `id ${view.superseded_id} は想起から外れました（行は消えていません）。訂正版は id ${view.correction_id} です。`;
      await load();
    } catch (failure) {
      error = messageOf(failure);
    } finally {
      saving = false;
    }
  }

  void load();
</script>

<section>
  <header>
    <h1>{handle} の記憶</h1>
    <button type="button" onclick={() => void load()}>再読み込み</button>
    <a href={hrefFor({ route: "characters" })}>キャラクターの一覧へ</a>
  </header>

  <!--
    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
    **消さずに訂正版を足すという決定は ADR-0022**——
    **その番号は、ここ（コメント）とこのファイルの冒頭にだけ残す。**
    **画面は利用者に向けて書く**（見張り番は `tests/screen-wording.test.ts` と
    `tests/memory-list.test.ts` の「画面が利用者の言葉で話す」）。
  -->
  <p class="hint">
    記憶は消えません。誤りは<strong>訂正版を足して</strong>置き換えます。
    訂正した行は想起されなくなりますが、記録としては残ります。
  </p>

  {#if selected !== null}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <h3>
        id {idOf(selected)} の訂正版を書く
        {#if selected.kind === "unreadable"}
          （読み出せなかった行）
        {/if}
      </h3>

      {#if selected.kind === "unreadable"}
        <p class="hint">
          元の値は読み出せていません。<strong>推測で埋めないでください。</strong>
          いま正しいと分かっていることだけを書きます。
        </p>
      {/if}

      <label>
        <span>主語（誰についての記憶か）</span>
        <!--
          **既定を作らない**（`draftForUnreadable`）。押し間違いで
          持ち主の入れ替わった記憶ができると、**正しい持ち主へ移す手段が無い。**
        -->
        <select
          name="subject"
          value={draft.subject ?? ""}
          disabled={saving}
          onchange={(event) => selectSubject(event.currentTarget.value)}
        >
          <option value="">選んでください</option>
          {#each SUBJECTS as subject (subject)}
            <option value={subject}>{subjectLabel(subject)}</option>
          {/each}
        </select>
      </label>

      <label>
        <span>訂正版の本文</span>
        <!--
          **上限を写していない**（`memoryRepair.ts`）。長すぎればサーバが理由を返す。
        -->
        <textarea
          name="content"
          rows="3"
          bind:value={draft.content}
          disabled={saving}
        ></textarea>
      </label>

      <label>
        <span>分類</span>
        <input name="category" bind:value={draft.category} disabled={saving} />
      </label>
      <p class="hint">空にすると未設定になります。</p>

      {#if !built.ok}
        <p class="error" role="alert">{built.message}</p>
      {:else if unchanged}
        <p class="error" role="alert">
          元と同じ内容です。訂正版として足すものがありません。
        </p>
      {/if}
      {#if error !== null}
        <p class="error" role="alert">{error}</p>
      {/if}

      <div class="actions">
        <button type="submit" disabled={!canSubmit}>訂正版を追加する</button>
        <button type="button" onclick={closeCorrection} disabled={saving}>
          閉じる
        </button>
      </div>
    </form>
  {/if}

  {#if notice !== null}
    <p class="notice" role="status">{notice}</p>
  {/if}

  {#if listError !== null}
    <p class="error" role="alert">記憶の一覧を取得できませんでした: {listError}</p>
  {:else if list === null}
    <p role="status">読み込み中…</p>
  {:else}
    <h2>いま想起される記憶（{activeMemories.length} 件）</h2>
    {#if activeMemories.length === 0}
      <p class="muted">ありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>id</th>
            <th>主語</th>
            <th>本文</th>
            <th>分類</th>
            <th>作成</th>
            <th></th>
          </tr>
        </thead>
        <tbody data-section="active">
          {#each activeMemories as memory (memory.id)}
            <!--
              **生の主語も持つ**（`data-subject`）。写した型が外れた日に、
              見出しが空になるだけでなく値が見える形にしておく。
            -->
            <tr
              data-memory-id={memory.id}
              data-subject={memory.subject}
              data-state="active"
            >
              <td>{memory.id}</td>
              <td>{subjectLabel(memory.subject)}</td>
              <td>{memory.content}</td>
              <td>
                {#if memory.category === null}
                  <span class="muted">未設定</span>
                {:else}
                  {memory.category}
                {/if}
              </td>
              <!--
                **⚠ 生の ISO 表記を出さない**（作業単位 F-7 / `timestamp.ts`）。
                **瞬間は動かさない**（地域化しない）。
                **落としたミリ秒は `title` に残す** ——
                1ターンで複数の記憶が入るので、**同じ秒の行は本番で普通に起きる。**
              -->
              <td title={memory.created_at}
                >{timeLabel(memory.created_at, zone)}</td
              >
              <td>
                <!-- **この行の記憶を渡す**（一覧の行と送り先がここで結び付く）。 -->
                <button
                  type="button"
                  onclick={() => openCorrection({ kind: "memory", memory })}
                >
                  訂正版を足す
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      **読み出せなかった行**（ADR-0022 §2）。
      **0件でも節ごと消さない**——「壊れた行が無い」ことと
      「この画面が壊れた行を見せない古い版である」ことを、見る側が区別できるように。
    -->
    <h2>読み出せなかった記憶（{brokenRows.length} 件）</h2>
    {#if brokenRows.length === 0}
      <p class="muted">ありません。</p>
    {:else}
      <p class="hint">
        この行があると、そのキャラクターの会話も抽出も止まります（想起は1行でも
        読めないと丸ごと止まるため）。訂正版を足すと想起から外れます。
      </p>
      <table>
        <thead>
          <tr>
            <th>id</th>
            <th>読めなかった列</th>
            <th></th>
          </tr>
        </thead>
        <tbody data-section="unreadable">
          {#each brokenRows as broken (broken.id)}
            <tr data-memory-id={broken.id} data-state="unreadable">
              <td>{broken.id}</td>
              <td><code>{broken.invalid_columns.join(", ")}</code></td>
              <td>
                <button
                  type="button"
                  onclick={() => openCorrection({ kind: "unreadable", broken })}
                >
                  訂正版を足す
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      **この画面で訂正した記憶**（上の ⚠ —— **画面と同じ寿命しか持たない**）。
      **下の「上書きされた記憶」とは別物。** あちらは DB の記録で、
      **こちらは、あちらがまだ答えていない回のためにある**
      （古い一覧が届いた回 / `superseded_by` がまだ入っていない回）。
    -->
    <h2>この画面で訂正した記憶（{corrected.length} 件）</h2>
    <p class="hint">
      この節は<strong>この画面を開いてからの記録</strong>で、再読み込みで消えます。
      DB に残っている記録は下の「上書きされた記憶」です。
    </p>
    {#if corrected.length === 0}
      <p class="muted">ありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>id</th>
            <th>主語</th>
            <th>本文</th>
            <th>いまの状態</th>
          </tr>
        </thead>
        <tbody data-section="superseded">
          {#each corrected as entry (idOf(entry.target))}
            <tr
              data-memory-id={idOf(entry.target)}
              data-subject={entry.target.kind === "memory"
                ? entry.target.memory.subject
                : undefined}
              data-state="superseded"
            >
              <td>{idOf(entry.target)}</td>
              <td>
                {#if entry.target.kind === "memory"}
                  {subjectLabel(entry.target.memory.subject)}
                {:else}
                  <span class="muted">読めませんでした</span>
                {/if}
              </td>
              <td>
                {#if entry.target.kind === "memory"}
                  {entry.target.memory.content}
                {:else}
                  <code>{entry.target.broken.invalid_columns.join(", ")}</code>
                {/if}
              </td>
              <td>
                想起から外れました（訂正版 id {entry.correctionId}）。行は残っています。
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      **DB に残っている、上書きされた記憶**（作業単位 D-3c / ADR-0022）。

      **ここは想起でも修復の対象でもない。** 並ぶのは
      **二度と想起されない行**で、**もう訂正もできない**
      （サーバが `memory_already_superseded` で拒む）ので、**押せる導線を置かない。**

      **節ごと消さないこと。** 「1件も無い」と「訊いていない／サーバが古い」は
      **別のこと**で、後者を 0 件と書くと**画面が嘘をつく。**
    -->
    <div data-block="stored-superseded">
      <h2>上書きされた記憶（DB に残っている記録）</h2>
      <p class="hint">
        訂正版に置き換わった行です。<strong>もう想起されません</strong>し、
        ここからさらに訂正することもできません。記録として残っているだけです。
      </p>

      {#if storedSuperseded === null}
        <!--
          **「0 件」と言わないこと。** 問い合わせが届いていないだけで、
          **DB には上書き済みの行がいくらでもありうる。**
        -->
        <p class="error" role="alert">
          この応答には、上書きされた記憶の節がありませんでした（この画面は
          <code>?include=superseded</code> を求めています）。<strong
            >1件も無いという意味ではありません。</strong
          >
        </p>
      {:else}
        <!--
          **総数を必ず言う**（`admin.rs` の `RECENT_SUPERSEDED_MEMORIES` の doc ——
          **「言えないまま切ると、画面は『これで全部』と嘘をつく」**）。
          **上限も応答のもの**（こちらに写しを持たない）。
          **出した件数は読めなかった行も数える**（サーバの上限が種別で分かれていない）。
        -->
        <p
          class="muted"
          data-superseded-summary
          data-superseded-total={totalSuperseded(storedSuperseded)}
        >
          {#if totalSuperseded(storedSuperseded) === null}
            <!--
              **総数が無いまま「すべて出しています」と言わない。**
              欄が欠けた応答をそのまま描くと「全　件…すべて出しています」になる
              ——**数を言わないまま「これで全部」と言い切る形。**
            -->
            上書きされた記憶の総数が、応答にありませんでした（直近 {shownSuperseded(
              storedSuperseded,
            )} 件だけを出しています。<strong>これで全部とは限りません。</strong>）
          {:else if totalSuperseded(storedSuperseded) === 0}
            まだ1件も上書きされた記憶がありません（このキャラクターの記憶は、まだ一度も訂正されていません）。
          {:else if hiddenSuperseded(storedSuperseded) > 0}
            全 {totalSuperseded(storedSuperseded)} 件の上書きされた記憶のうち、直近 {shownSuperseded(
              storedSuperseded,
            )} 件を出しています（1回に返る上限は {limitLabel(storedSuperseded)} / 残り {hiddenSuperseded(
              storedSuperseded,
            )} 件はここに出ていません）。
          {:else}
            全 {totalSuperseded(storedSuperseded)} 件の上書きされた記憶を、すべて出しています（1回に返る上限は
            {limitLabel(storedSuperseded)}）。
          {/if}
        </p>
      {/if}

      {#if supersededChain !== null && supersededChain.memories.length > 0}
        <table>
          <thead>
            <tr>
              <th>id</th>
              <th>主語</th>
              <th>本文</th>
              <th>分類</th>
              <th>作成</th>
              <th>置き換わった先</th>
              <th>鎖</th>
            </tr>
          </thead>
          <tbody data-section="stored-superseded">
            <!--
              **新しい順のまま出す。** 並びを決めているのは `store` の1文
              （同着は本番で普通に起きる —— 1ターンで最大5件の記憶が入る）。
              **並ぶのは「訂正した時刻」ではなく、その記憶が書かれた時刻。**
            -->
            {#each supersededChain.memories as row (row.memory.id)}
              <tr
                data-memory-id={row.memory.id}
                data-subject={row.memory.subject}
                data-superseded-by={row.memory.superseded_by}
                data-state="stored-superseded"
              >
                <td>{row.memory.id}</td>
                <td>{subjectLabel(row.memory.subject)}</td>
                <td>{row.memory.content}</td>
                <td>
                  {#if row.memory.category === null}
                    <span class="muted">未設定</span>
                  {:else}
                    {row.memory.category}
                  {/if}
                </td>
                <!-- **⚠ 生の ISO 表記を出さない**（作業単位 F-7 / 上の表と同じ扱い）。 -->
                <td title={row.memory.created_at}
                  >{timeLabel(row.memory.created_at, zone)}</td
                >
                <!-- **後継はこの行のもの**（`id` と取り違えると鎖が自分を指す）。 -->
                <td>{successorLabel(row.chain)}</td>
                <td>{chainLabel(row.chain)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      <!--
        **読めない上書き済みの行**（ADR-0022 §2）。**落とさないこと。**
        落とすと**壊れた環だけが鎖から落ち、`total` にだけ残る。**
        **後継は壊れた行でも分かる**（`parse` を通さない列）ので、鎖はここで切れない。
      -->
      {#if supersededChain !== null && supersededChain.unreadable.length > 0}
        <h3>読み出せなかった、上書き済みの記憶</h3>
        <p class="hint">
          この行は<strong>すでに想起から外れています</strong>ので、会話も抽出も止めません。
        </p>
        <table>
          <thead>
            <tr>
              <th>id</th>
              <th>読めなかった列</th>
              <th>置き換わった先</th>
              <th>鎖</th>
            </tr>
          </thead>
          <tbody data-section="stored-superseded-unreadable">
            {#each supersededChain.unreadable as row (row.broken.id)}
              <tr
                data-memory-id={row.broken.id}
                data-superseded-by={row.broken.superseded_by}
                data-state="stored-superseded-unreadable"
              >
                <td>{row.broken.id}</td>
                <td><code>{row.broken.invalid_columns.join(", ")}</code></td>
                <td>{successorLabel(row.chain)}</td>
                <td>{chainLabel(row.chain)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}
</section>

<style>
  /* **表・枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /*
    **見出しの列が広い。** この画面の見出しは
    「上書きする記憶」「訂正版の本文」のように長いので、
    土台の 10rem では折り返す。**文字数の欄は無いので2列。**
  */
  form label {
    grid-template-columns: 14rem 1fr;
  }
</style>
