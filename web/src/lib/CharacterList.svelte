<script lang="ts">
  import CharacterForm from "./CharacterForm.svelte";
  import CharacterRepairForm from "./CharacterRepairForm.svelte";
  import {
    ApiRequestError,
    deleteCharacter,
    getDrift,
    getStats,
    listCharacters,
    updateCharacter,
    type CharacterListView,
    type CharacterView,
    type StatsView,
  } from "./api";
  import {
    activeCharacters,
    archivedCharacters,
    purgeConfirmed,
  } from "./characterArchive";
  import { describeDeleteFailure, type DeleteFailure } from "./characterDelete";
  import {
    archivedAtFixOf,
    repairTargetOf,
    type RepairableRow,
  } from "./characterRepair";
  import { hrefFor } from "./router";

  /**
   * 1件分の性格ステータスの取得結果。
   *
   * **`null` を「失敗」と「まだ」に使い回さない。** 使い回すと、
   * 取得に失敗した行が「軸が0本のキャラクター」として表示される。
   */
  type StatsState =
    | { kind: "loading" }
    | { kind: "ready"; stats: StatsView }
    | { kind: "failed"; message: string };

  let list = $state<CharacterListView | null>(null);
  let listError = $state<string | null>(null);
  let stats = $state<Record<string, StatsState>>({});

  /**
   * **いま開いているフォーム。**
   *
   * | 値 | 何 |
   * |---|---|
   * | `null` | 開いていない |
   * | `{ kind: "create" }` | 新規作成（`POST`） |
   * | `{ kind: "edit", character }` | その行の編集（`PATCH`） |
   *
   * **`CharacterView` を直に持たない。** 持つと「`null` は作成」と
   * 「`null` は開いていない」が同じ表現になり、**作成のフォームが
   * 開いているのかどうかを区別できない。**
   */
  type Editing =
    | { kind: "create" }
    | { kind: "edit"; character: CharacterView };

  let editing = $state<Editing | null>(null);

  /**
   * **フォームを作り直すための鍵。**
   *
   * `{#key}` に渡す。**識別子だけでは足りない** ——
   * 作成 → 同じ行の編集、と続けたときに値が変わらない組が出る。
   * 見張り番は `tests/character-list-editing.test.ts::
   * 別の行を開くと、前の行の下書きが残らない`。
   */
  const editingKey = $derived(
    editing === null
      ? "none"
      : editing.kind === "create"
        ? "create"
        : `edit:${editing.character.handle}`,
  );

  /**
   * **削除の確認**（作業単位 D-7 / ADR-0016 §2）。
   *
   * | 値 | 何 |
   * |---|---|
   * | `null` | 確認していない（**押しただけでは消えない**） |
   * | `{ character, busy, failure }` | その行を消すか訊いている |
   *
   * **`confirm()` を使わない。** jsdom で観測できず（＝見張り番を置けず）、
   * この画面の作法にも合わない。**画面の中で訊く。**
   *
   * **`failure` をここに持つ**——確認と同じ入れ物に入れておけば、
   * **別の行を開いた瞬間に前の行の失敗が消える**（持ち越すと、
   * A の拒否の理由が B の確認に出たままになる＝帰属の壊れ方）。
   */
  type Deleting = {
    character: CharacterView;
    /** 送っている最中。**二重に押させない。** */
    busy: boolean;
    /** **消せなかった理由。** 読み分けは `characterDelete.ts`。 */
    failure: DeleteFailure | null;
  };

  let deleting = $state<Deleting | null>(null);

  /**
   * **しまった一覧からの「完全に消す」の確認**（作業単位 A-10 / ADR-0049 §決定1）。
   *
   * **`deleting` と別に持つ。** 上の一覧の「削除」は会話の無いキャラクターしか通らない確認で、
   * こちらは**会話・記憶ごと消える**確認。同じ入れ物にすると、
   * 識別子を打ち込ませる確認が「押すだけの確認」に化ける形が作れる。
   *
   * `typed` は打ち込んだ識別子。**一致するまで「完全に消す」は押せない**（`purgeConfirmed`）。
   */
  type Purging = {
    character: CharacterView;
    typed: string;
    busy: boolean;
    failure: DeleteFailure | null;
  };

  let purging = $state<Purging | null>(null);

  /**
   * **しまう／戻すの失敗**（どの行の失敗かを持つ —— 別の行に出すと帰属が壊れる）。
   * 送っている最中の行も持ち、**二重に押させない。**
   */
  let archiveBusy = $state<string | null>(null);
  let archiveFailure = $state<{ handle: string; message: string } | null>(
    null,
  );

  /** 上の一覧（しまっていないキャラクター）。 */
  const active = $derived(list === null ? [] : activeCharacters(list.characters));
  /** しまった一覧。 */
  const archived = $derived(
    list === null ? [] : archivedCharacters(list.characters),
  );

  /**
   * **いま直している壊れた行**（作業単位 D-6 / ADR-0016 §3・§4）。
   *
   * **`editing` と別に持つ。** まとめると
   * **「読み出せた行の編集」と「1つも読めない行の修復」が同じ表現**になり、
   * **差分モードと全欄モードのどちらを送るのかが、値からは決まらなくなる。**
   *
   * **⚠ 「識別子が読めない行はこの状態を作れない」とは書けない。**
   * **型では塞げていない**（`RepairableRow` の doc / D-6 のレビューで実測 ——
   * `handle: broken.handle ?? ""` は `pnpm run check` を 0 エラーで通る）。
   * **守っているのは `repairTargetOf` の `null` ガードと、それを見るテスト。**
   */
  let repairing = $state<RepairableRow | null>(null);

  /**
   * **修復フォームを作り直すための鍵**（`editingKey` と同じ理由）。
   *
   * 作り直さないと、**前の行に書きかけたペルソナが別の行へ書き込まれる**
   * ——しかも見出しは新しい行を名乗る（帰属の壊れ方）。
   * 見張り番は `tests/character-list-repair.test.ts::
   * 別の行を開くと、前の行の下書きが残らない`。
   */
  const repairingKey = $derived(
    repairing === null ? "none" : `repair:${repairing.id}`,
  );

  function messageOf(error: unknown): string {
    if (error instanceof ApiRequestError) {
      return error.message;
    }
    return "予期しない失敗です";
  }

  async function loadStats(handle: string): Promise<void> {
    stats = { ...stats, [handle]: { kind: "loading" } };
    try {
      const loaded = await getStats(handle);
      stats = { ...stats, [handle]: { kind: "ready", stats: loaded } };
    } catch (error) {
      stats = {
        ...stats,
        [handle]: { kind: "failed", message: messageOf(error) },
      };
    }
  }

  async function load(): Promise<void> {
    listError = null;
    try {
      const loaded = await listCharacters();
      list = loaded;
      // **1件ずつ独立に取る。** 1件の失敗で一覧全体を落とさない
      // （落とすと、壊れた行が1つあるだけで修復の入口が閉じる）。
      // **しまったキャラクターの性格値は取らない**（しまった一覧には出さない）。
      await Promise.all(
        activeCharacters(loaded.characters).map((character) =>
          loadStats(character.handle),
        ),
      );
    } catch (error) {
      list = null;
      listError = messageOf(error);
    }
  }

  /** 確認を開く。**必ず作り直す**（前の行の失敗を持ち越さない）。 */
  function askDelete(character: CharacterView): void {
    deleting = { character, busy: false, failure: null };
  }

  function cancelDelete(): void {
    deleting = null;
  }

  /**
   * 実際に消す。
   *
   * **消せるかどうかを事前に確かめない。** 判定は `store` が削除と同じ文で
   * 不可分に行うので（`admin.rs` の `delete_character` の doc）、
   * **押してみるまで分からない。** ここで会話の有無を数えに行かないこと。
   *
   * 成功したら**一覧を取り直す。** 画面の側で行を落とすと、
   * 「消えたように見えるが、実は消えていない」形を作れてしまう。
   */
  async function confirmDelete(): Promise<void> {
    const current = deleting;
    if (current === null || current.busy) {
      return;
    }
    deleting = { ...current, busy: true, failure: null };
    try {
      await deleteCharacter(current.character.handle);
    } catch (failure) {
      deleting = {
        ...current,
        busy: false,
        failure: describeDeleteFailure(failure),
      };
      return;
    }
    deleting = null;
    await load();
  }

  /**
   * **しまう（`true`）／戻す（`false`）**（作業単位 A-10 / ADR-0049 §決定1）。
   *
   * **確認を挟まない** —— しまっても中身は1行も変わらず、しまった一覧から戻せる。
   * 成功したら一覧を取り直す（画面の側で行を動かすと、動いたように見えて実は動いていない形が作れる）。
   */
  async function setArchived(handle: string, archive: boolean): Promise<void> {
    if (archiveBusy !== null) {
      return;
    }
    archiveBusy = handle;
    archiveFailure = null;
    try {
      await updateCharacter(handle, { archived: archive });
    } catch (failure) {
      archiveFailure = { handle, message: messageOf(failure) };
      archiveBusy = null;
      return;
    }
    archiveBusy = null;
    // **開いている確認がその行のものなら閉じる**（戻した行を「完全に消す」確認が残らないように）。
    if (purging?.character.handle === handle) {
      purging = null;
    }
    if (deleting?.character.handle === handle) {
      deleting = null;
    }
    await load();
  }

  /** 完全に消す確認を開く。**必ず作り直す**（前の行の打ち込みと失敗を持ち越さない）。 */
  function askPurge(character: CharacterView): void {
    purging = { character, typed: "", busy: false, failure: null };
  }

  function cancelPurge(): void {
    purging = null;
  }

  /**
   * 実際に完全に消す。**識別子が一致しないうちは送らない**（ボタンが押せなくても、ここでも確かめる）。
   *
   * 消せるかどうかの判定はサーバが削除と同じ文で行う（戻された直後なら 409 で断られる）。
   */
  async function confirmPurge(): Promise<void> {
    const current = purging;
    if (
      current === null ||
      current.busy ||
      !purgeConfirmed(current.character.handle, current.typed)
    ) {
      return;
    }
    purging = { ...current, busy: true, failure: null };
    try {
      await deleteCharacter(current.character.handle);
    } catch (failure) {
      purging = {
        ...current,
        busy: false,
        failure: describeDeleteFailure(failure),
      };
      return;
    }
    purging = null;
    await load();
  }

  void load();
