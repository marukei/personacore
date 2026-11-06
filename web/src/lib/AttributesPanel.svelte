<script lang="ts">
  /**
   * **属性と変更の記録の画面**（作業単位 D-12a / ADR-0009 / ADR-0028 §決定5）。
   *
   * # なぜこの画面が要るのか
   *
   * **属性はプロンプトの「削れない枠」に載る**ので、モデルは毎ターン必ず読む。
   * ADR-0023 の 13d が測ったとおり**それが効く** ——名指しした事実はドリフトゼロ。
   * **そして同じ理由で、古くなっても毎ターン必ず読む。**
   *
   * 直す道は管理 API しか無かった（`PUT .../attributes`）。
   * **会話で「引っ越しました」と言われても、属性は古い事実を固定し続ける。**
   *
   * # なぜ「変更の記録」を必ず出すのか
   *
   * **`GET .../attributes` が答えるのは「いまどうなっているか」だけ。**
   * 属性が静かに変わったとき、**気付く手段が他に無い**
   * ——性格の変容には票の表がある（`StatsPanel`）。属性にもこれが要る。
   *
   * # ⚠ **`PUT .../attributes` は丸ごと置き換えである**
   *
   * **要求に含まれない属性は消える。空の配列を渡すと全属性が消える**
   * （`admin.rs` の `replace_attributes`）。
   * **「変えない」を表す手段が無い**ので、**画面は必ず全件を送る**
   * （`attributesEdit.ts` の `buildReplaceAttributes`）。
   *
   * **属性を消す手段は「送らないこと」しかない。**
   * だから「外す」は**押した時点では何も起きず、保存したときに消える。**
   * **何が残って何が消えるかは、押す前に `replaceSummary` が言う。**
   *
   * # 2つの読みは別々（**片方が落ちても、もう片方は出す**）
   *
   * `GET .../attributes` と `GET .../attributes/revisions` は別の要求で、
   * **1行でも壊れていれば履歴の一覧だけが 500 になる**
   * （属性そのものの読みも置き換えも通る / `admin.rs` の `get_attribute_revisions`）。
   * **失敗も別々に持つ** —— 片方の失敗で画面ごと落とすと、
   * **記録が読めないだけで置き換えの入口まで消える。**
   *
   * # 書き込みの口は1本だけ
   *
   * 変更の記録は**読むだけ**（追記のみ / ADR-0028 §決定5）。
   * **記録を消す・直す操作をこの画面に生やさないこと。** サーバにも経路が無い
   * （1件を指すパスすら生やしていない）。
   *
   * # ⚠ 壊れた行の修復フォームは無い（**守れていないこと**）
   *
   * `GET .../attributes` が 500 になる状態（上限を超えた値が保存されている等）でも
   * **`PUT` は既存の行を読まないので届く**——つまりサーバ側には修復経路がある。
   * **しかしこの画面は下書きを `GET` から作る**ので、その回はフォームを描けない。
   * `StatsPanel` の `StatsRepairForm` に当たるものは、**この作業単位では作っていない。**
   * **理由と代わりの手段は `web/README.md` の「守れていないこと」にある。**
   */

  import {
    ApiRequestError,
    getAttributes,
    listAttributeRevisions,
    replaceAttributes,
    type AttributeRevisionsView,
    type AttributesView,
  } from "./api";
  import {
    ATTRIBUTE_LIMITS,
    MAX_ATTRIBUTES,
    afterLabel,
    attributeCounter,
    attributeSubjectLabel,
    beforeLabel,
    buildReplaceAttributes,
    describeReplaceAttributesFailure,
    draftOfAttributes,
    emptyAttributeRow,
    isOverAttributeLimit,
    replaceSummary,
    revisionCountNotice,
    type AttributesDraft,
    type ReplaceAttributesFailure,
  } from "./attributesEdit";
  // **主語の選択肢の綴りは1箇所**（`memoryRepair.ts` / 写しを増やさない）。
  import { SUBJECTS } from "./memoryRepair";
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
   * **ここで解決して `timeLabel` へ渡す** ——あちらに自分で決めさせない。
   * 決めさせると、**あの関数を見る検査が、走らせた機械の時間帯でしか成立しなくなる。**
   *
   * **⚠ 綴りを直接書かないこと**（`"Asia/Tokyo"` 等）。
   * 書くと、**開発者の機械では正しく見えて、他の誰の画面でも時刻がずれる。**
   * 見張り番は `tests/attributes-panel.test.ts`（`Pacific/Marquesas`）と
   * `tests/app-routing-attributes.test.ts`（`Asia/Katmandu`）——
   * **消費側が2本あって別々の時間帯を名乗る**ので、どちらを直書きしても赤くなる。
   */
  const zone = browserZone();

  /**
   * 取得の結果。
   *
   * **`null` を「失敗」と「まだ」に使い回さない。** 使い回すと、
   * **取得に失敗したキャラクターが「属性が0件」「記録が0件」に見える**
   * ——**変わっていないことと、読めなかったことは別**である。
   */
  type Loaded<T> =
    | { kind: "loading" }
    | { kind: "ready"; value: T }
    | { kind: "failed"; message: string };

  let attributesState = $state<Loaded<AttributesView>>({ kind: "loading" });
  let revisionsState = $state<Loaded<AttributeRevisionsView>>({
    kind: "loading",
  });

  /**
   * 置き換えの下書き。**いまの値を読み終えてから作る。**
   *
   * **`load()` のたびに作り直す** —— 打ちかけを残すと、
   * **別の窓で変わった値の上に古い写しを書き戻す**ことになる。
   */
  let draft = $state<AttributesDraft | null>(null);

  let saving = $state(false);
  let notice = $state<string | null>(null);
  /** **置き換えが拒否された理由。** 読み分けは `attributesEdit.ts`。 */
  let failure = $state<ReplaceAttributesFailure | null>(null);

  /** 送る本文。**組み立ては純粋関数**（`attributesEdit.ts`）。 */
  const built = $derived(draft === null ? null : buildReplaceAttributes(draft));

  const canSubmit = $derived(built !== null && built.ok && !saving);

  /**
   * 画面が使う行。**`draft` を直に読ませない。**
   *
   * `draft` は `null` を取りうるので、テンプレートの各所で絞り込みが要る
   * ——**絞り込みは分岐や閉包をまたぐと外れる**ので、ここで1回だけ畳む。
   */
  const rows = $derived(draft?.rows ?? []);

  /** いまサーバに在る属性（**要約の突き合わせの相手**）。 */
  const stored = $derived(
    attributesState.kind === "ready" ? attributesState.value.attributes : [],
  );

  /** 保存すると何が記録されるか（`attributesEdit.ts`）。 */
  const summary = $derived(draft === null ? "" : replaceSummary(stored, draft));

  function messageOf(error: unknown): string {
    if (error instanceof ApiRequestError) {
      return error.message;
    }
    return "予期しない失敗です";
  }

  /**
   * いまの値と変更の記録を読む。**2本は独立に扱う。**
   *
   * 片方の失敗でもう片方を落とさない（`StatsPanel` の `load` と同じ理由）。
   */
  async function load(): Promise<void> {
    attributesState = { kind: "loading" };
    revisionsState = { kind: "loading" };
    await Promise.all([loadAttributes(), loadRevisions()]);
  }

  async function loadAttributes(): Promise<void> {
    try {
      const value = await getAttributes(handle);
      attributesState = { kind: "ready", value };
      draft = draftOfAttributes(value);
    } catch (error) {
      attributesState = { kind: "failed", message: messageOf(error) };
      draft = null;
    }
  }

  async function loadRevisions(): Promise<void> {
    try {
      revisionsState = { kind: "ready", value: await listAttributeRevisions(handle) };
    } catch (error) {
      revisionsState = { kind: "failed", message: messageOf(error) };
    }
  }

  /** 属性を1件足す。**送るのは保存のときだけ**（ここでは何も起きない）。 */
  function addRow(): void {
    if (draft === null) {
      return;
    }
    draft.rows.push(emptyAttributeRow());
  }

  /**
   * その行を「外す」／「戻す」。
   *
   * **押しただけでは何も起きない。** 外した行は**本文に載らなくなる**だけで、
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

  /** キーの欄（**足した行だけ**）。 */
  function setKey(index: number, key: string): void {
    const row = draft?.rows[index];
    if (row === undefined) {
      return;
    }
    row.key = key;
  }

  /** 主語の欄（**足した行だけ**）。 */
  function setSubject(index: number, subject: string): void {
    const row = draft?.rows[index];
    if (row === undefined) {
      return;
    }
    row.subject = subject;
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
      await replaceAttributes(handle, request.body);
      // **読み直す。** 応答も「書いた後に読み直した姿」だが、
      // **変更の記録は別の入口**なので、両方取り直さないと
      // **いま自分が起こした変更だけが表に出ない。**
      notice = "属性を置き換えました。";
      await load();
    } catch (error) {
      failure = describeReplaceAttributesFailure(error);
    } finally {
      saving = false;
    }
  }

  void load();
