<script lang="ts">
  /**
   * **`upstreams.json` に書いたプロファイル —— 選ぶ・足す・直す**（作業単位 F-4b-3 / Issue origin#24）。
   *
   * # 何ができて、何ができないか（ADR-0042 §決定5・8）
   *
   * | できる | できない |
   * |---|---|
   * | **一覧**（名前と、面ごとの種類と要点の1行） | **モデルを探す・落とす**（層2） |
   * | **切り替える**（`POST /admin/upstream/profile`） | **消す**（利用者のファイルから行を消す経路は、要ると分かってから決める） |
   * | **足す・直す**（`PUT /admin/upstream/profiles/{name}`。書ける置き場所だけ） | **名前を変える**（`PUT` は別の名前を足すだけで、元の名前が残る） |
   *
   * # 面ごとに「起こす」「繋ぐ」を選ぶ（作業単位 D-19 / Issue origin#87）
   *
   * **一覧は面ごとに種類の印と要点を1行で言う**（起こす: gguf のファイル名 → 待ち受けのポート / 繋ぐ: 宛先）。
   * **下書きは面ごとに種類を選ばせ、その種類の欄だけを出す**（`upstreamProfiles.ts` の `KIND_FIELDS`）。
   * **種類を切り替えても、もう一方の種類の欄の値は下書きに残す**（戻せば元に戻る）が、
   * **保存で送るのは選んだ種類の欄だけ**（繋ぐにすると gguf・起動の引数・実行ファイルはファイルから消える）。
   * ファイルに種類の鍵は無い —— サーバは gguf を書いた面を起こす。
   *
   * # 書いた値と観測した値を混ぜない（Issue origin#24 の閉じる条件）
   *
   * **この節は `upstreams.json` の生の値だけを出す**（一覧の gguf は書いたままのパスのファイル名）。
   * **上流が名乗った名前・窓・起こした状態は、上の節（`UpstreamPanel.svelte`）が出す。**
   * **この節の中に観測の値を1つも置かないこと** —— 見張り番は
   * `tests/upstream-profiles-panel.test.ts` の「書いた値と、上流に聞いた値が、別の節に分かれて出る」。
   *
   * **⚠ 例外は「選んでいます」の印だけ**（作業単位 D-19）。元は `GET /admin/upstream` の `selected_profile` だが、
   * **それは上流に聞いた値ではなく、この実行ファイルの切り替えの状態**（起動時に選んだ名前か、最後に受け付けた切り替えの名前）。
   * **値を1つも出さず、どの枠を強調するかにだけ使う。** 上流が載っているか・答えたかは言わない
   * （選んだが起こしていない `not_started` もある —— だから「動いています」ではなく「選んでいます」）。
   * **どれを選んでいるか分からないとき**（観測が読めない・選んでいない・一覧に無い名前）**は印を付けず、
   * 全部に「切り替える」を出す** —— 押せなくなる側に倒さない。
   * **選んでいる枠には「切り替える」の代わりに「切り替え直す」（普通のボタン）を状態によらず出す**（D-19 の是正）——
   * 同じ同名の差し替えで、保存した内容の反映と、`failed` / `stopped` からのやり直しに使う。
   *
   * # 保存は黙って上書きしない
   *
   * 保存の依頼には、**下書きを開いたときの一覧の印（`revision`）を添える**（`draftRevision`）。
   * 外で書き換えられていれば 409 が返り、**下書きを開いたまま理由と「読み直す」を出す。**
   * **読み直すと下書きは閉じる** —— 外で書き換えられた内容を先に見せ、直すならそこから開き直させる。
   * （下書きは開いたときの印を持つので、閉じずに残しても保存は必ず 409 になるだけ。
   * 閉じるのは、効かない下書きを押させる手間を省くため。）
   *
   * **足すときに在る名前を打ったら、送らない**（在る名前への `PUT` は、そのプロファイルを置き換える）。
   *
   * # 保存の失敗をどの欄に出すか
   *
   * * **詳細に畳んだ欄**（起こす面の起動の引数・実行ファイル・コンテキスト長）を名指した理由は、**その面の詳細を開いて**欄に出す。
   * * **その面の種類に無い欄**（繋ぐ面の gguf・起動の引数・実行ファイル）を名指した理由は、**下書き全体に出す**
   *   （`keepOnShownField`。出す先の無い欄に結びつけて、理由を消さない）。
   *
   * # 鍵の欄と、保存の前の確認（作業単位 A-13b-3 / Issue origin#86 / ADR-0056 §決定3・5）
   *
   * * **鍵の欄は両方の種類の欄**（`KIND_FIELDS` / レビュー是正）。**値を出さない**（伏せ字の空の欄 ＋ 設定済みか・末尾4文字）。
   *   新しい鍵を書けば置き換え、「鍵を消す」なら消し、触らなければ残す（依頼の文字列・`null`・送らない）。
   *   **書いた鍵は、下書きを閉じたとき・保存が通ったときに下書きから消す**（`forgetTypedKeys`）。
   * * **送る前に聞く**（`nextKeyQuestion`）—— 宛先だけを変えた鍵のある面（送る / 鍵を消す）、
   *   **IP アドレスで**ローカルネットワークの外の `http://` へ鍵を送る面（承認して保存する / 承認せずに保存する）。
   *   答えるまで `PUT` を送らず、「保存する」も押せない。確認が出たらフォーカスを確認へ移して巻き取り、閉じたら「保存する」へ戻す。
   * * **ホスト名の宛先では止めない**（レビュー是正）。鍵の欄の隣で「送るたびに名前解決して判定する」と言い、
   *   承認していなければ「承認する」を置く（危険を示して聞き、答えは保存で書く）。
   * * **ローカルネットワークの警告と承認の状態は、その面の鍵の欄の隣**（`keySendingState`）。
   * * **裏方のチェックを外すと、裏方の鍵も消えることをチェックの隣で言う**（振る舞いは変えない）。
   * * 宛先の分類は画面が綴りからする（理由は `upstreamKey.ts` の冒頭）。
   *
   * # 名前を変える操作を置かない理由
   *
   * 直すときの名前は変えられない（欄にしない）。**名前を変えるには「消す」が要る**が、
   * 消す操作は置かないと決めてある（ADR-0042 §決定8）。
   *
   * # 見た目（作業単位 D-19 / 利用者が承認した絵の Main・Edit）
   *
   * **ユーティリティと、土台の `.card` / `.primary` で組む**（ADR-0040 §決定4 —— 色・字・間隔は `<style>` に書かない）。
   * **`<style>` は持たない。**
   */

  import {
    ApiRequestError,
    listUpstreamProfiles,
    saveUpstreamProfile,
    switchUpstreamProfile,
    type ProfilesView,
    type UpstreamReport,
    type WrittenProfile,
  } from "./api";
  import {
    DEFAULT_COMMAND,
    KEY_INPUT_NOTE,
    KIND_FIELDS,
    NAME_TAKEN_MESSAGE,
    SAME_AS_MAIN_SUMMARY,
    answerKeyQuestion,
    buildSaveRequest,
    canEdit,
    clearKey,
    contextTokensFailure,
    describeSaveFailure,
    draftOfProfile,
    emptyProfileDraft,
    endpointV1Failure,
    faceKindBadge,
    faceKindLabel,
    faceLabel,
    failureOnScreen,
    fieldLabel,
    isProfilesView,
    joinedFlagFailure,
    keepKey,
    keepOnShownField,
    keySendingNote,
    keySendingState,
    keyStatusText,
    grantApproval,
    launchWithoutGgufFailure,
    listingNotices,
    nameTaken,
    nextKeyQuestion,
    resetApproval,
    revokeApproval,
    subKeyLossNote,
    summaryLines,
    type EditableFaceKey,
    type Face,
    type FaceKind,
    type KeyAnswer,
    type KeyQuestion,
    type ProfileDraft,
    type SaveFailure,
  } from "./upstreamProfiles";
  import { approvalQuestionText, carryQuestionText, nameApprovalText, type QuestionText } from "./upstreamKey";
  import { tick } from "svelte";

  let {
    onswitched,
    selected = null,
  }: {
    /** **切り替えの応答**（`GET /admin/upstream` と同じ形）を、観測の節へ渡す。 */
    onswitched: (report: UpstreamReport) => void;
    /**
     * **いま選ばれているプロファイルの名前**（`GET /admin/upstream` の `selected_profile` / 作業単位 D-19）。
     * **`null` は「分からない・選んでいない」**（印を付けず、全部に「切り替える」を出す）。
     */
    selected?: string | null;
  } = $props();

  /** 種類を選ぶボタンの並び（絵の Edit のまま、起こす → 繋ぐ）。 */
  const FACE_KINDS: readonly FaceKind[] = ["launch", "connect"];

  let listing = $state<ProfilesView | null>(null);
  let loadError = $state<string | null>(null);

  /** 切り替えの応答を待っているプロファイルの名前。 */
  let switching = $state<string | null>(null);
  let switchError = $state<string | null>(null);
  let switchNotice = $state<string | null>(null);

  /**
   * **開いている編集欄が何のためのものか**（作業単位 B-70）。
   * 直すときの `name` は、押した行のプロファイル名（欄をその行の直後に描き、閉じたらその行の「直す」へフォーカスを戻す）。
   * **直すときは名前を変えられない**（名前の欄は足すときだけ）ので、閉じた後も同じ名前で行を引ける。
   */
  type EditorOpened = { mode: "add" } | { mode: "edit"; name: string };

  /** 開いている下書き。**`null` は閉じている。** */
  let editing = $state<EditorOpened | null>(null);
  /** 「プロファイルを足す」ボタン（足す欄を閉じたら、フォーカスをここへ戻す / 作業単位 B-70）。 */
  let addButton = $state<HTMLButtonElement | null>(null);
  /** この節の根（閉じたときに、押した行の「直す」を名前で引く / 作業単位 B-70）。 */
  let root = $state<HTMLElement | null>(null);
  let draft = $state<ProfileDraft>(emptyProfileDraft());
  /**
   * **下書きを開いたときの一覧の印**（レビュー1周目 要修正3）。**保存にはこれを添える。**
   *
   * 下書きの値は開いたときの一覧から作る。**その後に読み直しが着いても、下書きの値は古いまま**なので、
   * いまの一覧の印を添えると、外の書き換えを古い値で黙って上書きする。
   * 開いたときの印を添えれば、書き換えられていればサーバが 409 で止める。
   */
  let draftRevision = $state<string | null>(null);
  let saving = $state(false);
  let failure = $state<SaveFailure | null>(null);
  /**
   * **保存の知らせ**（作業単位 B-69）。`name` は保存したプロファイル、`reswitch` は保存した時点でそれを選んでいたか
   * （**そのときだけ**知らせの中に「切り替え直す」を置く。ほかは今の文面）。
   * **ボタンは、`name` がいまも選ばれている間だけ出す**（レビュー是正1。下の `{#if}`）。
   * **切り替えが通ったら消す**（`select` —— 切り替えの結果の知らせに置き換わる。通らなければ残し、もう一度押せる）。
   */
  let saveNotice = $state<{ name: string; text: string; reswitch: boolean } | null>(null);
  /**
   * **面ごとの「詳細」が開いているか**（作業単位 D-19）。下書きを開くたびに閉じる。
   * **出している理由が詳細の中の欄に結びついていれば開く**（届いたときに限らない。下の `$effect`）。
   */
  let detailsOpen = $state<Record<Face, boolean>>({ main: false, sub: false });
  /**
   * **保存の前に聞いていること**（作業単位 A-13b-3 / `upstreamProfiles.ts` の `nextKeyQuestion`）。`null` は聞いていない。
   * 答えると下書きに書き（`answerKeyQuestion`）、保存を最初からやり直す —— 次に聞くことが無くなったら送る。
   */
  let question = $state<KeyQuestion | null>(null);
  /**
   * **ホスト名の宛先の承認を聞いている面**（鍵の欄の隣の「承認する」から / 作業単位 A-13b-3 のレビュー是正）。
   * `null` は聞いていない。答えは下書きに書き、保存で送る（ここでは送らない）。
   */
  let approvingName = $state<Face | null>(null);
  /**
   * **ホスト名の承認の確認が、いま実際に描かれているか**（作業単位 A-13b-3 の再レビュー是正）。
   *
   * 確認の描画・「保存する」の `disabled`・`save()` の早期 return の3か所は、**この1つの式**で決める。
   * 着手時は後ろの2か所が `approvingName !== null` しか見ておらず、確認を出したまま鍵を消す・宛先を変える・
   * 裏方のチェックを外すと、確認は消えるのに「保存する」が理由の出ないまま押せなくなった。
   */
  const nameApprovalShown = $derived(
    approvingName !== null &&
      editing !== null &&
      (approvingName === "main" || draft.hasSub) &&
      keySendingState(draft, approvingName).kind === "name",
  );

  // **確認の条件を外れたら、聞いている面を忘れる**（条件が戻っても、確認がひとりでに出直さない）。
  $effect(() => {
    if (approvingName !== null && !nameApprovalShown) {
      approvingName = null;
    }
  });
  /** 「保存する」ボタン（確認を閉じたら、フォーカスをここへ戻す）。 */
  let saveButton = $state<HTMLButtonElement | null>(null);

  /**
   * **確認が出たら、フォーカスを確認の見出しへ移し、確認が見えるように巻き取る**（作業単位 A-13b-3 のレビュー是正 /
   * 司令塔が 1024×768 で、確認が画面の下へはみ出し、フォーカスが「保存する」に残ったのを見た）。
   * **確認が閉じて、ほかの確認も出ていなければ、フォーカスを「保存する」へ戻す。**
   * 保存の確認は答えのたびに作り直す（`{#key}`）ので、次の確認が出ればそちらへ移る。
   */
  function revealDialog(node: HTMLElement): { destroy: () => void } {
    node.querySelector<HTMLElement>("[data-dialog-title]")?.focus();
    if (typeof node.scrollIntoView === "function") {
      node.scrollIntoView({ block: "nearest" });
    }
    return {
      destroy() {
        // **フォーカスが確認の中にあったときだけ戻す**（作業単位 A-13b-3 の再々レビュー是正）。
        // 確認は、利用者がほかの欄を打っている最中にも外れる（宛先を `https://` へ打ち直す途中など）。
        // そのときに戻すと、続けて打った文字が欄に入らない。
        // **確認の中にあったかは、消えた後に `document.activeElement` が `body` / `null` へ落ちたかで見る**
        // （確認の中の要素ごと消えるので、そこにあったフォーカスは落ちる。ほかの欄にあれば、そのまま残る）。
        // ⚠ 「`destroy` の時点で `node.contains(document.activeElement)`」は使えない —— `destroy` が呼ばれる時点で
        // 確認はもう DOM から外れていて、常に偽だった（その枝を外しても緑・残しても緑 / 変異 H3・H4 で実測）。
        void tick().then(() => {
          const lost = document.activeElement === null || document.activeElement === document.body;
          if (lost && question === null && approvingName === null) {
            saveButton?.focus();
          }
        });
      },
    };
  }

  /**
   * **編集欄が開いたら、見出しへフォーカスを移し、欄が見えるように巻き取る**（作業単位 B-70 / Issue origin#99 ——
   * 欄が一覧の一番下に出て巻き取りもせず、プロファイルが増えると押しても何も起きなかったように見えた）。
   * 作法は `revealDialog` と同じ。
   *
   * **閉じたら（保存・やめる・読み直す）、押したボタンへフォーカスを戻す** —— 直すならその名前の行の「直す」、
   * 足すなら「プロファイルを足す」。**行は閉じた時点の DOM から名前で引く**（保存で一覧が読み直されて並びが
   * 変わっても、同じプロファイルの行へ戻す）。その名前の行がもう無ければ戻さない。
   *
   * @param opened 開いたときの `editing`（**閉じた後は `editing` が `null` なので、開いたときの値を握っておく**）。
   */
  function revealEditor(node: HTMLElement, opened: EditorOpened): { destroy: () => void } {
    node.querySelector<HTMLElement>("[data-editor-title]")?.focus();
    if (typeof node.scrollIntoView === "function") {
      node.scrollIntoView({ block: "nearest" });
    }
    return {
      destroy() {
        // **フォーカスが欄の中にあったときだけ戻す**（`revealDialog` の destroy と同じ理由と見方 ——
        // 欄ごと消えると、そこにあったフォーカスは `body` / `null` へ落ちる。ほかの場所にあれば残る）。
        void tick().then(() => {
          const lost = document.activeElement === null || document.activeElement === document.body;
          if (!lost) {
            return;
          }
          if (opened.mode === "add") {
            addButton?.focus();
            return;
          }
          const back = [...(root?.querySelectorAll<HTMLButtonElement>("button[data-edit-profile]") ?? [])].find(
            (button) => button.dataset.editProfile === opened.name,
          );
          back?.focus();
        });
      },
    };
  }

  /**
   * **書いた新しい鍵を下書きに残さない**（作業単位 A-13b-3 のレビュー是正）。下書きを閉じるとき・保存が通ったときに空にする。
   */
  function forgetTypedKeys(): void {
    draft.main.api_key = "";
    draft.sub.api_key = "";
  }

  /** **この画面が生きている間**の中断ハンドル（`UpstreamPanel.svelte` と同じ理由）。 */
  const lifetime = new AbortController();

  $effect(() => {
    return () => {
      lifetime.abort();
    };
  });

  function messageOf(thrown: unknown): string {
    if (thrown instanceof ApiRequestError) {
      return thrown.message;
    }
    return "予期しない失敗です";
  }

  /** **一覧を読み直す。** 開いている下書きは閉じる（読み直した内容から開き直させる）。 */
  async function load(): Promise<void> {
    loadError = null;
    editing = null;
    failure = null;
    question = null;
    approvingName = null;
    forgetTypedKeys();
    try {
      const next: unknown = await listUpstreamProfiles(lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      // **形の違う応答で画面ごと落とさない**（`isProfilesView` の doc）。
      if (!isProfilesView(next)) {
        listing = null;
        loadError = "一覧の応答を読めませんでした";
        return;
      }
      listing = next;
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      listing = null;
      loadError = messageOf(thrown);
    }
  }

  async function select(name: string): Promise<void> {
    // **送った時点で出ていた保存の知らせ**（レビュー是正3）。待つ間に保存した新しい知らせは、この切り替えに
    // 入っていないかもしれないので、応答で消さない。
    const before = saveNotice;
    switching = name;
    switchError = null;
    switchNotice = null;
    try {
      const next = await switchUpstreamProfile(name, lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      onswitched(next);
      // **保存の知らせは、切り替えの結果に置き換える**（作業単位 B-69）。「切り替え直してください」を残さない ——
      // 一覧の行から切り替えたときも、知らせが言う「いま動いている上流」が変わったので同じ。
      if (saveNotice === before) {
        saveNotice = null;
      }
      switchNotice = `「${name}」に切り替えました。起こした上流の状態は、上の表に出ます。`;
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      switchError = `切り替えられませんでした: ${messageOf(thrown)}`;
    } finally {
      switching = null;
    }
  }

  function openAdd(): void {
    draft = emptyProfileDraft();
    draftRevision = listing?.revision ?? null;
    failure = null;
    saveNotice = null;
    question = null;
    approvingName = null;
    detailsOpen = { main: false, sub: false };
    editing = { mode: "add" };
  }

  function openEdit(profile: WrittenProfile): void {
    draft = draftOfProfile(profile);
    draftRevision = listing?.revision ?? null;
    failure = null;
    saveNotice = null;
    question = null;
    approvingName = null;
    detailsOpen = { main: false, sub: false };
    editing = { mode: "edit", name: profile.name };
  }

  function close(): void {
    editing = null;
    failure = null;
    question = null;
    approvingName = null;
    forgetTypedKeys();
  }

  function setField(face: Face, key: EditableFaceKey, value: string): void {
    draft[face][key] = value;
  }

  /** **種類を選ぶ。** 下書きの値は動かさない（もう一方の種類の欄の値も残る）。 */
  function setKind(face: Face, kind: FaceKind): void {
    draft.kind[face] = kind;
  }

  /**
   * **保存の失敗を出す。** その面の種類に無い欄を名指したものは全体へ（`keepOnShownField`）。
   * 詳細に畳んだ欄を名指したものは、下の `$effect` がその面の詳細を開く。
   *
   * @param kind 送ったときの面ごとの種類（保存を待つ間は種類を変えられないが、送った形で決める）。
   */
  function fail(next: SaveFailure, kind: Record<Face, FaceKind>): void {
    failure = keepOnShownField(next, kind);
  }

  /**
   * **出している理由が「詳細」に畳んだ欄に結びついていれば、その面の詳細を開く**（D-19 の再レビュー）。
   *
   * **理由が届いたときに限らない** —— コンテキスト長はどちらの種類にもあるので、繋ぐ面の欄に理由が出た後で
   * 起こすへ切り替えると、理由は欄に結びついたまま、閉じた詳細の中へ移る。だから「出している理由」
   * （`shownFailure`。種類と裏方のチェックを読む）が変わるたびに見る。
   * **開いた詳細を利用者が閉じるのは止めない**（この処理は詳細の開閉を読まないので、閉じても開き直さない）。
   */
  $effect(() => {
    const shown = shownFailure();
    if (shown?.kind === "field" && KIND_FIELDS[draft.kind[shown.face]].folded.includes(shown.key)) {
      detailsOpen[shown.face] = true;
    }
  });

  function save(event: SubmitEvent): void {
    event.preventDefault();
    // **確認が出ている間は送らない**（「保存する」は押せないが、入力欄の Enter でも submit は来る）。
    // 通すと同じ確認を作り直し、利用者が見ていた確認が別物に替わる。
    if (question !== null || nameApprovalShown) {
      return;
    }
    void proceed();
  }

  /**
   * **確認に答える**（作業単位 A-13b-3）。答えを下書きに書き、**保存を最初からやり直す**
   * （次に聞くことがあれば聞き、無ければ送る）。答えは聞いたときの宛先に付く（`answerKeyQuestion`）。
   */
  function answer(reply: KeyAnswer): void {
    if (question === null) {
      return;
    }
    answerKeyQuestion(draft, question, reply);
    question = null;
    void proceed();
  }

  /** **確認を取り消す**（送らない。下書きは開いたまま）。 */
  function cancelQuestion(): void {
    question = null;
  }

  /** 確認の文面（面の見出しは `faceLabel`）。 */
  function questionTextOf(asked: KeyQuestion): QuestionText {
    return asked.kind === "carry"
      ? carryQuestionText(faceLabel(asked.face), asked.hint, asked.from, asked.to)
      : approvalQuestionText(faceLabel(asked.face), asked.endpoint);
  }

  /**
   * **保存の本体。** 送る前に止める検査 → **保存の前の確認**（作業単位 A-13b-3）→ 送る。
   *
   * **確認は検査の後に聞く**（直せば通らない下書きに、鍵の確認を先に答えさせない）。
   */
  async function proceed(): Promise<void> {
    if (listing === null || editing === null) {
      return;
    }
    failure = null;
    const kind = { ...draft.kind };
    if (editing.mode === "add" && nameTaken(draft.name, listing.profiles)) {
      failure = { kind: "name", message: NAME_TAKEN_MESSAGE };
      return;
    }
    // **起こす面の gguf が空なら、送らずにその欄で言う**（D-19 のレビュー / `launchWithoutGgufFailure` の doc）——
    // 送ると黙って繋ぐ面として書かれる。
    const launchless = launchWithoutGgufFailure(draft);
    if (launchless !== null) {
      fail(launchless, kind);
      return;
    }
    // **旗と値を1行に書いた引数は、送らずにその欄で言う**（作業単位 D-13 / `joinedFlagFailure` の doc）。
    const joined = joinedFlagFailure(draft);
    if (joined !== null) {
      fail(joined, kind);
      return;
    }
    // **数として読めないコンテキスト長も、送らずにその欄で言う**（作業単位 A-13a-2 / `contextTokensFailure` の doc）。
    const window = contextTokensFailure(draft);
    if (window !== null) {
      fail(window, kind);
      return;
    }
    // **末尾が /v1 の宛先も、送らずにその欄で言う**（作業単位 B-68 / `endpointV1Failure` の doc）——
    // 送ると、サーバは受け付けて …/v1/v1/… に送る。鍵の確認より先に止める（直せば通らない下書きに答えさせない）。
    const versioned = endpointV1Failure(draft);
    if (versioned !== null) {
      fail(versioned, kind);
      return;
    }
    // **鍵の確認と平文送信の承認は、送る前に聞く**（作業単位 A-13b-3 / 利用者の決定1・ADR-0056 §決定5）。
    approvingName = null;
    const asked = nextKeyQuestion(draft);
    if (asked !== null) {
      question = asked;
      return;
    }
    question = null;
    const name = draft.name;
    const hasSub = draft.hasSub;
    const body = buildSaveRequest(draft, draftRevision);
    saving = true;
    try {
      // **返るのは、サーバが書いた直後に読み直した一覧。** それをそのまま出す。
      const next: unknown = await saveUpstreamProfile(name, body, lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      if (!isProfilesView(next)) {
        // **保存は通ったかもしれない**ので、そうは言わない。読み直させる。
        failure = {
          kind: "form",
          message: "保存の応答を読めませんでした。読み直して、保存されたかを確かめてください。",
        };
        return;
      }
      listing = next;
      editing = null;
      forgetTypedKeys();
      // **選んでいるプロファイルを保存したら、知らせの中に「切り替え直す」を置く**（作業単位 B-69 / Issue origin#98 ——
      // 「切り替え直すまで変わりません」を読んでも、次に何を押すかに辿り着きにくかった）。
      // **ほかのプロファイルでは置かない** —— 別のプロファイルへ切り替えるのは大きい判断なので、一覧の行のボタンに任せる。
      saveNotice =
        name === selected
          ? {
              name,
              text: `「${name}」を保存しました。いま動いている上流に反映するには、切り替え直してください。`,
              reswitch: true,
            }
          : {
              name,
              text: `「${name}」を保存しました。いま動いている上流は、このプロファイルに切り替えるまで変わりません。`,
              reswitch: false,
            };
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      fail(describeSaveFailure(thrown, hasSub), kind);
    } finally {
      saving = false;
    }
  }

  /**
   * **いま画面に出す理由**（D-19 のレビュー）。欄に結びついた理由でも、その欄が画面から消えていれば
   * （種類を切り替えた・裏方のチェックを外した）下書き全体へ移す（`failureOnScreen`）。戻せば欄へ戻る。
   */
  function shownFailure(): SaveFailure | null {
    return failure === null ? null : failureOnScreen(failure, draft);
  }

  /** その欄に結びつく理由（無ければ `null`）。 */
  function fieldFailure(face: Face, key: EditableFaceKey): string | null {
    const shown = shownFailure();
    return shown?.kind === "field" && shown.face === face && shown.key === key ? shown.message : null;
  }

  function nameFailure(): string | null {
    return failure?.kind === "name" && editing?.mode === "add" ? failure.message : null;
  }

  /** 下書き全体に出す理由（名前の欄が無い「直す」では、名前の理由もここ）。 */
  function formFailure(): SaveFailure | null {
    const shown = shownFailure();
    if (shown === null || shown.kind === "field") {
      return null;
    }
    if (shown.kind === "name" && editing?.mode === "add") {
      return null;
    }
    return shown;
  }

  /**
   * **引数の欄の見本**（作業単位 D-13）。旗と値を別の行に書く形を、入力する前に見せる。
   * `textarea` の `placeholder` の改行は、行として描かれる。
   */
  const ARGS_PLACEHOLDER = ["--ctx-size", "16384", "--jinja"].join("\n");

  /**
   * **欄の隣に置く注記**（作業単位 D-19 —— 着手前は下書きの末尾に、どの面の話かを言わずに並んでいた）。
   * **注記は、それに関係する欄の隣にだけ置く。**
   */
  const WINDOW_NOTE =
    "コンテキスト長は、上流がコンテキスト長を答えないとき（鍵で守られた中継など）だけ、上流で選んだ窓のトークン数を書きます。空にすると書かず、上流に聞きます。";

  function notesOf(face: Face, kind: FaceKind, key: EditableFaceKey): string[] {
    switch (key) {
      case "endpoint":
        // **起こす面の宛先はループバックだけ**（サーバの保存の検証 `endpoint_not_loopback` と同じ集合）。
        // **どちらの面も、ホストとポートまで書く**（作業単位 B-68 / Issue origin#97 —— `/v1` まで書くと
        // `/v1/v1/…` に送る。保存の前の止めは `endpointV1Failure`）。
        return kind === "launch"
          ? [
              "起こした llama がここで待ち受けます。この機の中（127.0.0.1 / localhost / ::1）だけです。",
              "ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1 は personacore が足すので書きません。",
            ]
          : [
              "ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1/chat/completions などは personacore が足すので、中継の説明書きが …/v1 までを示していても、/v1 は書きません。パスの前置き（https://example.com/api など）は書けます。",
            ];
      case "args":
        return [
          "起動の引数は1行に1つ書きます。オプションとその値も別の行です（--ctx-size と 16384、-ngl と 99 は別の行。1行に続けて書くと、空白ごと1つの引数として渡り、上流が起動しません）。オプションの行の先頭に空白を入れないでください。空白を含むパスは、そのまま1行に書けます。空にすると書きません。",
        ];
      case "command":
        return [`空にすると書かず、${DEFAULT_COMMAND} を起こします。`];
      case "context_tokens":
        // **裏方を別にしないときの使われ方は、本文の面でだけ言う。**
        return face === "main"
          ? [`${WINDOW_NOTE}裏方を別の上流にしなければ、本文の値が抽出にも使われます。`]
          : [WINDOW_NOTE];
      case "api_key":
        return [KEY_INPUT_NOTE];
      case "gguf":
      case "model":
        return [];
    }
  }

  /**
   * **種類を切り替えたときの挙動**（作業単位 D-19）。種類を選ぶ組の隣に、面ごとに出す。
   * **鍵は両方の種類の欄**なので、種類を切り替えても消えない（作業単位 A-13b-3 のレビュー是正で、着手時に足した
   * 「起こすにすると、鍵はファイルから消えます」を外した）。
   */
  const KIND_NOTE =
    "保存で書くのは、選んだ種類の欄だけです。繋ぐにすると、gguf・起動の引数・実行ファイルはファイルから消えます（種類を戻せば、下書きの値は元に戻ります）。";

  void load();
</script>

{#snippet field(face: Face, key: EditableFaceKey)}
  {@const id = `profile-${face}-${key}`}
  {@const problem = fieldFailure(face, key)}
  {@const notes = notesOf(face, draft.kind[face], key)}
  <!--
    **endpoint の欄は、注記（何を書くかの案内）も欄の説明として結ぶ**（作業単位 B-68 / Issue origin#97）。
    理由が先、案内が後。ほかの欄は今までどおり理由だけを結ぶ。
  -->
  {@const describedBy =
    [
      ...(problem === null ? [] : [`${id}-error`]),
      ...(key === "endpoint" ? notes.map((_, index) => `${id}-note-${index}`) : []),
    ].join(" ") || undefined}
  <div class="flex flex-col gap-1.5">
    <!-- **`block`** —— 土台の `form label` は「見出し / 入力 / 文字数」の格子で、見出しが 10rem の列で折り返す。 -->
    <label class="block text-sm font-semibold" for={id}>{fieldLabel(draft.kind[face], key)}</label>
    {#if key === "args"}
      <textarea
        {id}
        class="w-full font-mono"
        rows="3"
        placeholder={ARGS_PLACEHOLDER}
        value={draft[face][key]}
        aria-invalid={problem === null ? undefined : "true"}
        aria-describedby={describedBy}
        oninput={(event) => setField(face, key, event.currentTarget.value)}
      ></textarea>
    {:else if key === "api_key"}
      <!--
        **鍵の欄**（作業単位 A-13b-3 / ADR-0056 §決定3）。**値は出さない** —— 開いたときは必ず空で、伏せ字の欄。
        **書いたときだけ置き換え、空なら残す。** 「鍵を消す」の間は書けない（消すほうが勝つ）。
        閲覧器のパスワードの保存・補完に拾わせない（`autocomplete="off"`）。
      -->
      <input
        {id}
        class="w-full font-mono"
        type="password"
        autocomplete="off"
        spellcheck="false"
        value={draft[face].api_key}
        disabled={draft.keys[face].cleared}
        aria-invalid={problem === null ? undefined : "true"}
        aria-describedby={describedBy}
        oninput={(event) => setField(face, key, event.currentTarget.value)}
      />
    {:else}
      <!--
        **コンテキスト長も `type="text"`**（`inputmode` で数字の鍵盤だけを促す）。
        `type="number"` だと、読めない入力の値が空文字になり「書かない」として黙って送られる。
      -->
      <input
        {id}
        class="w-full font-mono"
        type="text"
        inputmode={key === "context_tokens" ? "numeric" : undefined}
        value={draft[face][key]}
        aria-invalid={problem === null ? undefined : "true"}
        aria-describedby={describedBy}
        oninput={(event) => setField(face, key, event.currentTarget.value)}
      />
    {/if}
    {#if problem !== null}
      <!-- **`role="alert"` は付けない**（着手前のまま。欄には `aria-describedby` で結びつく）。色だけを「壊れている」側にする。 -->
      <p class="field-error m-0 text-warn" id={`${id}-error`}>{problem}</p>
    {/if}
    {#each notes as note, index (index)}
      <p class="note m-0" id={`${id}-note-${index}`}>{note}</p>
    {/each}
    {#if key === "api_key"}
      {@render keyExtras(face)}
    {/if}
  </div>
{/snippet}

<!--
  **鍵の欄の隣に出すもの**（作業単位 A-13b-3）—— 設定済みか（末尾4文字）・消す / 消すのをやめる・
  送り方の状態（ローカルネットワークの警告 / 承認の状態）と、承認を外す / 取り消す。
  **どれも、その面の鍵の欄の囲みの中に置く**（どの面の話かを取り違えない）。
-->
{#snippet keyExtras(face: Face)}
  {@const keys = draft.keys[face]}
  {@const sending = keySendingState(draft, face)}
  {@const sendingNote = keySendingNote(sending)}
  <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
    <p class="key-status m-0 text-sm">{keyStatusText(draft, face)}</p>
    {#if keys.written?.set === true}
      {#if keys.cleared}
        <button type="button" disabled={saving} onclick={() => keepKey(draft, face)}>消すのをやめる</button>
      {:else}
        <button type="button" disabled={saving} onclick={() => clearKey(draft, face)}>鍵を消す</button>
      {/if}
    {/if}
  </div>
  {#if sendingNote !== null}
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <!-- **暗号化せずに送る話は、警告の色で言う**（ローカルネットワークの警告・外へ送る承認）。 -->
      <p class="key-sending m-0 text-sm text-warn">{sendingNote}</p>
      {#if sending.kind === "kept"}
        <button type="button" disabled={saving} onclick={() => revokeApproval(draft, face)}>承認を外す</button>
      {:else if sending.kind === "granting" || sending.kind === "denying"}
        <button type="button" disabled={saving} onclick={() => resetApproval(draft, face)}>取り消す</button>
      {:else if sending.kind === "name" && approvingName !== face}
        <button
          type="button"
          disabled={saving || question !== null || nameApprovalShown}
          onclick={() => {
            approvingName = face;
          }}>承認する</button
        >
      {/if}
    </div>
  {/if}
  <!--
    **ホスト名の宛先の承認の確認**（作業単位 A-13b-3 のレビュー是正）。保存の前には止めないので、ここで聞く。
    危険の文は保存の前の確認と同じ（`upstreamKey.ts`）。答えは下書きに書き、保存で送る。
  -->
  {#if nameApprovalShown && approvingName === face}
    {@const text = nameApprovalText(faceLabel(face), draft[face].endpoint)}
    <div
      class="card flex flex-col gap-3 border-warn"
      role="alertdialog"
      aria-labelledby={`name-approval-${face}-title`}
      aria-describedby={`name-approval-${face}-body`}
      use:revealDialog
    >
      <h4 id={`name-approval-${face}-title`} class="m-0 text-base" tabindex="-1" data-dialog-title>{text.title}</h4>
      <div id={`name-approval-${face}-body`} class="flex flex-col gap-1.5">
        {#each text.body as line, index (index)}
          <p class="m-0">{line}</p>
        {/each}
      </div>
      <div class="actions flex-wrap">
        <button
          type="button"
          onclick={() => {
            grantApproval(draft, face);
            approvingName = null;
          }}>承認する</button
        >
        <button
          type="button"
          onclick={() => {
            approvingName = null;
          }}>やめる</button
        >
      </div>
    </div>
  {/if}
{/snippet}

{#snippet faceFields(face: Face)}
  {@const kind = draft.kind[face]}
  <!-- **`min-w-0`** —— `fieldset` は既定で中身より縮まないので、狭い幅で横へはみ出す。 -->
  <fieldset class="card m-0 flex min-w-0 flex-col gap-4">
    <legend class="px-2 text-base font-bold">{faceLabel(face)}</legend>
    <!--
      **種類を選ぶ**（絵の Edit の切り替え）。**押せるもので、`aria-pressed` で状態を名乗る。**
      見た目の「押している」も同じ属性から出す（`aria-pressed:` の変位）——読み上げと目の印が1つの条件から出る。
      **⚠ 角丸と枠は外側の組が持つ**（ボタンの土台の角丸・枠を打ち消す）。
    -->
    <div
      role="group"
      aria-label={`${faceLabel(face)}の上流の種類`}
      class="flex w-fit max-w-full flex-wrap overflow-hidden rounded-lg border border-rule-2"
    >
      {#each FACE_KINDS as option (option)}
        <button
          type="button"
          class="min-h-11 rounded-none border-0 px-4.5 text-ink-2 aria-pressed:bg-ink aria-pressed:font-semibold aria-pressed:text-paper"
          aria-pressed={kind === option ? "true" : "false"}
          disabled={saving}
          onclick={() => setKind(face, option)}>{faceKindLabel(option)}</button
        >
      {/each}
    </div>
    <p class="note m-0">{KIND_NOTE}</p>

    {#each KIND_FIELDS[kind].shown as key (key)}
      {@render field(face, key)}
    {/each}
    {#if KIND_FIELDS[kind].folded.length > 0}
      <details bind:open={detailsOpen[face]}>
        <summary class="cursor-pointer text-sm text-ink-2">詳細（起動の引数・実行ファイル・コンテキスト長）</summary>
        <div class="mt-3.5 flex flex-col gap-4">
          {#each KIND_FIELDS[kind].folded as key (key)}
            {@render field(face, key)}
          {/each}
        </div>
      </details>
    {/if}
  </fieldset>
{/snippet}

<section class="mt-8 flex flex-col gap-3" aria-labelledby="upstream-profiles-heading" bind:this={root}>
  <h2 id="upstream-profiles-heading" class="m-0">upstreams.json に書いたプロファイル</h2>
  <p class="note m-0">
    この節の値は、ファイルに書いたままの値です。上流が名乗った名前やコンテキスト長は、上の節にあります。
    モデルを探したり、ダウンロードしたりはしません。gguf のパスは自分で書いてください。
  </p>

  {#if loadError !== null}
    <p class="error m-0" role="alert">プロファイルの一覧を取得できませんでした: {loadError}</p>
  {:else if listing === null}
    <p class="muted m-0" role="status">読み込み中…</p>
  {:else}
    {#each listingNotices(listing) as notice, index (index)}
      <p class="listing-notice m-0 break-all">{notice}</p>
    {/each}
  {/if}

  <div class="actions flex-wrap">
    <button type="button" onclick={() => void load()}>読み直す</button>
    {#if listing !== null && canEdit(listing)}
      <button type="button" class="primary" onclick={openAdd} disabled={editing !== null} bind:this={addButton}
        >プロファイルを足す</button
      >
    {/if}
  </div>

  <!--
    **足す欄は「プロファイルを足す」の並びの直後**（作業単位 B-70）。切り替え・保存の知らせはこの下のまま ——
    一覧の行から切り替えたときの知らせは、今までどおり一覧のすぐ上に出る（開くと保存の知らせは消える / `openAdd`）。
  -->
  {#if editing?.mode === "add"}
    {@render editor(editing)}
  {/if}

  {#if switchError !== null}
    <p class="error m-0" role="alert">{switchError}</p>
  {/if}
  {#if switchNotice !== null}
    <p class="notice m-0" role="status">{switchNotice}</p>
  {/if}
  {#if saveNotice !== null}
    <!--
      **知らせの「切り替え直す」は、一覧の行の「切り替え直す」と同じ `select` を呼ぶ**（作業単位 B-69）。
      **2つ目の切り替えの経路を作らない** —— 待つ間の無効化（`switching`）とエラーの出し方（`switchError`）も同じになる。
      **ボタンは、保存したプロファイルがいまも選ばれている間だけ出す**（レビュー是正1）。別のタブ等で切り替わって
      読み直しが `selected` を変えたら消す —— 残すと、押したときに選び直された上流を保存した側へ戻してしまう。
      **消えたら、文面は「保存しました。」だけにする**（「切り替え直してください」は、もう選んでいないプロファイルには偽）。
      **名前（`aria-label`）はプロファイル名を名乗る**（レビュー是正5 —— 同じ節に、一覧の行の「切り替え直す」も並ぶ）。
    -->
    {@const notice = saveNotice}
    <p class="notice m-0" role="status">
      {#if !notice.reswitch}{notice.text}{:else if notice.name === selected}{notice.text}{" "}<button
          type="button"
          aria-label={`「${notice.name}」を切り替え直す`}
          disabled={switching !== null}
          onclick={() => void select(notice.name)}>切り替え直す</button
        >{:else}「{notice.name}」を保存しました。{/if}
    </p>
  {/if}

  {#if listing !== null && listing.profiles.length > 0}
    <p class="note m-0">
      切り替えると、「起こす」面はこの実行ファイルが gguf を載せて起こします（実行ファイルを書いていなければ
      {DEFAULT_COMMAND}）。「繋ぐ」面は起こさず、宛先と依頼するモデルの綴りだけを差し替えます。
    </p>
    <div class="flex flex-col gap-3">
      {#each listing.profiles as profile, index (profile.name)}
        {@const chosen = selected !== null && profile.name === selected}
        <!--
          **1件を枠（`.card`）にする**（絵の Main）。**選んでいる1件は枠を強調し、`aria-current` で名乗る。**
          **見出し（`h3`）は枠の直接の子に置く**（一覧を読む側は、名前で枠を引く）。
        -->
        <section
          class={[
            "profile card flex flex-wrap items-center gap-x-2.5 gap-y-3",
            chosen && "border-2 border-read",
          ]}
          aria-labelledby={`upstream-profile-${index}`}
          aria-current={chosen ? "true" : undefined}
        >
          <h3 id={`upstream-profile-${index}`} class="m-0 font-mono text-lg break-all">{profile.name}</h3>
          {#if chosen}
            <span class="rounded-full bg-read px-2 py-0.5 text-xs font-semibold text-paper">選んでいます</span>
          {/if}
          <dl class="m-0 flex basis-full flex-col gap-1.5 text-sm">
            {#each summaryLines(profile) as line (line.face)}
              <div class="flex gap-3">
                <dt class="w-14 shrink-0 text-muted">{line.label}</dt>
                <dd class="m-0 min-w-0">
                  {#if line.summary === null}
                    <span class="text-muted">{SAME_AS_MAIN_SUMMARY}</span>
                  {:else}
                    <span
                      class="inline-flex items-center rounded-full border border-rule-2 px-2 text-xs whitespace-nowrap text-ink-2"
                      >{faceKindBadge(line.summary.kind)}</span
                    >{" "}<span class="font-mono break-all">{line.summary.target}</span
                    >{#each line.summary.extras as extra, extraIndex (extraIndex)}<span class="text-muted"
                        >{" · "}{extra}</span
                      >{/each}
                  {/if}
                </dd>
              </div>
            {/each}
          </dl>
          <div class="actions ml-auto flex-wrap">
            <!--
              **選んでいる枠には「切り替え直す」（普通のボタン）**（D-19 の是正）。押したときの動きは
              「このプロファイルに切り替える」と同じ同名の差し替え。**状態によらず出す** ——
              保存した内容を反映するとき、起こした上流が `failed` / `stopped` になったときにやり直す手段を消さない。
            -->
            {#if chosen}
              <button type="button" disabled={switching !== null} onclick={() => void select(profile.name)}
                >切り替え直す</button
              >
            {:else}
              <button
                type="button"
                class="primary"
                disabled={switching !== null}
                onclick={() => void select(profile.name)}>このプロファイルに切り替える</button
              >
            {/if}
            {#if canEdit(listing)}
              <button
                type="button"
                disabled={editing !== null}
                data-edit-profile={profile.name}
                onclick={() => openEdit(profile)}>直す</button
              >
            {/if}
          </div>
        </section>
        <!--
          **直す欄はその行（枠）の直後に、枠の兄弟として描く**（作業単位 B-70）。枠の中には入れない ——
          枠は名前の見出しで名乗る塊（`aria-labelledby`）で、横に流す並び（`flex-wrap`）でもある。
          中に入れると、塊の中に別の見出しの欄が混ざり、選んでいる枠の強調も欄まで囲む。
        -->
        {#if editing?.mode === "edit" && editing.name === profile.name}
          {@render editor(editing)}
        {/if}
      {/each}
    </div>
  {/if}

  {#if switching !== null}
    <p class="muted m-0" role="status">「{switching}」に切り替えています…</p>
  {/if}
</section>

<!--
  **編集欄**（作業単位 B-70 / Issue origin#99）。**描く場所は押した場所のすぐ下** —— 直すならその行（枠）の直後、
  足すなら「プロファイルを足す」の並びの直後。**欄そのものは1つ**（この snippet と下書きの状態を使い回し、描く場所だけ変える）。
  開いたら見出しへフォーカスを移して巻き取り、閉じたら押したボタンへ戻す（`revealEditor`）。
-->
{#snippet editor(opened: EditorOpened)}
  <form class="gap-5" onsubmit={(event) => void save(event)} use:revealEditor={opened}>
    <h3 tabindex="-1" data-editor-title>{opened.mode === "add" ? "プロファイルを足す" : `「${draft.name}」を直す`}</h3>
    {#if opened.mode === "add"}
      {@const problem = nameFailure()}
      <div class="flex flex-col gap-1.5">
        <label class="block text-sm font-semibold" for="profile-name">名前</label>
        <input
          id="profile-name"
          class="w-full font-mono"
          type="text"
          value={draft.name}
          aria-invalid={problem === null ? undefined : "true"}
          aria-describedby={problem === null ? undefined : "profile-name-error"}
          oninput={(event) => {
            draft.name = event.currentTarget.value;
          }}
        />
        {#if problem !== null}
          <p class="field-error m-0 text-warn" id="profile-name-error">{problem}</p>
        {/if}
        <p class="note m-0">
          名前は、起動するときの --upstream &lt;名前&gt; でも選べる綴りにしてください。
        </p>
      </div>
    {/if}

    {@render faceFields("main")}

    <!--
      **箱は `label` の直接の子**（土台の `label:has(> input[type=checkbox])` が箱と文言を横に並べる /
      見張り番は `tests/base-layers.test.ts`）。

      **チェックを外したとき、裏方に鍵があれば、隣で「保存すると裏方の鍵も消えます」と言う**
      （作業単位 A-13b-3 / 利用者の決定2）。**振る舞いは変えない**（`sub: null` で裏方の面ごと消える）。
    -->
    <div class="flex flex-wrap items-center gap-x-3">
      <label>
        <input
          type="checkbox"
          checked={draft.hasSub}
          onchange={(event) => {
            draft.hasSub = event.currentTarget.checked;
          }}
        />
        裏方（抽出・変容）を別の上流にする
      </label>
      {#if subKeyLossNote(draft) !== null}
        <span class="text-sm text-warn">{subKeyLossNote(draft)}</span>
      {/if}
    </div>
    {#if draft.hasSub}
      {@render faceFields("sub")}
    {:else}
      <p class="note m-0">書かなければ、裏方は本文と同じ上流を使います。</p>
    {/if}

    {#if formFailure() !== null}
      {@const shown = formFailure()}
      <p class="error m-0" role="alert">
        {shown?.message}
        {#if shown?.kind === "changed"}
          <button type="button" onclick={() => void load()}>読み直す</button>
        {/if}
      </p>
    {/if}

    <!--
      **保存の前の確認**（作業単位 A-13b-3）—— 鍵を新しい宛先へも送るか（利用者の決定1）／
      鍵を暗号化せずに送ることを承認するか（ADR-0056 §決定5）。**答えるまで送らない。**
      **`confirm()` は使わない**（jsdom で観測できない / D-7 と同じ立場）。**どちらの答えも主ボタンにしない**
      （承認する側へ寄せない）。
    -->
    {#if question !== null}
      <!--
        **答えのたびに作り直す**（`{#key}`）—— 次の確認が出たとき、フォーカスと巻き取りをやり直す（`revealDialog`）。
      -->
      {#key question}
      {@const text = questionTextOf(question)}
      <div
        class="card flex flex-col gap-3 border-warn"
        role="alertdialog"
        aria-labelledby="key-question-title"
        aria-describedby="key-question-body"
        use:revealDialog
      >
        <h4 id="key-question-title" class="m-0 text-base" tabindex="-1" data-dialog-title>{text.title}</h4>
        <div id="key-question-body" class="flex flex-col gap-1.5">
          {#each text.body as line, index (index)}
            <p class="m-0">{line}</p>
          {/each}
        </div>
        <div class="actions flex-wrap">
          {#if question.kind === "carry"}
            <button type="button" onclick={() => answer("carry")}>送る</button>
            <button type="button" onclick={() => answer("clear")}>鍵を消す</button>
          {:else}
            <button type="button" onclick={() => answer("grant")}>承認して保存する</button>
            <button type="button" onclick={() => answer("deny")}>承認せずに保存する</button>
          {/if}
          <!-- **下書きの「やめる」（閉じる）と別の綴りにする**（同じ下書きの中に並ぶ）。 -->
          <button type="button" onclick={cancelQuestion}>保存をやめる</button>
        </div>
      </div>
      {/key}
    {/if}

    <div class="actions flex-wrap items-center border-t border-rule pt-5">
      <!-- **確認が出ている間は押せない**（作業単位 A-13b-3 のレビュー是正。答えるか「保存をやめる」まで送らない）。 -->
      <button
        type="submit"
        class="primary"
        disabled={saving || question !== null || nameApprovalShown}
        bind:this={saveButton}>保存する</button
      >
      <button type="button" onclick={close} disabled={saving}>やめる</button>
      <p class="note m-0">保存しても、切り替えるまで動いている上流は変わりません。</p>
    </div>
  </form>
{/snippet}
