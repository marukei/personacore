<script lang="ts">
  /**
   * **「設定」の画面**（作業単位 F-12b / ADR-0057 §決定5 / `GET /admin/server` / `PUT /admin/server/port`）。
   *
   * **personacore 自身の設定を置く画面。** いまは待ち受けのポートだけ。**上流の画面には置かない**
   * （推論サーバの話と、personacore 自身の待ち受けの話が混ざる）。
   *
   * # 言うこと（文は `serverSettings.ts` が組む。ここは並べるだけ）
   *
   * * いま待ち受けている番号と出どころ・保存した番号・次の起動で使う番号と出どころ（行）
   * * 保存できない起動（`editable: false`）では理由を言い、**入力と操作を出さない**
   * * 保存のファイルが読めない（`saved_port_unreadable: true`）ときは、そう言い、直し方を言う
   * * 保存・消去の後: 次の起動から効く（環境変数が勝つならそう言う）・使われている番号・**いまの待ち受けは移らない**
   * * 失敗（400 / 409 / 500 / 繋がらない）は `role="alert"` の段落で
   *
   * # 読み直すのは、開いたときと保存・消去の後だけ
   *
   * **周期で読み直さない**（GET は軽いが、周期にする理由が無い —— 待ち受けの番号は起動の間に変わらず、
   * 保存した番号を変えるのはこの画面か、手で書き換えたファイルだけ）。
   * **保存・消去の後は、その応答を採る**（サーバが書いた後に読み直した状態なので、読み直しの GET を足さない）。
   * 見張り番は `tests/settings-panel.test.ts` の「開いた後、時間が経っても読み直さない」。
   *
   * **ユーティリティと土台のクラスで組む**（色・字・間隔は `<style>` に書かない）。**`<style>` は持たない。**
   */

  import { ApiRequestError, getServer, saveListeningPort, type ServerView } from "./api";
  import { labelFor } from "./router";
  import {
    READ_ONLY_NOTICE,
    UNREADABLE_NOTICE,
    canClear,
    describeSaveFailure,
    describeSaved,
    initialDraft,
    parsePortInput,
    serverRows,
  } from "./serverSettings";

  let view = $state<ServerView | null>(null);
  let loadError = $state<string | null>(null);
  /** 入力欄の文字（**番号に読むのは送るとき**）。 */
  let draft = $state("");
  /** 保存・消去の応答を待っているか。待っている間は押せない。 */
  let busy = $state(false);
  /** 保存・消去の後の文（次に押したら消す）。 */
  let outcome = $state<string | null>(null);
  /** 保存・消去の失敗・入力の誤り（次に押したら消す）。 */
  let failure = $state<string | null>(null);

  /**
   * **この画面が生きている間**の中断ハンドル。破棄されたら落とす
   * （落とさないと、画面を離れた後に応答が着いて、もう無い画面の状態を書き換える）。
   */
  const lifetime = new AbortController();

  $effect(() => {
    return () => {
      lifetime.abort();
    };
  });

  async function load(): Promise<void> {
    loadError = null;
    try {
      const next = await getServer(lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      view = next;
      draft = initialDraft(next);
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      view = null;
      loadError = thrown instanceof ApiRequestError ? thrown.message : "予期しない失敗です";
    }
  }

  /** **送る。** `requested` が `null` なら保存を消す。応答を、いまの状態として採る。 */
  async function send(requested: number | null): Promise<void> {
    busy = true;
    outcome = null;
    failure = null;
    try {
      const next = await saveListeningPort(requested, lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      view = next;
      outcome = describeSaved(requested, next);
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      failure = describeSaveFailure(thrown, requested);
    } finally {
      busy = false;
    }
  }

  function save(event: SubmitEvent): void {
    event.preventDefault();
    const input = parsePortInput(draft);
    if (!input.ok) {
      outcome = null;
      failure = input.message;
      return;
    }
    void send(input.port);
  }

  void load();
</script>

<section>
  <!-- **見出しは `labelFor` から出す**（ナビと同じ綴り。直書きすると写しになる / `UpstreamPanel.svelte` と同じ形）。 -->
  <h1>{labelFor({ route: "settings" })}</h1>
  <p class="note">personacore 自身の設定です。</p>

  <section class="mt-6 flex flex-col gap-4" aria-labelledby="settings-port-heading">
    <h2 id="settings-port-heading" class="m-0">待ち受けのポート</h2>
    <p class="note m-0">
      番号は起動するときに決まります。効く順は、環境変数 PERSONACORE_PORT、ここで保存した番号、既定の番号です。
      保存した番号は次の起動から効きます（いま待ち受けている番号は移りません）。
    </p>
    {#if loadError !== null}
      <p class="error" role="alert">
        設定を取得できませんでした: {loadError}
        <button type="button" onclick={() => void load()}>再試行</button>
      </p>
    {:else if view === null}
      <p class="muted" role="status">読み込み中…</p>
    {:else}
      <dl class="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1.5">
        {#each serverRows(view) as row (row.label)}
          <div class="contents">
            <dt class="text-muted">{row.label}</dt>
            <dd class="m-0 wrap-anywhere">{row.value}</dd>
          </div>
        {/each}
      </dl>
      {#if !view.editable}
        <!-- **保存できない起動では、入力と操作を出さない**（押しても 409 で断られるだけ）。 -->
        <p class="m-0">{READ_ONLY_NOTICE}</p>
      {:else}
        {#if view.saved_port_unreadable}
          <p class="m-0 text-warn">{UNREADABLE_NOTICE}</p>
        {/if}
        <form class="flex flex-col gap-3" onsubmit={save}>
          <label class="block text-sm font-semibold" for="listening-port">保存する番号（1024〜65535）</label>
          <!--
            **`type="number"` と `min` / `max` を付けない** —— 範囲を判定するのはサーバ（`serverSettings.ts` の冒頭）。
            付けると閲覧器が範囲の外を送らせず、断る文がサーバの答えと別の場所に2つできる。
          -->
          <input
            id="listening-port"
            class="max-w-40 font-mono"
            type="text"
            inputmode="numeric"
            autocomplete="off"
            bind:value={draft}
          />
          <div class="actions flex-wrap items-center">
            <button type="submit" class="primary" disabled={busy}>保存</button>
            <button type="button" disabled={busy || !canClear(view)} onclick={() => void send(null)}
              >保存を消して既定に戻す</button
            >
          </div>
        </form>
      {/if}
      {#if busy}
        <p class="muted m-0" role="status">保存しています…</p>
      {/if}
      {#if outcome !== null}
        <p class="notice m-0" role="status">{outcome}</p>
      {/if}
      {#if failure !== null}
        <p class="error m-0" role="alert">{failure}</p>
      {/if}
    {/if}
  </section>
</section>
