<script lang="ts">
  /**
   * **性格値と変容の画面**（作業単位 D-4 / ADR-0015 / ADR-0024 §9）。
   *
   * # なぜこの画面が要るのか
   *
   * **変容は本文にもユーザーにも見せない**（ADR-0024 §9）。
   * **つまり、動いたことが分かる場所がログしか無かった。ログは流れる。**
   * そして **`GET .../stats` が返すのは「いまの値」だけ**なので、
   * **変容したのか、人が `PUT .../stats` で設定したのかを区別できない**
   * ——これが D-4a（`GET .../drift`）を足した理由そのもの。
   *
   * **したがって、この画面は変容の側を必ず出す** ——
   * 軸ごとの**累積**・**上げ下げの件数**・**直近の票**・**票の総数**。
   * 出さなければ、画面があっても D-4 の目的は1ミリも果たされない。
   *
   * # ⚠ **`PUT .../stats` は丸ごと置き換えである**
   *
   * **要求に含まれない軸は消える。空の配列を渡すと全軸が消える**
   * （`admin.rs` の `replace_stats`）。
   * **「変えない」を表す手段が無い**ので、**画面は必ず全軸を送る**
   * （`statsEdit.ts` の `buildReplace` / `characterRepair.ts` と同じ作法）。
   *
   * **軸を消す手段は「送らないこと」しかない。**
   * だから「外す」は**押した時点では何も起きず、保存したときに消える。**
   * **何が残って何が消えるかは、押す前に `replaceSummary` が言う。**
   *
   * # 2つの読みは別々（**片方が落ちても、もう片方は出す**）
   *
   * `GET .../stats` と `GET .../drift` は別の要求で、
   * `get_drift` の中の5つの読みすら1つのトランザクションで囲っていない
   * （`admin.rs` の `get_drift` の doc）。
   * **したがって、いまの値と変容の表の値はずれることがある。**
   * **どちらの数字がどちらの応答のものかを、混ぜないこと。**
   *
   * **失敗も別々に持つ** —— 片方の失敗で画面ごと落とすと、
   * **変容が読めないだけで置き換えの入口まで消える。**
   *
   * # 書き込みの口は1本だけ
   *
   * 変容の記録は**読むだけ**（票は追記のみ / `admin.rs` の `get_drift`）。
   * **票を消す・直す操作をこの画面に生やさないこと。** サーバにも経路が無い。
   *
   * # いまの値が読めないときも、直せる（**作業単位 D-8**）
   *
   * **`GET .../stats` は 500 になりうる**（尺度を超える値・孤児・範囲外・
   * **正規化すると同じになる軸の重複** / 作業単位 B-19）。
   * **その回に画面へ残っていたのは「再読み込み」と「キャラクターの一覧へ」だけ**で、
   * **直すには `curl` か `sqlite3` が要った。**
   *
   * **`PUT .../stats` は既存の軸を読まないので、その状態でも通る**
   * （`tests/server_admin.rs` で実測）。**閉じていたのは画面だけ。**
   * **失敗の枝に修復のフォームを置く**（`StatsRepairForm.svelte`）。
   *
   * **⚠ 本物の壊れ方では、変容の側も一緒に落ちる**
   * （`get_drift` も `Store::stat_allocs` を読む / 同テストで実測）。
   * **だから修復の入口を「変容が読めたとき」に結び付けないこと。**
   * 結び付けると、**本物の壊れ方では一度も出ない。**
   */

  import {
    ApiRequestError,
    getDrift,
    getStats,
    listCharacters,
    replaceStats,
    type DriftView,
    type StatsView,
  } from "./api";
  import StatsRepairForm from "./StatsRepairForm.svelte";
  import {
    describeStatsFailure,
    repairEntryFor,
    scaleFromListing,
    type ScaleLookup,
    type StatsReadFailure,
  } from "./statsRepair";
  import {
    axisValueLabel,
    buildReplace,
    chargeTimeLabel,
    deltaLabel,
    describeReplaceFailure,
    draftOfStats,
    emptyRow,
    noAxesNotice,
    replaceSummary,
    runModeLabel,
    runModeNotice,
    runModeSourceNote,
    CHARACTER_LIST_LINK_LABEL,
    unallocatedNotice,
    type ReplaceFailure,
    type StatsDraft,
  } from "./statsEdit";
  import { hrefFor, labelFor } from "./router";
  // **時刻の綴りは1箇所**（作業単位 F-7 / 記憶の一覧と同じ関数）。
  import { browserZone, timeLabel } from "./timestamp";

  interface Props {
    /** どのキャラクターのものか。**経路が運ぶ**（`router.ts` の `Screen`）。 */
    handle: string;
  }

  let { handle }: Props = $props();

  /**
   * **読み手の時間帯**（作業単位 F-7b）。
   *
   * **ここで解決して `timeLabel` / `chargeTimeLabel` へ渡す** ——
   * **あちらに自分で決めさせない。**
   * 決めさせると、**あの関数を見る検査が、走らせた機械の時間帯でしか
   * 成立しなくなる**（F-7 が地域化を避けた2つ目の理由）。
   *
   * **⚠ 綴りを直接書かないこと**（`"Asia/Tokyo"` 等）。
   * 書くと、**開発者の機械では正しく見えて、他の誰の画面でも時刻がずれる。**
   * **この表は「上げ／下げの比を読める唯一の場所」**（ADR-0024 §9 の下）なので、
   * ずれても本文には何も出ない。見張り番は `tests/timestamp-wiring.test.ts`。
   */
  const zone = browserZone();

  /**
   * 取得の結果。
   *
   * **`null` を「失敗」と「まだ」に使い回さない。** 使い回すと、
   * **取得に失敗したキャラクターが「軸が0本」「票が0件」に見える**
   * ——**変容が起きていないことと、読めなかったことは別**である
   * （`CharacterList` の `StatsState` と同じ立場）。
   */
  type Loaded<T> =
    | { kind: "loading" }
    | { kind: "ready"; value: T }
    | { kind: "failed"; message: string };

  /**
   * いまの値の取得。**失敗は `StatsReadFailure` として持つ**（作業単位 D-8）。
   *
   * **文言だけでは、修復の入口を出してよいかが決まらない。**
   * サーバへ届いていない回や、そのキャラクターが居ない回は、
   * **押せば必ず失敗するボタン**にしかならない（`statsRepair.ts` の `repairEntryFor`）。
   */
  type StatsLoaded =
    | { kind: "loading" }
    | { kind: "ready"; value: StatsView }
    | { kind: "failed"; failure: StatsReadFailure };

  let statsState = $state<StatsLoaded>({ kind: "loading" });
  let driftState = $state<Loaded<DriftView>>({ kind: "loading" });

  /**
   * 尺度の引き当て。**`null` は「まだ引いていない」。**
   *
   * **いまの値が読めなかったときにだけ引く**（通常の回は応答に尺度が入っている）。
   * **出どころは一覧**——1件を指す `GET` は存在せず、変容の応答も一緒に落ちている
   * （`statsRepair.ts` の `ScaleLookup` の doc / `tests/server_admin.rs` で実測）。
   */
  let scaleState = $state<ScaleLookup | null>(null);

  /**
   * 置き換えの下書き。**いまの値を読み終えてから作る。**
   *
   * **`load()` のたびに作り直す** —— 打ちかけを残すと、
   * **別の窓で変わった値の上に古い写しを書き戻す**ことになる。
   */
  let draft = $state<StatsDraft | null>(null);

  let saving = $state(false);
  let notice = $state<string | null>(null);
  /** **置き換えが拒否された理由。** 読み分けは `statsEdit.ts`。 */
  let failure = $state<ReplaceFailure | null>(null);

  /** 送る本文。**組み立ては純粋関数**（`statsEdit.ts`）。 */
  const built = $derived(draft === null ? null : buildReplace(draft));

  const canSubmit = $derived(built !== null && built.ok && !saving);

  /**
   * 画面が使う値。**`draft` を直に読ませない。**
   *
   * `draft` は `null` を取りうるので、テンプレートの各所で絞り込みが要る
   * ——**絞り込みは分岐や閉包をまたぐと外れる**ので、ここで1回だけ畳む。
   */
  const rows = $derived(draft?.rows ?? []);
  /**
   * **置き換えのフォームが描かれているか**（作業単位 B-16 の是正）。
   *
   * 下の「変容の記録」に出す案内は **「軸を追加」** と
   * **「この内容で置き換える」** を名指しするが、
   * **その2つはこのフォームの中にしか無い。**
   * **`GET .../stats` が失敗すると、案内だけが残って操作が消える**
   * （レビューの実測 —— そのとき画面にあるのは「再読み込み」と
   * 「キャラクターの一覧へ」の2つだけだった）。
   *
   * **条件はテンプレートの `{:else}` と同じ式にしてある。**
   * 片方だけ変えると、また案内と操作がずれる。
   */
  const canReplace = $derived(statsState.kind === "ready" && draft !== null);
  /**
   * **失敗の枝に出すもの**（作業単位 D-8）。**判定は純粋関数。**
   *
   * **⚠ `canReplace` を、これで置き換えないこと。**
   * 修復のフォームがあっても、**捨て軸の案内が言う手順はこの状態では成立しない** ——
   * 手順は「同じ綴りで足して置き換える」だが、
   * **いま保存されている他の軸が読めない**ので、そのとおりにすると**残りが全部消える。**
   * **辿れる操作があることと、その手順が正しいことは別**である。
   */
  const repairEntry = $derived(
    statsState.kind === "failed"
      ? repairEntryFor(statsState.failure, scaleState)
      : null,
  );
  /** 割り当てを失った軸の案内（無ければ `null`）。**組み立ては純粋関数。** */
  const lostNotice = $derived(
    driftState.kind === "ready"
      ? unallocatedNotice(driftState.value, canReplace)
      : null,
  );
  /**
   * **変容が止まっているときの案内**（無ければ `null` / 作業単位 D-5a / D-15）。
   *
   * **判定は純粋関数**（`runModeNotice`）。**出どころで言い分ける**（D-15）。
   * **`canReplace` を渡さない** —— この案内が名指しするのは
   * **ヘッダの「キャラクターの一覧へ」だけ**で、それは読み込みの成否によらず描かれる
   * （置き換えのフォームの中の操作は名指ししない）。
   */
  const stoppedNotice = $derived(
    driftState.kind === "ready"
      ? runModeNotice(driftState.value.run_mode, driftState.value.run_mode_source)
      : null,
  );
  /**
   * **軸が1本も無いときの案内**（無ければ `null` / 作業単位 F-6）。
   *
   * **判定は純粋関数**（`noAxesNotice`）。
   * **`canReplace` を渡す** —— 直し方の2手（「軸を追加」「この内容で置き換える」）は
   * **置き換えのフォームの中にしか無い**ので、
   * 描かれていない回は名指ししない（`unallocatedNotice` と同じ作法）。
   *
   * **⚠ 停止中の案内と両方出ることがある。** 2つは**別のことを言っている** ——
   * **軸を足しても `off` のままなら動かないし、on にしても軸が無ければ走らない。**
   * 片方に畳むと、もう片方の理由が読めなくなる。
   */
  const noAxes = $derived(
    driftState.kind === "ready"
      ? noAxesNotice(driftState.value, canReplace)
      : null,
  );
  /** 保存すると何が残り、何が消えるか（`statsEdit.ts`）。 */
  const summary = $derived(draft === null ? "" : replaceSummary(draft));

  function messageOf(error: unknown): string {
    if (error instanceof ApiRequestError) {
      return error.message;
    }
    return "予期しない失敗です";
  }

  /**
   * いまの値と変容を読む。**2本は独立に扱う。**
   *
   * 片方の失敗でもう片方を落とさない（`CharacterList` の `loadStats` と同じ理由）。
   */
  async function load(): Promise<void> {
    statsState = { kind: "loading" };
    driftState = { kind: "loading" };
    scaleState = null;
    await Promise.all([loadStats(), loadDrift()]);
  }

  async function loadStats(): Promise<void> {
    try {
      const value = await getStats(handle);
      statsState = { kind: "ready", value };
      draft = draftOfStats(value);
    } catch (error) {
      const failure = describeStatsFailure(error);
      statsState = { kind: "failed", failure };
      draft = null;
      // **修復の入口を出しうる回だけ、尺度を引きに行く**（作業単位 D-8）。
      // 届いていない回・居ない回は、引いても使い道が無い（`repairEntryFor`）。
      if (failure.kind === "server" || failure.kind === "unknown") {
        await loadScale();
      }
    }
  }

  /**
   * **尺度を一覧から引く。**
   *
   * **読めなくても失敗にしない** —— 上端が分からないだけで、**修復はできる**
   * （超えていればサーバが `stat_value_above_scale` を返す）。
   *
   * # ⚠ `catch` を「何でも飲む器」にしない（レビューの指摘 / 2026-08-28）
   *
   * **応答の形の食い違いは、ここへは来ない** ——
   * `scaleFromListing` が**全域関数**で、`{ kind: "malformed" }` という**値**を返す
   * （`statsRepair.ts` 冒頭の ⚠）。**握り潰していたのはその経路だった。**
   *
   * **ここに残るのは「要求そのものが失敗した」回だけ**（届かない・500・JSON にならない ——
   * `api.ts` はどれも `ApiRequestError` にする）。
   * **それ以外が来たら、それはこちら側の壊れ**なので、
   * **「尺度が読めない」と同じ結末にせず `malformed` に寄せる**
   * ——**画面には別の文が出るので、埋もれない。**
   *
   * **再送出はしない。** すると描画の途中で落ちうる（画面を壊さないこと）。
   */
  async function loadScale(): Promise<void> {
    try {
      scaleState = scaleFromListing(await listCharacters(), handle);
    } catch (error) {
      scaleState =
        error instanceof ApiRequestError
          ? { kind: "unknown" }
          : { kind: "malformed" };
    }
  }

  async function loadDrift(): Promise<void> {
    try {
      driftState = { kind: "ready", value: await getDrift(handle) };
    } catch (error) {
      driftState = { kind: "failed", message: messageOf(error) };
    }
  }

  /** 軸を1本足す。**送るのは保存のときだけ**（ここでは何も起きない）。 */
  function addRow(): void {
    if (draft === null) {
      return;
    }
    draft.rows.push(emptyRow());
  }

  /**
   * その軸を「外す」／「戻す」。
   *
   * **押しただけでは何も起きない。** 外した軸は**本文に載らなくなる**だけで、
   * 消えるのは保存したときである（丸ごと置き換えなので、それが「消す」）。
   */
  function toggleRemoved(index: number): void {
    const row = draft?.rows[index];
    if (row === undefined) {
      return;
    }
    row.removed = !row.removed;
  }

  /** 値の欄。**`bind:` を使わない** —— 索引の型を絞る場所を1つに保つ。 */
  function setValue(index: number, value: string): void {
    const row = draft?.rows[index];
    if (row === undefined) {
      return;
    }
    row.value = value;
  }

  /** 軸名の欄（**足した行だけ**）。 */
  function setAxis(index: number, axis: string): void {
    const row = draft?.rows[index];
    if (row === undefined) {
      return;
    }
    row.axis = axis;
  }

  /** 見えていない票の数。**総数と、出した件数の差。** */
  function hiddenVotes(drift: DriftView): number {
    return Math.max(0, drift.total_votes - drift.recent_votes.length);
  }

  async function save(): Promise<void> {
    const request = built;
    if (!canSubmit || request === null || !request.ok) {
      return;
    }
    saving = true;
    notice = null;
    failure = null;
    try {
      await replaceStats(handle, request.body);
      // **読み直す。** 応答も「書いた後に読み直した姿」だが、
      // **変容の側は別の入口**なので、両方取り直さないと片方だけが古びる。
      notice = "性格値を置き換えました。";
      await load();
    } catch (error) {
      failure = describeReplaceFailure(error);
    } finally {
      saving = false;
    }
  }

  void load();
