<script lang="ts">
  import { untrack } from "svelte";

  import {
    ApiRequestError,
    createCharacter,
    updateCharacter,
    type CharacterView,
  } from "./api";
  import {
    LIMITS,
    buildCreate,
    buildUpdate,
    countChars,
    draftOf,
    emptyDraft,
    isEmptyUpdate,
    isOverLimit,
    labelOf,
    type CharacterDraft,
    type LimitedField,
  } from "./characterEdit";
  import {
    DRIFT_MODE_FIELD_LABEL,
    driftModeChoices,
    isKnownDriftModeChoice,
  } from "./driftModeChoice";

  interface Props {
    /** **`null` は新規作成。** それ以外はその行の編集。 */
    character?: CharacterView | null;
    /**
     * **起動時の既定の走らせ方を引く口**（作業単位 D-15）。
     * 「既定に従う（いまは: …）」の括弧に使う。
     *
     * **部品は自分で `GET` しない。** 引く口は親（`CharacterList.svelte`）が渡す ——
     * 親が `getDrift` の `default_run_mode` を返す。**渡さなければ括弧を出さない。**
     * 呼ぶのは**編集している行があるときだけ**（新規作成ではキャラクターがまだ無い）。
     */
    loadDefaultRunMode?: (handle: string) => Promise<string>;
    /** 保存が通ったとき（一覧を読み直すため）。**読み直せなかった回は呼ばれない。** */
    onSaved?: (character: CharacterView) => void;
    /** 閉じる。**渡さなければ「閉じる」ボタンを出さない。** */
    onClose?: () => void;
  }

  let { character = null, loadDefaultRunMode, onSaved, onClose }: Props = $props();

  /**
   * **この画面が作った行。**
   *
   * # なぜ、作成の後に編集へ切り替えるのか
   *
   * `POST` が通った時点で**その識別子はもう存在する。** 同じ画面がもう一度
   * `POST` すると 409（`character_already_exists`）になるだけで、
   * **利用者から見れば「保存できない画面」**になる。
   *
   * **作成の直後は、まだ性格値も属性も無い**（どちらも別の入口 / 作業単位 D-2 の範囲外）。
   * 一覧へ戻すと、打ち間違いを直したいときにもう一度探すことになるので、
   * **その行の編集を開いたまま**にする。**一覧の側は `onSaved` で読み直される。**
   */
  let saved = $state<CharacterView | null>(null);

  /** **いま編集している行。** `null` なら、まだ作成していない。 */
  const current = $derived(saved ?? character);

  /**
   * 下書き。**初期値だけを `character` から取る。**
   *
   * **⚠ 後から `character` が別の行に差し替わっても追随しない**
   * （`untrack` はそれを明示するため）。**追随させないのが意図** ——
   * 打ちかけの本文が、親の再描画で黙って消えるのが最悪。
   * **別の行を編集させたい親は `{#key}` で作り直すこと**（`CharacterList.svelte`）。
   */
  let draft = $state<CharacterDraft>(
    untrack(() => (character === null ? emptyDraft() : draftOf(character))),
  );

  let saving = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  /**
   * **起動時の既定の走らせ方**（作業単位 D-15）。**分からないあいだは `null`**
   * （新規作成・引いている途中・読めなかった回）で、そのときは括弧を出さない。
   *
   * **いま編集している行の識別子で引き直す**（作成の直後は、作った行で引く）。
   * **古い応答で上書きしない** —— 引き直しの前の応答が後から届いても捨てる。
   */
  let defaultRunMode = $state<string | null>(null);

  /**
   * **引き直しの鍵は識別子の文字列だけ。** `current`（行そのもの）に依存させると、
   * 保存のたびに行が新しくなり、同じ行の既定を毎回引き直す。
   */
  const currentHandle = $derived(current?.handle);

  $effect(() => {
    const handle = currentHandle;
    const load = loadDefaultRunMode;
    defaultRunMode = null;
    if (handle === undefined || load === undefined) {
      return;
    }
    let live = true;
    load(handle).then(
      (mode) => {
        // **文字列でなければ使わない**（応答の形が崩れていても推測しない）。
        if (live && typeof mode === "string") {
          defaultRunMode = mode;
        }
      },
      () => {
        // **読めなかったら括弧を出さない**（「停止」と推測しない）。
      },
    );
    return () => {
      live = false;
    };
  });

  /** 走らせ方の選択肢（**4つ**。「既定に従う」の括弧だけが既定で変わる）。 */
  const driftChoices = $derived(driftModeChoices(defaultRunMode));

  /**
   * **送る本文と、その送り先。**
   *
   * `kind` が `create` か `update` かは `current` だけで決まる
   * ——画面が別途「いまは作成中」という旗を持たない（2つ持つと食い違う）。
   */
  const built = $derived(
    current === null ? buildCreate(draft) : buildUpdate(current, draft),
  );

  /** 送るものがあるか。**編集で差分が空なら押させない**（同じ本文を二度送らない）。 */
  const hasSomethingToSend = $derived(
    built.ok && (built.kind === "create" || !isEmptyUpdate(built.body)),
  );

  const canSubmit = $derived(hasSomethingToSend && !saving);

  /**
   * **その欄が、その欄の上限を超えているか。**
   *
   * **⚠ 欄と上限の組み合わせは、この1行が決めている。**
   * `isOverLimit("base_persona", …)` のように片方を決め打っても、
   * **超過そのものは `built` の側でも判定される**ので
   * **ボタンもメッセージも正しいまま、印だけが別の欄に付く。**
   * **見張り番は `tests/character-form.test.ts::`
   * `印は、超えた欄にだけ付く`**（他の欄に付かないことも見る）。
   */
  function over(field: LimitedField): boolean {
    return isOverLimit(field, draft[field]);
  }

  /**
   * いま何文字か。**サーバと同じ数え方**（`countChars` の doc）。
   *
   * **⚠ ここも「どの欄の上限か」を取り違えても何も壊れない**
   * （形は `n / m` のままで、`m` だけが他人の値になる）。
   * **1つの文字列にまとめてあるのは読みやすさのためで、守りではない。**
   * **守っているのは `tests/character-form.test.ts::`
   * `カウンタは、その欄の上限を出す`** ——
   * **欄ごとに `data-counter` を引いて、他の欄の上限が出ていないことまで見る。**
   */
  function counterOf(field: LimitedField): string {
    return `${countChars(draft[field])} / ${LIMITS[field]}`;
  }

  function messageOf(failure: unknown): string {
    if (failure instanceof ApiRequestError) {
      return failure.message;
    }
    return "予期しない失敗です";
  }

  async function save(): Promise<void> {
    const request = built;
    if (!canSubmit || !request.ok) {
      return;
    }
    const target = current;
    saving = true;
    error = null;
    notice = null;

    try {
      if (request.kind === "create" || target === null) {
        // **`create` 以外がここへ来ることはない**（`built` は `current` で決まる）。
        if (request.kind !== "create") {
          return;
        }
        const created = await createCharacter(request.body);
        saved = created;
        draft = draftOf(created);
        notice = `${created.handle} を作成しました。続けて編集できます。`;
        onSaved?.(created);
      } else {
        // **送り先は「いま編集している行」の識別子。**
        const updated = await updateCharacter(target.handle, request.body);
        if (updated.character === null) {
          // **失敗ではない**（`admin.rs` の `UpdatedCharacterView`）。
          // 更新は通っていて、読み直せなかっただけ。
          // **下書きは触らない** ——送った値がそのまま残るので、
          // まだ直っていない列があれば続けて直せる。
          notice =
            "更新しました。ただし更新後の姿を読み直せていません（一覧の「読み出せなかった行」を見てください）。";
        } else {
          saved = updated.character;
          draft = draftOf(updated.character);
          notice = "更新しました。";
          onSaved?.(updated.character);
        }
      }
    } catch (failure) {
      error = messageOf(failure);
    } finally {
      saving = false;
    }
  }