</script>

<section>
  <header>
    <h1>{labelFor({ route: "attributes", handle })}</h1>
    <button type="button" onclick={() => void load()}>再読み込み</button>
    <a href={hrefFor({ route: "characters" })}>キャラクターの一覧へ</a>
  </header>

  <!--
    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
    **属性が削れない枠に載るという決定は ADR-0009 / ADR-0028**——
    **その番号は、ここ（コメント）とこのファイルの冒頭にだけ残す。**
    見張り番は `tests/screen-wording.test.ts`。
  -->
  <p class="hint">
    <strong>属性は、毎ターンのプロンプトに必ず載ります。</strong>
    古い値もそのまま載り続けるので、変わったらここで直します。
  </p>

  <h2>いまの値</h2>

  {#if attributesState.kind === "failed"}
    <!--
      **「読めなかった」を「属性が無い」に見せない。**
      見せると、**空の置き換えを送らせる**という最悪の形になる。

      **⚠ 「壊れています」と言わない。** `ApiError::internal()` に `code` は無いので、
      **格納値が壊れた 500 と DB 障害の 500 を、FE は区別できない。**
      言えるのは「いまの値を読み出せない」までである。
    -->
    <p class="error" role="alert" data-attributes-error="1">
      いまの値を取得できませんでした: {attributesState.message}
    </p>
  {:else if attributesState.kind === "loading" || draft === null}
    <p role="status">読み込み中…</p>
  {:else}
    <form
      onsubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <table>
        <thead>
          <tr>
            <th>主語</th>
            <th>キー</th>
            <th>値</th>
            <th></th>
          </tr>
        </thead>
        <tbody data-section="attributes">
          <!--
            **索引で鍵を付ける。** キーは空にも重複にもなりうるので、
            キーを鍵にすると**行が入れ替わる**（帰属の壊れ方）。
          -->
          {#each rows as row, index (index)}
            <tr
              data-attribute-subject={row.subject}
              data-attribute-key={row.key}
              data-removed={row.removed ? "true" : "false"}
              data-added={row.stored ? "false" : "true"}
              class:removed={row.removed}
            >
              <td>
                {#if row.stored}
                  <!--
                    **保存済みの主語は付け替えられない。**
                    付け替えは「消して足す」でしかない（`(subject, key)` が
                    変われば別の属性になる）ので、**その2手を1つの欄に見せない。**
                  -->
                  {attributeSubjectLabel(row.subject)}
                {:else}
                  <select
                    name="subject"
                    value={row.subject}
                    disabled={saving}
                    onchange={(event) => setSubject(index, event.currentTarget.value)}
                  >
                    <!--
                      **既定を作らない**（ADR-0028 §決定4 —— 主語を取り違えると、
                      そのキャラクターの以後の全ターンに効く）。
                    -->
                    <option value="">主語を選んでください</option>
                    {#each SUBJECTS as subject (subject)}
                      <option value={subject}>{attributeSubjectLabel(subject)}</option>
                    {/each}
                  </select>
                {/if}
              </td>
              <td>
                {#if row.stored}
                  <!-- **保存済みのキーは書き換えられない**（上と同じ理由）。 -->
                  {row.key}
                {:else}
                  <input
                    name="key"
                    value={row.key}
                    disabled={saving}
                    oninput={(event) => setKey(index, event.currentTarget.value)}
                  />
                  <span
                    class="counter"
                    data-counter="key"
                    class:over={isOverAttributeLimit("key", row.key)}
                  >
                    {attributeCounter("key", row.key)}
                  </span>
                {/if}
              </td>
              <td>
                <input
                  name="value"
                  value={row.value}
                  disabled={saving || row.removed}
                  oninput={(event) => setValue(index, event.currentTarget.value)}
                />
                <span
                  class="counter"
                  data-counter="value"
                  class:over={isOverAttributeLimit("value", row.value)}
                >
                  {attributeCounter("value", row.value)}
                </span>
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
        <p class="muted">属性が1件もありません。</p>
      {/if}

      <div class="actions">
        <button type="button" onclick={addRow} disabled={saving}>属性を追加</button>
      </div>

      <!--
        **3つの上限を、越える前に出す**（作業単位 D-12a）。
        **件数の上限は「越えてから」では遅い** ——
        9件目を打ち終えてから断られることになる。
        **値はどれも写しである**（写し元は `src/domain/attribute.rs` /
        見張り番は `tests/attributes-edit.test.ts`）。
      -->
      <p class="note">
        キーは {ATTRIBUTE_LIMITS.key} 文字、値は {ATTRIBUTE_LIMITS.value} 文字、
        属性は全部で {MAX_ATTRIBUTES} 件までです。
      </p>

      <!--
        **丸ごと置き換えであることを、押す前に言う**（`replaceSummary`）。
        言わないと、**外したつもりのない属性が消えたことに、押した後まで気付けない。**
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
            **推測で埋めない。** 名指しする操作は**実在するものだけ。**
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

  <h2>変更の記録</h2>

  {#if revisionsState.kind === "failed"}
    <p class="error" role="alert" data-revisions-error="1">
      変更の記録を取得できませんでした: {revisionsState.message}
    </p>
  {:else if revisionsState.kind === "loading"}
    <p role="status">読み込み中…</p>
  {:else}
    {@const revisions = revisionsState.value}

    <!--
      **総数を必ず言う**（`AttributeRevisionsView::total_revisions` の doc ——
      上限を名乗るだけでは「たまたまその数」と「切られた」を区別できない）。
      **上限も応答のもの**（こちらに写しを持たない）。
    -->
    <p class="muted" data-revision-total={revisions.total_revisions}>
      {revisionCountNotice(revisions)}
    </p>

    {#if revisions.revisions.length > 0}
      <table>
        <thead>
          <tr>
            <th>日時</th>
            <th>主語</th>
            <th>キー</th>
            <th>前</th>
            <th>後</th>
          </tr>
        </thead>
        <tbody data-section="attribute-revisions">
          <!--
            **新しい順のまま出す。** 並びを決めているのは `store` の1文なので、
            ここで並べ替えない（同着は本番で普通に起きる ——
            1回の置き換えで複数の属性が動く）。
          -->
          {#each revisions.revisions as revision (revision.id)}
            <tr
              data-revision-id={revision.id}
              data-revision-subject={revision.subject}
              data-revision-key={revision.key}
            >
              <!--
                **⚠ 生の綴りを `title` に残す**（作業単位 F-7）。
                **見出しはミリ秒を畳んである**ので、ログや `sqlite3` の値と
                1文字ずつ突き合わせる道が、これが無いと閉じる。
                **1回の置き換えで同じ秒の行が複数立つ**ので、畳んだ見出しだけでは
                画面から区別できない。
              -->
              <td title={revision.created_at}
                >{timeLabel(revision.created_at, zone)}</td
              >
              <td>{attributeSubjectLabel(revision.subject)}</td>
              <td>{revision.key}</td>
              <!--
                **⚠ `null` を素で埋めないこと。**
                `before` の `null` は**無かった**（＝追加）、
                `after` の `null` は**消えた**（＝削除）で、**別のことを言っている。**
                潰すと**追加と削除の区別が画面から消える。**
                綴りを決めるのは `attributesEdit.ts` の
                `beforeLabel` / `afterLabel` **1箇所だけ。**
              -->
              <td>{beforeLabel(revision.before)}</td>
              <td>{afterLabel(revision.after)}</td>
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
    **外した行。** 保存すると消える行なので、**「壊れている」側の色**を敷く
    ——良い報せではない（`StatsPanel` の `tr.unallocated` と同じ立場）。
  */
  tr.removed td {
    background: var(--color-warn-bg);
  }
</style>
