<script lang="ts">
  /**
   * **過去の会話の一覧**（作業単位 D-12c-2 / Issue origin#4）。**選ぶだけ。**
   *
   * # 読むのは親
   *
   * 一覧は `ChatView.svelte` が `GET .../sessions` で読んで渡す。
   * **ここは並べて、選ばれたことを伝えるだけ**（何を読み込むか・何を送るかは親が決める）。
   * 親子で2回読むと、**「続ける会話」と「一覧の先頭」が別々の応答から来る**ことになる。
   *
   * # ⚠ 並べ替えない
   *
   * **並びはサーバが素の `aoi` の続け方と同じ順で返している**（`api.ts` の `SessionListView`）。
   * 先頭に「送ると続く会話」と書けるのは、この並びのままだからである。
   *
   * # 時刻は読み手の時間帯で
   *
   * `MemoryList.svelte` と同じ —— **ここで解決して `timeLabel` へ渡す。**
   * **生の綴りは `title` に残す。** 見張り番は `tests/timestamp-wiring.test.ts`。
   *
   * **⚠ この部品は一覧を開いたときにしか描かれない。** 時間帯を読むのはここだけなので、
   * **会話の画面を開いただけでは時間帯を読まない**（`tests/support/zone-setup.ts` の門）。
   */
  import type { SessionListView, SessionSummaryView } from "./api";
  import { continueLatest } from "./chat";
  import { browserZone, timeLabel } from "./timestamp";

  interface Props {
    /** どのキャラクターの会話か（先頭の注記に使う）。 */
    handle: string;
    /** サーバが返した一覧（**並びのまま**）。 */
    list: SessionListView;
    /** いま画面に出している会話の id（開く釦を塞ぐ）。 */
    current: number | null;
    /** 送信中は選ばせない（送り先が途中で変わる）。 */
    busy: boolean;
    /** 選ばれた。 */
    onChoose: (session: SessionSummaryView) => void;
    /**
     * **「新しい会話を始める」が押された**（作業単位 D-17 / Issue origin#38）。
     *
     * **会話を選ぶ場所で、新しく始めることもできる**ようにする。何を送るか（`@new`）は親が決める
     * （冒頭の「読むのは親」と同じ立場）。
     */
    onBeginNew: () => void;
  }

  let { handle, list, current, busy, onChoose, onBeginNew }: Props = $props();

  /** **読み手の時間帯**（`MemoryList.svelte` の `zone` と同じ理由）。 */
  const zone = browserZone();

  /** 選ぶための手がかり。**切ったことを隠さない**（`…` を足す）。 */
  function openingOf(session: SessionSummaryView): string {
    if (session.first_user_message === null) {
      return "（まだ発言がありません）";
    }
    return session.first_user_message_truncated
      ? `${session.first_user_message}…`
      : session.first_user_message;
  }
</script>

<section class="picker" aria-label="過去の会話">
  <h2>過去の会話</h2>
  <!--
    **選ばずに、新しく始めることもできる**（作業単位 D-17）。**一覧の先頭に置く** ——
    会話が1つも無いときも、長い一覧の下まで流さなくても押せるように。**送信中は押せない**（送り先が途中で変わる）。
  -->
  <p class="begin">
    <button type="button" disabled={busy} onclick={onBeginNew}>新しい会話を始める</button>
    <span class="note">選んだ会話の続きではなく、新しい会話にします。</span>
  </p>
  {#if list.sessions.length === 0}
    <p class="note">まだ会話がありません。</p>
  {:else}
    <p class="note">
      選ぶと、その会話を読めます。そのまま送ると、その会話の続きになります。
    </p>
    <ol class="sessions">
      {#each list.sessions as session, index (session.id)}
        <li data-session-id={session.id}>
          <span class="when" title={session.updated_at}
            >{timeLabel(session.updated_at, zone)}</span
          >
          <span class="count">{session.message_count} 件</span>
          {#if index === 0}
            <!--
              **先頭は、素の識別子で送ったら続く会話**（並びがサーバの続け方と同じだから）。
              **並べ替えた瞬間に嘘になる**（冒頭の ⚠）。
            -->
            <span class="muted"
              >（<code>{continueLatest(handle)}</code> で送ると続く会話）</span
            >
          {/if}
          <p class="opening">{openingOf(session)}</p>
          <button
            type="button"
            disabled={busy || session.id === current}
            onclick={() => onChoose(session)}
          >
            この会話を開く
          </button>
        </li>
      {/each}
    </ol>
    {#if list.total_sessions > list.sessions.length}
      <!-- **全部ではないと言う**（言わずに切ると、画面が「これで全部」と嘘をつく）。 -->
      <p class="note">
        全 {list.total_sessions} 件のうち、新しい {list.sessions.length} 件を出しています。
      </p>
    {/if}
  {/if}
</section>

<style>
  /* **枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  .sessions {
    list-style: none;
    margin: 0.5rem 0;
    padding: 0;
  }

  .sessions > li {
    border-top: 1px solid var(--color-rule);
    padding: 0.4rem 0;
  }

  .opening {
    margin: 0.2rem 0;
    white-space: pre-wrap;
  }

  .count {
    margin-left: 0.5rem;
  }

  .begin .note {
    margin-left: 0.5rem;
  }
</style>