</script>

<section>
  <header>
    <h1>{labelFor({ route: "stats", handle })}</h1>
    <button type="button" onclick={() => void load()}>再読み込み</button>
    <!--
      **この見出しは、変容の案内が「」で名指しする導線でもある**（作業単位 D-15）。
      **綴りは `statsEdit.ts` の1つを使う**（案内と画面で別々に書かない）。
    -->
    <a href={hrefFor({ route: "characters" })}>{CHARACTER_LIST_LINK_LABEL}</a>
  </header>

  <!--
    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
    **変容を本文にもユーザーにも見せないという決定は ADR-0024 §9**——
    **その番号は、ここ（コメント）とこのファイルの冒頭にだけ残す。**
    **画面は利用者に向けて書く**（見張り番は `tests/screen-wording.test.ts` と
    `tests/stats-panel.test.ts` の「画面が利用者の言葉で話す」）。
  -->
  <p class="hint">
    <strong>変容は本文に出ません。</strong>
    値が動いたかどうかは、下の「変容の記録」でしか分かりません。
  </p>

  <h2>いまの値</h2>

  {#if statsState.kind === "failed"}
    <!--
      **「読めなかった」を「軸が無い」に見せない。**
      見せると、**空の置き換えを送らせる**という最悪の形になる。
    -->
    <p class="error" role="alert" data-stats-error="1">
      いまの値を取得できませんでした: {statsState.failure.message}
    </p>

    <!--
      **修復の入口**（作業単位 D-8）。**ここが無いと `curl` か `sqlite3` しか無い。**

      **⚠ 「壊れています」と言わない。** `ApiError::internal()` に `code` は無いので、
      **格納値が壊れた 500 と DB 障害の 500 を、FE は区別できない**
      （`statsRepair.ts` の ⚠ / 見張り番は `tests/stats-repair.test.ts`）。
      言えるのは「いまの値を読み出せない」までである。
    -->
    {#if repairEntry !== null && repairEntry.kind === "blocked"}
      <!--
        **押せば必ず失敗するボタンは出さない**（D-6 / D-7 と同じ立場）。
        **名指しする操作は、この画面に実在するものだけ**
        （「再読み込み」と「キャラクターの一覧へ」）。
      -->
      <p class="guidance" data-repair-blocked-reason={repairEntry.reason}>
        {repairEntry.message}
      </p>
    {:else if repairEntry !== null && repairEntry.kind === "form"}
      <!--
        **ここで `{#key handle}` を巻かない。** この画面自体が
        `App.svelte` の `{#key screen.handle}` で作り直される
        （**巻いても、識別子が変わったときにこの画面は読み直さない**ので、
        片方だけ作り直すと**見出しと中身がずれたまま**になる）。
        見張り番は `tests/app-routing-stats.test.ts`。
      -->
      <StatsRepairForm
        {handle}
        scale={repairEntry.scale}
        onRepaired={() => void load()}
      />
    {/if}
  {:else if statsState.kind === "loading" || draft === null}
    <p role="status">読み込み中…</p>
  {:else}
    {@const scale = statsState.value.stat_scale}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <p class="muted">
        このキャラクターの尺度は <strong>{scale}</strong> です（値の上端）。
        尺度そのものは、一覧の「編集」から変えられます。
      </p>

      <table>
        <thead>
          <tr>
            <th>軸</th>
            <th>値</th>
            <th></th>
          </tr>
        </thead>
        <tbody data-section="stats">
          <!--
            **索引で鍵を付ける。** 軸名は空にも重複にもなりうるので、
            軸名を鍵にすると**行が入れ替わる**（帰属の壊れ方）。
          -->
          {#each rows as row, index (index)}
            <tr
              data-axis={row.axis}
              data-removed={row.removed ? "true" : "false"}
              data-added={row.stored ? "false" : "true"}
              class:removed={row.removed}
            >
              <td>
                {#if row.stored}
                  <!--
                    **保存済みの軸名は変えられない。**
                    改名は「消して足す」でしか表現できないので、
                    **その2手を1つの欄に見せない**（気付かずに軸を消すことになる）。

                    **⚠ 守っているのはこの `{#if}` の1行だけ**
                    （`statsEdit.ts` の `removedAxes` がこれに寄りかかっている）。
                    **見張り番と、改名で実際に何が起きるかの実測は
                    `tests/stats-panel.test.ts` の**
                    **`保存済みの軸名は書き換えられない（改名は「消して足す」でしかない）`。**
                  -->
                  {row.axis}
                {:else}
                  <input
                    name="axis"
                    value={row.axis}
                    disabled={saving}
                    oninput={(event) => setAxis(index, event.currentTarget.value)}
                  />
                {/if}
              </td>
              <td>
                <input
                  name="value"
                  value={row.value}
                  disabled={saving || row.removed}
                  oninput={(event) => setValue(index, event.currentTarget.value)}
                />
                <span class="muted">/ {scale}</span>
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

      {#if rows.length === 0}
        <p class="muted">軸が1本もありません。</p>
      {/if}

      <div class="actions">
        <button type="button" onclick={addRow} disabled={saving}>軸を追加</button>
      </div>

      <!--
        **丸ごと置き換えであることを、押す前に言う**（`replaceSummary`）。
        言わないと、**外したつもりのない軸が消えたことに、押した後まで気付けない。**
      -->
      <p class="summary" data-replace-summary="1">{summary}</p>

      {#if built !== null && !built.ok}
        <p class="error" role="alert" data-build-error="1">{built.message}</p>
      {/if}
      {#if failure !== null}
        <!-- **サーバが言ったことを、そのまま出す**（こちらから足さない）。 -->
        <p class="error" role="alert" data-replace-reason={failure.reason}>
          {failure.message}
        </p>
        {#if failure.guidance !== null}
          <!--
            **次に取れる行動。** 分類できなかった失敗には出ない（`null`）——
            **推測で埋めない。** 名指しする操作は**実在するものだけ**
            （見張り番は `tests/stat-scale-path.test.ts`）。
          -->
          <p class="guidance" data-replace-guidance="1">{failure.guidance}</p>
        {/if}
      {/if}
      {#if notice !== null}
        <p class="notice" role="status">{notice}</p>
      {/if}

      <div class="actions">
        <button type="submit" disabled={!canSubmit}>この内容で置き換える</button>
      </div>
    </form>
  {/if}

  <h2>変容の記録</h2>

  {#if driftState.kind === "failed"}
    <p class="error" role="alert" data-drift-error="1">
      変容の記録を取得できませんでした: {driftState.message}
    </p>
  {:else if driftState.kind === "loading"}
    <p role="status">読み込み中…</p>
  {:else}
    {@const drift = driftState.value}

    <!--
      **走らせ方を必ず出す**（作業単位 D-5a）。

      **サーバ側の既定は `off`** で、`off` は判定もせず票も累積も1行も作らない。
      **つまり既定の配置では「総数 0 件・累積 0・票なし」が出る** ——
      **変容が壊れている状態と、この画面の上で1ドットも変わらない。**
      走らせ方はサーバの起動ログにしか出ておらず、**画面を見ている人は読めない。**

      **綴りは応答のもの**（こちらに写しを持たない / `api.ts` の `run_mode`）。
      **生の綴りも印として持つ**（`data-run-mode`）——`data-delta` と同じ理由で、
      写した語彙が外れた日に見出しだけでなく値が見える形にしておく。
    -->
    <p class="muted" data-run-mode={drift.run_mode}>
      変容の走らせ方: <strong>{runModeLabel(drift.run_mode)}</strong>
    </p>
    <!--
      **どこから来た走らせ方か**（作業単位 D-15 / ADR-0045 §決定4）。**走らせ方によらず出す。**
      **生の綴りも印として持つ**（`data-run-mode` と同じ理由）。
    -->
    <p class="muted" data-run-mode-source={drift.run_mode_source}>
      {runModeSourceNote(drift.run_mode_source)}
    </p>

    <!--
      **止まっているなら、その理由と、どこで切り替えられるかを言う**（`runModeNotice`）。

      **名指しするのはヘッダの「キャラクターの一覧へ」だけ**（作業単位 D-15）。
      走らせ方の編集はこの画面に無いので、そこから辿れると言う。
      **無い操作を案内しない**（`B-15` / `B-16` / `D-6` / `D-7` / `D-8` と同じ立場）。
      見張り番は `tests/stats-panel.test.ts` の
      `assertNoticeNamesOnlyRealOperations`（**この案内は名指しが必須の側に入っている**）。
    -->
    {#if stoppedNotice !== null}
      <p class="guidance" data-run-mode-notice="1">{stoppedNotice}</p>
    {/if}

    <!--
      **軸が1本も無いなら、それが「0 件」の理由だと言う**（作業単位 F-6）。

      **サーバは軸ゼロの回を上流へ投げない**（`drift_skipped=no_axes` /
      `src/server/drift.rs`）。**言わないと、画面は**
      **「実行中（票を残し、値も動かします）」と「まだ1件も票がありません」を
      理由なしに並べる** —— **新しいキャラクターは必ずこの状態から始まる。**

      **⚠ 停止中の案内と両方出ることがある。** 2つは別のことを言っているので畳まない。
      見張り番は `tests/stats-panel.test.ts` の
      `軸が1本も無いことが、0 件の理由として読める`（**この案内も
      `assertNoticeNamesOnlyRealOperations` の網に入っている**）。
    -->
    {#if noAxes !== null}
      <p class="guidance" data-no-axes-notice="1">{noAxes}</p>
    {/if}

    <table>
      <thead>
        <tr>
          <th>軸</th>
          <!--
            **上の「いまの値」と、同じ数とは限らない。**
            2つは別々の要求なので、間に別の窓が書き換えればずれる
            ——**どちらの応答の数かが読み手に分かる見出しにする。**
          -->
          <th>値（この記録を読んだ時点）</th>
          <th>累積</th>
          <!--
            **累積が最後に動いた日時**（作業単位 D-5a）。
            **累積の表は履歴を持たない**ので、表から取れる時間の手掛かりはこの1列だけ。
            **票の日時とは別の時刻である** ——
            記録のみで回している間は、票だけが進んでこちらは止まる。
          -->
          <th>累積が動いた日時</th>
          <th>上げ</th>
          <th>下げ</th>
        </tr>
      </thead>
      <tbody data-section="drift-axes">
        <!--
          **値 0 の軸も、累積 0 の軸も落とさない**（サーバは両方返す）。
          落とすと「軸を消したのか、まだ動いていないのか」が区別できなくなる。
          **`ups` と `downs` を差に畳まないこと**（`DriftAxisView` の doc）。
        -->
        <!--
          **⚠ 鍵は軸名。** 応答の中で軸名が一意であることに寄りかかっている。
          **その一意性が崩れる作り方が1つある**（DB を直接編集して、
          正規化すると同じになる軸を2つ置く）——条件と実測は
          `admin.rs` の `DriftView::total_votes` の doc が持つ。
          **そのときここで何が起きるかは未実測。**
        -->
        {#each drift.axes as axis (axis.axis)}
          <tr
            data-drift-axis={axis.axis}
            data-allocated={axis.value === null ? "false" : "true"}
            data-value={axis.value}
            data-charge={axis.charge}
            data-charge-updated-at={axis.charge_updated_at}
            data-ups={axis.ups}
            data-downs={axis.downs}
            class:unallocated={axis.value === null}
          >
            <td>{axis.axis}</td>
            <!--
              **この値は変容の応答のもの**（いまの値の応答とは別の読み）。

              **⚠ `axis.value` を直に埋めないこと**（作業単位 B-16）。
              **割り当ての無い軸は `null`** で返るので、直に埋めると `null / 7` と出る。
              **`?? 0` も駄目** —— 値 0 の軸と同じ行になる（`api.ts` の `value` の doc）。
              綴りを決めるのは `statsEdit.ts` の `axisValueLabel` **1箇所だけ**。
            -->
            <td>{axisValueLabel(axis.value, drift.stat_scale)}</td>
            <td>{axis.charge}</td>
            <!--
              **⚠ `axis.charge_updated_at` を直に埋めないこと**（作業単位 D-5a）。
              **累積の行が無い軸は `null`** で返るので、直に埋めると `null` と出る。
              **現在時刻で埋めるのはもっと悪い** ——「たったいま動いた」と読める。
              綴りを決めるのは `statsEdit.ts` の `chargeTimeLabel` **1箇所だけ**。

              **⚠ 生の綴りを `title` に残す**（作業単位 F-7）。
              **見出しはミリ秒を畳んである**ので、ログや `sqlite3` の値と
              1文字ずつ突き合わせる道が、これが無いと閉じる。
              **`data-*` と違い、`title` は読み手が実際に触れる。**
            -->
            <td title={axis.charge_updated_at}
              >{chargeTimeLabel(axis.charge_updated_at, zone)}</td
            >
            <td>{axis.ups}</td>
            <td>{axis.downs}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if drift.axes.length === 0}
      <p class="muted">軸が1本もありません。</p>
    {/if}

    <!--
      **割り当てを失った軸があることと、直し方を言う**（作業単位 B-16）。

      **サーバは落とさずに返す** —— 累積と票は残っていて、総数にも数えられている。
      **言わないと、読み手には「知らない軸が出てきた」としか見えない。**

      **名指しする操作は、この画面に実在するものだけ**（`unallocatedNotice` の doc）。
      **`canReplace` を渡しているのはそのため** ——
      `GET .../stats` が失敗した回は、名指しする2つが描かれない。

      見張り番は `tests/stats-panel.test.ts` の
      `案内が名指しした操作は、この画面に実在する` と、
      **どの状態にも当てる** `assertNoticeNamesOnlyRealOperations`。
    -->
    {#if lostNotice !== null}
      <p class="guidance" data-unallocated-notice="1">{lostNotice}</p>
    {/if}

    <!--
      **総数を必ず言う**（`RECENT_DRIFT_VOTES` の doc ——
      **「言えないまま切ると、画面は『これで全部』と嘘をつく」**）。
      **上限も応答のもの**（こちらに写しを持たない）。
    -->
    <p class="muted" data-vote-total={drift.total_votes}>
      {#if drift.total_votes === 0}
        まだ1件も票がありません（このキャラクターの値は、まだ一度も変容していません）。
      {:else if hiddenVotes(drift) > 0}
        全 {drift.total_votes} 件の票のうち、直近 {drift.recent_votes.length} 件だけを出しています（1回に返る上限は
        {drift.recent_limit} 件 / 残り {hiddenVotes(drift)} 件はここに出ていません）。
      {:else}
        全 {drift.total_votes} 件の票を、すべて出しています（1回に返る上限は {drift.recent_limit} 件）。
      {/if}
    </p>

    {#if drift.recent_votes.length > 0}
      <table>
        <thead>
          <tr>
            <th>軸</th>
            <th>向き</th>
            <th>会話</th>
            <th>日時</th>
          </tr>
        </thead>
        <tbody data-section="drift-votes">
          <!--
            **新しい順のまま出す。** 並びを決めているのは `store` の1文なので、
            ここで並べ替えない（同着は本番で普通に起きる —— 1ターンで複数の軸が動く）。
            **生の綴りも持つ**（`data-delta`）——写した語彙が外れた日に、
            見出しが消えるだけでなく値が見える形にしておく。
          -->
          {#each drift.recent_votes as vote, index (index)}
            <tr
              data-vote-axis={vote.axis}
              data-delta={vote.delta}
              data-session-id={vote.session_id}
            >
              <td>{vote.axis}</td>
              <td>{deltaLabel(vote.delta)}</td>
              <td>{vote.session_id}</td>
              <!--
                **⚠ 生の綴りを `title` に残す**（作業単位 F-7）。
                **同じ秒に立った票は本番で普通に起きる**（1ターンで複数の軸が動く）ので、
                畳んだ見出しだけだと画面から区別できない。
              -->
              <td title={vote.created_at}
                >{timeLabel(vote.created_at, zone)}</td
              >
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/if}
</section>

<style>
  /* **表・枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /*
    **割り当てを失った軸。** 消えているのに記録だけが残っている行（B-16）。
    **「壊れている」側の色**を敷く —— 直すべき状態であって、良い報せではない。
  */
  tr.unallocated td {
    background: var(--color-warn-bg);
  }
</style>