</script>

<form
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <h3>
    {#if current === null}
      新しいキャラクター
    {:else}
      {current.display_name}（<code>{current.handle}</code>）を編集
    {/if}
  </h3>

  {#if current === null}
    <label>
      <!-- **見出しは `labelOf` から取る。** 上限のメッセージと同じ綴りにする。 -->
      <span>{labelOf("handle")}</span>
      <input
        name="handle"
        placeholder="aoi"
        bind:value={draft.handle}
        disabled={saving}
      />
      <span class="counter" data-counter="handle" class:over={over("handle")}>
        {counterOf("handle")}
      </span>
    </label>
    <!--
      **使える文字を、入力する前に一度だけ言う**（作業単位 D-14 / Issue origin#30）。

      **⚠ これは `Handle::ALLOWED_CHARACTERS` の写し（3箇所目）である。**
      `src/domain/character.rs` の `ALLOWED_CHARACTERS` の doc は「3箇所目を作らないこと」と書いていたが
      （いまは画面を3箇所目に数えている）、サーバの理由は使える文字を列挙しない
      （`HandleError::Charset` の doc）ので、**日本語を打った利用者が何を直せばよいか分からなかった。**
      **写しが古びないように、写し元を読む見張り番がある** ——
      `tests/character-form-examples.test.ts` が、描かれた説明から文字の集合を読み
      （範囲 `a〜z` と、1文字の引用 `「-」` だけを読む）、`src/domain/character.rs` と突き合わせる。
      **綴りを変えるときは、その読み方で読める形のまま変えること。**
      **この画面の他の場所で、同じ文字を列挙しないこと**（同じ見張り番が「一度だけ」を見ている）。

      **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
      **識別子が不変であることの決定は ADR-0016 §1**——
      **その番号は、ここ（コメント）とコード側にだけ残す。**
      **画面は利用者に向けて書く**（見張り番は `tests/screen-wording.test.ts` と
      `tests/character-form.test.ts` の「画面が利用者の言葉で話す」）。
    -->
    <p class="hint" data-hint="handle">
      英小文字（a〜z）・数字（0〜9）・「-」・「_」だけが使えます。会話の送り先や URL に使われます。日本語の名前は、下の表示名に書いてください。作成のときだけ決められます（後から変えられません）。
    </p>
  {/if}

  <label>
    <span>{labelOf("display_name")}</span>
    <input
      name="display_name"
      placeholder="葵"
      bind:value={draft.display_name}
      disabled={saving}
    />
    <span class="counter" data-counter="display_name" class:over={over("display_name")}>
      {counterOf("display_name")}
    </span>
  </label>
  <p class="hint" data-hint="display_name">
    キャラクターの呼び名です。日本語で書けます。{#if current === null}省くと識別子と同じ値になります。{/if}
  </p>

  <label>
    <span>{labelOf("base_persona")}</span>
    <textarea
      name="base_persona"
      rows="6"
      placeholder="葵は大学2年生。落ち着いた口調で話し、相手の話を最後まで聞く。"
      bind:value={draft.base_persona}
      disabled={saving}
    ></textarea>
    <span class="counter" data-counter="base_persona" class:over={over("base_persona")}>
      {counterOf("base_persona")}
    </span>
  </label>
  <p class="hint" data-hint="base_persona">
    キャラクターの設定と話し方を書きます。会話のたびにモデルへ渡ります。
  </p>

  <label>
    <span>{labelOf("output_format_instruction")}</span>
    <textarea
      name="output_format_instruction"
      rows="3"
      placeholder="台詞は「」で囲み、動作や情景は「」の外に書く。"
      bind:value={draft.output_format_instruction}
      disabled={saving}
    ></textarea>
    <span
      class="counter"
      data-counter="output_format_instruction"
      class:over={over("output_format_instruction")}
    >
      {counterOf("output_format_instruction")}
    </span>
  </label>
  <p class="hint" data-hint="output_format_instruction">
    返答の書き方の指示です。空にすると未設定に戻ります（この節はプロンプトから落ちます）。
  </p>

  <label>
    <span>性格ステータスの尺度</span>
    <!--
      **上下限を書かない**（`StatScale::MIN` / `MAX` を写さない）。
      範囲外はサーバが「小さすぎます」「大きすぎます」と返す。
    -->
    <input name="stat_scale" bind:value={draft.stat_scale} disabled={saving} />
  </label>
  <!--
    **何の上限か、と、省いたときの値**（作業単位 D-14 / Issue origin#30）。

    **⚠ 「10」は `StatScale::DEFAULT` の写しである**（`src/domain/stat.rs`）。
    省いたときに配られる値（`admin.rs` の `None => Ok(StatScale::DEFAULT)`）。
    **写し元を読む見張り番がある** —— `tests/character-form-examples.test.ts` が
    描かれた「省くと N になります」の N を写し元と突き合わせる。
    **編集では省けない**（空は「変えない」）ので、既定の値は言わない。
  -->
  <p class="hint" data-hint="stat_scale_meaning">
    性格ステータスの各軸に振れる値の上限です（0 からこの値まで）。{#if current === null}省くと 10 になります。{/if}
  </p>
  {#if current !== null}
    <!--
      **⚠ 案内は、実在する導線だけを指す**（作業単位 B-15 / D-7 と同じ立場）。

      **D-4 が入るまで、「性格値を下げる」操作はどこにも無かった。**
      いまは一覧の行の「性格値と変容」がそれで、**ここはその1つだけを名指しする。**

      **⚠ 場所を名乗らない案内を書かないこと**（元の文言がそうだった）——
      **どこで下げるのかを言わない案内は、実在するかどうかを誰も判定できない。**
      **見張り番は `tests/stat-scale-path.test.ts`** ——
      **「…」で名指しした操作が、その画面に実際に描かれていることを見る**
      （さらに、一覧から値を下げてここへ戻るまでを1本で辿る）。
    -->
    <p class="hint" data-hint="stat_scale">
      保存済みの値より小さい尺度には下げられません。先に一覧の「性格値と変容」から、
      その尺度を超えている値を下げてください。
    </p>
  {/if}

  <label>
    <span>{DRIFT_MODE_FIELD_LABEL}</span>
    <select name="stat_drift_mode" bind:value={draft.stat_drift_mode} disabled={saving}>
      {#each driftChoices as choice (choice.value)}
        <option value={choice.value}>{choice.label}</option>
      {/each}
      {#if !isKnownDriftModeChoice(draft.stat_drift_mode)}
        <!--
          **知らない綴りは、そのまま出す**（サーバが走らせ方を足した日に、
          選択欄が空白になって**触っていないのに別の値を送る**のを避ける）。
        -->
        <option value={draft.stat_drift_mode}>{draft.stat_drift_mode}</option>
      {/if}
    </select>
  </label>
  <!--
    **何を選ぶと何が起きるか**（作業単位 D-15 / ADR-0045）。

    **⚠ 出典（ADR の番号）を画面に書かないこと**（作業単位 F-8）。
    **「既定に従う」は、起動時の既定（サーバの設定）に従う** —— 括弧の中は
    変容の応答の `default_run_mode` から言う（新規作成では分からないので出さない）。
  -->
  <p class="hint" data-hint="stat_drift_mode">
    会話のあとで性格ステータスを少しずつ動かすかどうかです。変えると次のやり取りから効きます。
    既定に従うと、サーバの起動時の設定に従います。
  </p>

  {#if !built.ok}
    <p class="error" role="alert">{built.message}</p>
  {/if}
  {#if error !== null}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if notice !== null}
    <p class="notice" role="status">{notice}</p>
  {/if}

  <div class="actions">
    <button type="submit" disabled={!canSubmit}>
      {current === null ? "作成" : "保存"}
    </button>
    {#if onClose !== undefined}
      <button type="button" onclick={onClose} disabled={saving}>閉じる</button>
    {/if}
  </div>
</form>

<style>
  /*
    **枠・字・色は `src/app.css` にある**（作業単位 D-10）——
    `form` / `h3` / `form label` / 入力 / `.counter` / `.hint` / `.error` /
    `.notice` / `.actions` は、5〜6 の部品に同じ綴りで写っていたので、あちらへ寄せた。
    **ここに残すのは、この部品でしか成り立たない位置だけ。**
  */

  /* **見出しの列の幅にそろえて、入力の真下から始める。** */
  .hint {
    margin: 0 0 0.4rem 10.5rem;
  }
</style>