</script>

<section>
  <header>
    <h1>キャラクター</h1>
    <button type="button" onclick={() => void load()}>再読み込み</button>
    <button type="button" onclick={() => (editing = { kind: "create" })}>
      新規作成
    </button>
  </header>

  <!--
    **`{#key}` で作り直す。** 開く行が変わったら下書きも作り直さないと、
    **前の行の本文を別のキャラクターへ書き込む**（`CharacterForm` の `draft` の doc）。
  -->
  {#key editingKey}
    {#if editing !== null}
      <CharacterForm
        character={editing.kind === "edit" ? editing.character : null}
        loadDefaultRunMode={(handle) =>
          // **起動時の既定**（作業単位 D-15）。**`run_mode` ではない** ——
          // キャラクターが値を持つと `run_mode` はその値になり、
          // 「既定に従うと、いまは何になるか」を言えなくなる。
          getDrift(handle).then((drift) => drift.default_run_mode)}
        onSaved={() => {
          // **一覧を読み直す。** 作成した行が出ないと「作れていない」に見える。
          // **フォームは閉じない**（作成の直後は編集が続く / `CharacterForm` の `saved`）。
          void load();
        }}
        onClose={() => (editing = null)}
      />
    {/if}
  {/key}

  <!--
    **壊れた行の修復**（作業単位 D-6 / ADR-0016 §3・§4）。
    **一覧の外に置く** —— 読み直しに失敗して `list` が `null` になっても、
    書きかけの修復が画面ごと消えないように。
    **`{#key}` で作り直す**（`repairingKey` の doc）。
  -->
  {#key repairingKey}
    {#if repairing !== null}
      <CharacterRepairForm
        row={repairing}
        onRepaired={() => {
          // **読み直せなかった回も取り直す。** 更新は起きているので、
          // `unreadable` の側が変わっているかもしれない。
          // **フォームは閉じない**——まだ直す列が残っていることがある。
          void load();
        }}
        onClose={() => (repairing = null)}
      />
    {/if}
  {/key}

  <!--
    **削除の確認**（作業単位 D-7）。**`confirm()` は使わない**（`Deleting` の doc）。
    **押した行の識別子を名乗る**（`data-confirm-delete`）ので、
    どの行のことを訊かれているのかが、見る側にも見張り番にも分かる。
  -->
  {#if deleting !== null}
    <div class="confirm" data-confirm-delete={deleting.character.handle}>
      <p>
        <strong>{deleting.character.display_name}</strong>（<code
          >{deleting.character.handle}</code
        >）を削除します。<strong>取り消せません。</strong>
      </p>
      <!--
        **事前に確かめられないことを、隠さずに書く**（ADR-0016 §2）。
        「消せるか調べてから押させる」画面は原理的に作れない。
      -->
      <p class="muted">
        会話が1件でもあるキャラクターは、しまってからでないとサーバが拒否します。
        消せるかどうかは、押してみるまで分かりません。
      </p>
      <button
        type="button"
        disabled={deleting.busy}
        onclick={() => void confirmDelete()}
      >
        削除する
      </button>
      <button type="button" disabled={deleting.busy} onclick={cancelDelete}>
        やめる
      </button>

      {#if deleting.failure !== null}
        <!--
          **サーバが言ったことをそのまま出す**（`chat.ts` の `outcomeNotice`
          と同じ立場 —— こちらから足さない）。
        -->
        <p class="error" role="alert" data-delete-reason={deleting.failure.reason}>
          {deleting.failure.message}
        </p>
        {#if deleting.failure.guidance !== null}
          <!--
            **次に取れる行動。** 分類できなかった失敗には出ない（`null`）——
            **推測で埋めない。**
          -->
          <p class="guidance" data-delete-guidance="1">
            {deleting.failure.guidance}
          </p>
        {/if}
      {/if}
    </div>
  {/if}

  {#if listError !== null}
    <p class="error" role="alert">一覧を取得できませんでした: {listError}</p>
  {:else if list === null}
    <p role="status">読み込み中…</p>
  {:else}
    {#if list.characters.length === 0}
      <p>キャラクターがまだ1件もありません。</p>
    {:else if active.length === 0}
      <!--
        **しまったキャラクターしか居ない**（作業単位 A-10）。「1件もありません」と言わない ——
        下のしまった一覧に居る。
      -->
      <p>しまっていないキャラクターはありません（しまったキャラクターは下の一覧にあります）。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>識別子</th>
            <th>表示名</th>
            <th>尺度</th>
            <th>性格値</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <!--
            **しまったキャラクターはここに出さない**（作業単位 A-10 / 出し分けは `characterArchive.ts`）。
            **行が識別子を名乗る**（`data-active-character`）ので、並び順に寄りかからずに指せる。
          -->
          {#each active as character (character.id)}
            <tr data-active-character={character.handle}>
              <td><code>{character.handle}</code></td>
              <td>{character.display_name}</td>
              <td>{character.stat_scale}</td>
              <td>
                {#if stats[character.handle]?.kind === "loading"}
                  <span class="muted" role="status">読み込み中…</span>
                {:else if stats[character.handle]?.kind === "failed"}
                  <span class="error">
                    {(stats[character.handle] as { message: string }).message}
                  </span>
                {:else if stats[character.handle]?.kind === "ready"}
                  {@const ready = stats[character.handle] as {
                    stats: StatsView;
                  }}
                  {#if ready.stats.stats.length === 0}
                    <span class="muted">未設定</span>
                  {:else}
                    <ul class="stats">
                      {#each ready.stats.stats as alloc (alloc.axis)}
                        <li>
                          {alloc.axis}: {alloc.value} / {ready.stats.stat_scale}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                {/if}
              </td>
              <td>
                <!--
                  **この行のキャラクターを渡す。** 一覧の行と送り先が
                  ここで結び付く（`tests/character-list-editing.test.ts`）。
                -->
                <button
                  type="button"
                  onclick={() => (editing = { kind: "edit", character })}
                >
                  編集
                </button>
                <!--
                  **この行のキャラクターの記憶へ**（ADR-0022 の修復の入口 /
                  作業単位 D-3）。**リンク先は経路が組み立てる**
                  （`router.ts` の `hrefFor` —— 綴りの写しを増やさない）。
                -->
                <a href={hrefFor({ route: "memories", handle: character.handle })}>
                  記憶
                </a>
                <!--
                  **この行のキャラクターの性格値と変容へ**（作業単位 D-4）。
                  **ここが「性格値を下げる」唯一の入口**である
                  ——`CharacterForm` の尺度のヒントが名指ししているのはこの導線
                  （B-15 / 見張り番は `tests/stat-scale-path.test.ts`）。
                -->
                <a href={hrefFor({ route: "stats", handle: character.handle })}>
                  性格値と変容
                </a>
                <!--
                  **この行のキャラクターの属性と変更の記録へ**（作業単位 D-12a）。
                  **ここが「属性を直す」唯一の入口**である
                  ——属性は削れない枠に毎ターン載るので、
                  古くなった値を直す道が画面に無いと `curl` しか無い。
                -->
                <a
                  href={hrefFor({ route: "attributes", handle: character.handle })}
                >
                  属性
                </a>
                <!--
                  **この行のキャラクターの約束・決まりへ**（作業単位 A-12a / ADR-0051）。
                  **ここが「約束・決まりを書く・終わらせる」唯一の入口**である。
                -->
                <a
                  href={hrefFor({ route: "agreements", handle: character.handle })}
                >
                  約束・決まり
                </a>
                <!--
                  **この行のキャラクターを渡す**（作業単位 D-7）。
                  **ここでは消さない** —— 確認を開くだけ（`Deleting` の doc）。
                -->
                <button type="button" onclick={() => askDelete(character)}>
                  削除
                </button>
                <!--
                  **しまう**（作業単位 A-10）。**確認を挟まない** —— 中身は変わらず、下の一覧から戻せる。
                -->
                <button
                  type="button"
                  disabled={archiveBusy !== null}
                  onclick={() => void setArchived(character.handle, true)}
                >
                  しまう
                </button>
                {#if archiveFailure !== null && archiveFailure.handle === character.handle}
                  <p class="error" role="alert" data-archive-failure={character.handle}>
                    しまえませんでした: {archiveFailure.message}
                  </p>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      **しまったキャラクター**（作業単位 A-10 / ADR-0049 §決定1）。
      **0件でも節ごと消さない**（読み出せなかった行と同じ理由）。
      **完全に消す口はここにしか置かない** —— 完全に消す前に必ず「しまう」を通る。
    -->
    <h2>しまったキャラクター（{archived.length} 件）</h2>
    <p class="muted">
      しまったキャラクターは会話の相手の一覧に出ず、話しかけることもできません。
      会話・記憶・性格値はそのまま残っていて、戻せば元どおりになります。
    </p>

    {#if purging !== null}
      <!--
        **完全に消す確認。** **識別子を打ち込ませる**（取り消しが効かないので、押すだけの確認にしない）。
        **押した行の識別子を名乗る**（`data-confirm-purge`）。
      -->
      <div class="confirm" data-confirm-purge={purging.character.handle}>
        <p>
          <strong>{purging.character.display_name}</strong>（<code
            >{purging.character.handle}</code
          >）を完全に消します。会話・記憶・性格値・属性・約束と決まりもすべて消え、<strong
            >取り消せません。</strong
          >
        </p>
        <label>
          確認のため、識別子 <code>{purging.character.handle}</code> を入力してください:
          <input
            type="text"
            autocomplete="off"
            spellcheck="false"
            value={purging.typed}
            disabled={purging.busy}
            oninput={(event) => {
              if (purging !== null) {
                purging = {
                  ...purging,
                  typed: event.currentTarget.value,
                  failure: null,
                };
              }
            }}
          />
        </label>
        <button
          type="button"
          disabled={purging.busy ||
            !purgeConfirmed(purging.character.handle, purging.typed)}
          onclick={() => void confirmPurge()}
        >
          完全に消す
        </button>
        <button type="button" disabled={purging.busy} onclick={cancelPurge}>
          やめる
        </button>

        {#if purging.failure !== null}
          <p class="error" role="alert" data-purge-reason={purging.failure.reason}>
            {purging.failure.message}
          </p>
          {#if purging.failure.guidance !== null}
            <p class="guidance" data-purge-guidance="1">
              {purging.failure.guidance}
            </p>
          {/if}
        {/if}
      </div>
    {/if}

    {#if archived.length === 0}
      <p class="muted">ありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>識別子</th>
            <th>表示名</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each archived as character (character.id)}
            <tr data-archived-character={character.handle}>
              <td><code>{character.handle}</code></td>
              <td>{character.display_name}</td>
              <td>
                <!--
                  **中を見る導線**（作業単位 A-10 のレビュー / ADR-0049 §決定1「中を見てから消すかを
                  決められるように」）。**上の一覧と同じ4つ・同じ綴り・同じ組み立て**（`hrefFor`）。
                  管理の読み取りの口は、しまったキャラクターにも答える。
                -->
                <a href={hrefFor({ route: "memories", handle: character.handle })}>
                  記憶
                </a>
                <a href={hrefFor({ route: "stats", handle: character.handle })}>
                  性格値と変容
                </a>
                <a
                  href={hrefFor({ route: "attributes", handle: character.handle })}
                >
                  属性
                </a>
                <a
                  href={hrefFor({ route: "agreements", handle: character.handle })}
                >
                  約束・決まり
                </a>
                <!-- **この行のキャラクターを渡す**（一覧の行と送り先がここで結び付く）。 -->
                <button
                  type="button"
                  disabled={archiveBusy !== null}
                  onclick={() => void setArchived(character.handle, false)}
                >
                  戻す
                </button>
                <!-- **ここでは消さない** —— 確認を開くだけ（`Purging` の doc）。 -->
                <button type="button" onclick={() => askPurge(character)}>
                  完全に消す
                </button>
                {#if archiveFailure !== null && archiveFailure.handle === character.handle}
                  <p class="error" role="alert" data-archive-failure={character.handle}>
                    戻せませんでした: {archiveFailure.message}
                  </p>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    <!--
      **読み出せなかった行**（ADR-0016 §3）。
      **0件でも節ごと消さない**——「壊れた行が無い」ことと
      「この画面が壊れた行を見せない古い版である」ことを、見る側が区別できるように。
    -->
    <h2>読み出せなかった行（{list.unreadable.length} 件）</h2>
    {#if list.unreadable.length === 0}
      <p class="muted">ありません。</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>id</th>
            <th>識別子</th>
            <th>読めなかった列</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          {#each list.unreadable as broken (broken.id)}
            <!--
              **直せる行かどうかは、純粋関数が決める**（`characterRepair.ts`）。
              **`null` なら入口を出さない** —— 識別子が読めない行は
              `PATCH` でも `DELETE` でも指せない。
            -->
            {@const repairable = repairTargetOf(broken)}
            {@const archivedAtFix = archivedAtFixOf(broken)}
            <!--
              **id を行に持たせる**（並び順に寄りかからずに、その行を指せるように）。
            -->
            <tr data-unreadable-id={broken.id}>
              <td>{broken.id}</td>
              <td>
                {#if broken.handle === null}
                  <!--
                    **⚠ 事実だけを言う**（作業単位 D-6 / D-7 と同じ立場）。
                    この行は `PATCH` でも `DELETE` でも指せない
                    ——識別子は不変（ADR-0016 §1）なので、
                    **直す手段は `sqlite3` だけ**（`domain::UnreadableCharacter` の doc）。
                    **「DB を直せ」を操作として案内しない** ——
                    画面から辿れる導線ではないので、状態の説明にとどめる。
                  -->
                  <span class="error" data-unrepairable="1">
                    識別子そのものが壊れており、API からは指せません（この行を直せるのは
                    DB を直接見るときだけです）。
                  </span>
                {:else}
                  <code>{broken.handle}</code>
                {/if}
              </td>
              <!--
                **生の列名をそのまま出す**（DB を直接見に行くときの手掛かり）。
                **行ごとに引く**——決め打つと、どの行が何で壊れたかを画面が嘘で言う。
              -->
              <td
                ><code data-invalid-columns={broken.invalid_columns.join(" ")}
                  >{broken.invalid_columns.join(", ")}</code
                ></td
              >
              <td>
                {#if repairable !== null}
                  <!--
                    **この行を渡す**（一覧の行と送り先がここで結び付く /
                    `tests/character-list-repair.test.ts`）。
                    **ここでは送らない** —— フォームを開くだけ。
                  -->
                  <button type="button" onclick={() => (repairing = repairable)}>
                    修復
                  </button>
                {/if}
                <!--
                  **しまった時刻が読めない行**（作業単位 A-10）。この行はしまった一覧にも出ないので
                  「戻す」が届かない。**直す口（時刻を消す＝戻す）をここに出す。**
                  **判定は `archivedAtFixOf`**（識別子が読めない行には出ない）。
                -->
                {#if archivedAtFix !== null}
                  <button
                    type="button"
                    disabled={archiveBusy !== null}
                    data-clear-archived-at={archivedAtFix}
                    onclick={() => void setArchived(archivedAtFix, false)}
                  >
                    しまった時刻を消す
                  </button>
                  <p class="muted">
                    しまった時刻が読めません。消すと、しまっていない状態に戻ります（あとでもう一度しまえます）。
                  </p>
                  {#if archiveFailure !== null && archiveFailure.handle === archivedAtFix}
                    <p class="error" role="alert" data-archive-failure={archivedAtFix}>
                      消せませんでした: {archiveFailure.message}
                    </p>
                  {/if}
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</section>

<style>
  /* **表・見出しの帯・色は `src/app.css` にある**（作業単位 D-10）。 */

  ul.stats {
    margin: 0;
    padding-left: 1.1rem;
  }

  /* **戻せない操作の確認。** 枠が「壊れている側」の色を持つ。 */
  .confirm {
    border: 1px solid var(--color-warn);
    padding: 0.75rem 1rem;
    margin: 1rem 0;
  }

  /* **確認の箱の中では、1文ぶん離す**（土台の既定より広い）。 */
  .guidance {
    margin: 0.5rem 0 0;
  }
</style>
