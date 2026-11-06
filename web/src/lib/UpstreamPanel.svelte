<script lang="ts">
  /**
   * **上流を見る画面**（作業単位 D-9 / `GET /admin/upstream`）。
   *
   * # なぜこの画面が要るのか
   *
   * **着手前、画面に「モデル」に見えるものは1つしか無かった** ——
   * 会話の画面の `送り先: aoi@1`。**それはセッションの識別子**であって、
   * **本文を書いているモデルの綴りではない。**
   * `GET /v1/models` が返すのも**キャラクター**なので、
   * **上流のモデルは画面のどこにも出ていなかった。**
   *
   * **並べて初めて、2つが別物だと分かる。**
   *
   * # 選ぶのは「自分が書いたプロファイル」から（作業単位 F-4b-3 / ADR-0042 §決定5）
   *
   * 2026-09-07 の「モデルは見えるだけでよい。選ばせもしない」は、
   * **切り替えが安くなった（走ったまま差し替えられる）ので置き換わった。**
   * **選べるのは利用者が `upstreams.json` に書いたプロファイルだけ**で、
   * **モデルを探す・落とす操作は置かない**（そちらの「見えるだけ」は残る）。
   * 選ぶ・足す・直すは下の節（`UpstreamProfiles.svelte`）が持つ。
   *
   * **⚠ 上の節（観測）には、押せるものを置かない。** 観測は上流が名乗った値で、
   * 画面から書き換えるものではない。失敗したときの「再試行」だけは置く
   * ——**この画面をもう一度読み直すだけ**である。
   * **例外がもう1つ: 本文の面が `not_started` のときの「起こす」**（作業単位 F-10b / ADR-0055 §決定3）。
   * これも観測を書き換えない（選ばれているプロファイルと同じ名前の差し替えを頼むだけ）。
   * **状態のカードの隣で聞く**のは、起こすかどうかは状態（`not_started`）を見て決めることだから
   * （下の一覧も、作業単位 D-19 から「選んでいます」の印を出すが、起こしたかどうかは言わない）。
   * **例外の3つ目: 「いま確かめる」**（作業単位 B-66 / Issue origin#94）。サーバが5分ごとにしている上流への照会を、
   * いま1回頼むだけで、観測を画面から書き換えない（`POST /admin/upstream/check`）。
   *
   * # 書いた値と観測した値を混ぜない（Issue origin#24 の閉じる条件）
   *
   * **上の節は `GET /admin/upstream`（上流に聞いた値・起こした状態）、
   * 下の節は `GET /admin/upstream/profiles`（ファイルに書いたままの値）。**
   * 節を分け、**同じ表に並べない。** 書いた gguf のパスは下にだけ、
   * 上流が名乗った名前（ファイル名だけ）は上にだけ出る。
   *
   * # 起こした状態は、変わっていくのを読み直して見せる
   *
   * **どちらかの面が「載せている」間だけ、この画面は観測を読み直し続ける**
   * （`pollIntervalMs` ごと）。載ったら・落ちたら止める。画面を離れたら止める。
   * **読み直しに上限の回数は付けない** —— 20 GB の gguf は1〜2分かかり、
   * 載らないまま黙る子もサーバは殺さない（ADR-0035）。**読み直すだけ**なので、
   * 続けても何も壊さない。
   *
   * **古い読み直しで表を巻き戻さない。** 切り替えの応答の後に着いた、
   * それより前に出した読み直しは捨てる（`ticket`）。
   *
   * # ⚠ **申告された窓を、採用した予算と読ませない**
   *
   * `queried_context_tokens` は**上流が申告した値**であって、
   * **personacore がいま使っている予算ではない。**
   * **2つが食い違う状態は正常にありうる**（引き直しが失敗した回は、
   * 予算は現在値を保ち、申告値は「答えなし」になる）。**その食い違いこそが読みたいもの。**
   * だから**画面は写しを持たず、注記でそう言う。**
   *
   * # 帰属は `upstream.ts` が1箇所で決める
   *
   * どの行の、どちらの面の、どの欄か ——**入れ替えの変異を1本で押さえる**ため、
   * 表の組み立ては純粋関数に出してある（`reportRows`）。
   * **見張り番は `tests/upstream-panel.test.ts`** で、
   * **`data-*` ではなく `<td>` のセルを配列で突き合わせる。**
   *
   * # 面ごとのカード・状態の印・異常の帯・照らし合わせ（作業単位 D-12b-2 / Issue origin#93）
   *
   * 着手前の節は、上流に聞いた値を2つの表に同じ重さで並べていた —— 「突き合わせ: 見た行」のような
   * 診断の行も、起きているか・繋がっているかも。**いま何が起きているかが目に入らなかった**（origin#3）。
   *
   * * **面ごとのカード**（本文生成・裏方）に、状態の印・上流が名乗った名前・起こし方・宛先・
   *   起こした上流の状態（着手前の表の1行）・コンテキスト長を出す
   * * **異常は帯で知らせる**（失敗・止めた・鍵を断られた・鍵を送っていない・繋がらない の5つだけ。最後の1つは作業単位 B-66 で
   *   足した —— 外の上流に照会したが答えない面。まだ照会していない面・起こした面の名乗りが無いことはカードの印だけが言う）
   * * **鍵を暗号化せずに送っている面は、カードの「鍵の送り方」の行の印で言う**（作業単位 A-13b-3。異常ではない）
   * * **突き合わせの表と注記は「照らし合わせ」（`<details>`）に畳む。** 常に閉じて出し、見出しが一致しない行の数を言う
   *
   * **どの値から何を決めるかは `upstreamStatus.ts` が1箇所で持つ**（表と理由はそちらの doc）。
   * 見張り番は `tests/upstream-status.test.ts`（組み合わせの表）と `tests/upstream-panel-cards.test.ts`（画面）。
   *
   * **⚠ 照らし合わせに `open` を渡さない。開閉は利用者の操作だけで変わる**（司令塔の判断 / D-12b-2 の2周目）。
   * `llama-server` を上流にすると一致しない行が常にある（G-5）ので、ずれで開くと常に開いて出る。
   * **`<details>` を読み直しごとに作り直す形（`{#key report}` で囲む等）にもしないこと** —— 利用者が開いたものが閉じる。
   * 見張り番は `tests/upstream-panel-cards.test.ts` の「利用者が開けば開いたまま、閉じれば閉じたまま」。
   *
   * **ユーティリティと、土台の `.card` で組む**（ADR-0040 §決定4 —— 色・字・間隔は `<style>` に書かない）。
   * **`<style>` は持たない**（作業単位 D-12b-2 で外した。着手前の4つの規則はユーティリティへ移した）。
   */

  import { ApiRequestError, checkUpstream, getUpstream, switchUpstreamProfile, type UpstreamReport } from "./api";
  import { labelFor } from "./router";
  import { reportRows } from "./upstream";
  import {
    FACES,
    START_CAUTION,
    faceLabel,
    hasLoadingFace,
    launchLabel,
    profileToStart,
    startButtonLabel,
    startFailureLabel,
    startingLabel,
  } from "./upstreamProfiles";
  import {
    cardContext,
    cardName,
    checkLabel,
    comparisonSummary,
    faceStatus,
    keyMark,
    launchKindLabel,
    launchKindOf,
    upstreamBands,
    type FaceTone,
  } from "./upstreamStatus";
  import { browserZone } from "./timestamp";
  import UpstreamProfiles from "./UpstreamProfiles.svelte";

  let {
    pollIntervalMs = 2000,
  }: {
    /**
     * **載せている最中に、観測を読み直す間隔**（ミリ秒）。
     *
     * **測定由来の値ではない**（速く読み直しても、載るのは速くならない）。
     * 外しても「表の移り変わりが遅れて見える」だけで、何も壊れない。
     * テストは短くして回す。
     */
    pollIntervalMs?: number;
  } = $props();

  let report = $state<UpstreamReport | null>(null);

  /**
   * **状態の印の見た目**（作業単位 D-12b-2 / 絵の Main）。色の種類は `upstreamStatus.ts` の `FaceTone` が決める。
   *
   * | 種類 | 印 | 点 |
   * |---|---|---|
   * | 動いている | 読む側の地に塗る | 塗った点 |
   * | 載せている | 読む側の枠だけ | 枠だけの点 |
   * | 異常 | 壊れている側（暖色）の地に塗る | 塗った点 |
   * | 起こしていない・まだ確かめていない・名乗りが無い・知らない綴り | 罫の枠、色なし | 枠だけの点 |
   *
   * **まだ確かめていない・名乗りが無い（`quiet`）は警告の色にしない**（`upstreamStatus.ts` の冒頭の ⚠）。
   */
  const BADGE_LOOK: Readonly<Record<FaceTone, { readonly badge: string; readonly dot: string }>> = {
    running: { badge: "bg-read-bg text-read", dot: "bg-read" },
    loading: { badge: "border border-read text-read", dot: "border-2 border-read" },
    alarm: { badge: "bg-warn-bg text-warn", dot: "bg-warn" },
    waiting: { badge: "border border-rule-2 text-ink-2", dot: "border-2 border-muted" },
    quiet: { badge: "border border-rule-2 text-ink-2", dot: "border-2 border-muted" },
    unknown: { badge: "border border-rule-2 text-ink-2", dot: "border-2 border-muted" },
  };
  /** **表を読んだ時刻**（状態の文面の経過の「いま」/ Issue origin#40）。 */
  let readAt = $state<Date>(new Date());
  const readerZone = browserZone();
  let error = $state<string | null>(null);

  /**
   * **いちばん新しい読みの番号。** 着いた応答の番号がこれより古ければ捨てる。
   *
   * **`$state` にしない** —— 描画には使わない。
   */
  let ticket = 0;

  /**
   * **この画面が生きている間**の中断ハンドル。破棄されたら落とす。
   *
   * 落とさないと、画面を離れた後も応答を待ち続け、
   * **もう無い画面の状態を書き換える。**
   */
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

  async function load(): Promise<void> {
    ticket += 1;
    const mine = ticket;
    error = null;
    try {
      const next = await getUpstream(lifetime.signal);
      if (lifetime.signal.aborted || mine !== ticket) {
        return;
      }
      readAt = new Date();
      report = next;
    } catch (thrown) {
      // **自分で落とした分は失敗として書かない**（画面はもう無い）。
      if (lifetime.signal.aborted || mine !== ticket) {
        return;
      }
      report = null;
      error = messageOf(thrown);
    }
  }

  /**
   * **切り替えの応答を、いまの観測として採る**（`UpstreamProfiles.svelte` から）。
   *
   * **番号を進める** —— それより前に出した読み直しが後から着いても、表を巻き戻さない。
   */
  function adopt(next: UpstreamReport): void {
    ticket += 1;
    error = null;
    readAt = new Date();
    report = next;
  }

  /** **起こす依頼の応答を待っているプロファイルの名前**（作業単位 F-10b）。`null` は待っていない。 */
  let starting = $state<string | null>(null);
  /** 起こす依頼が断られた理由（409 等。次に押したら消す）。 */
  let startError = $state<string | null>(null);

  /**
   * **起こしていない本文の面を起こす**（作業単位 F-10b / ADR-0055 §決定3）。
   *
   * **いま選ばれているプロファイルと同じ名前で `POST /admin/upstream/profile` を送る**（名前は
   * `profileToStart` が `selected_profile` から決める。一覧からは選ばない）。経路は下の節の
   * 「このプロファイルに切り替える」と同じ差し替えで、**応答を [`adopt`] で採る** ——
   * 起こした面は `loading` で返り、載るまで読み直し続ける（既存の差し替えの表示）。
   * **断られたら（409 等）サーバの文面をそのまま言う**（下の節の「切り替えられませんでした: …」と同じ形）。
   */
  async function start(name: string): Promise<void> {
    starting = name;
    startError = null;
    try {
      const next = await switchUpstreamProfile(name, lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      adopt(next);
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      startError = startFailureLabel(messageOf(thrown));
    } finally {
      starting = null;
    }
  }

  /** **「いま確かめる」の応答を待っているか**（作業単位 B-66）。待っている間はボタンを押せない。 */
  let checking = $state(false);
  /** 確かめる依頼が失敗した理由（次に押したら消す）。 */
  let checkError = $state<string | null>(null);

  /**
   * **両方の面の上流に、いま照会する**（作業単位 B-66 / Issue origin#94 の決定3・4 / `POST /admin/upstream/check`）。
   *
   * **応答（照会した後の観測）を [`adopt`] で採る** —— 起こすボタン・切り替えと同じ採り方（読み直しで巻き戻さない）。
   * **失敗したらサーバの文面をそのまま言い、観測は消さない**（いま見えている観測は、失敗した依頼より新しい）。
   * **照会の中身はサーバの5分ごとと同じ**で、観測を画面から書き換える操作ではない。
   */
  async function checkNow(): Promise<void> {
    checking = true;
    checkError = null;
    try {
      const next = await checkUpstream(lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      adopt(next);
    } catch (thrown) {
      if (lifetime.signal.aborted) {
        return;
      }
      checkError = `確かめられませんでした: ${messageOf(thrown)}`;
    } finally {
      checking = false;
    }
  }

  // **載せている間だけ、読み直す。** 表が変わるたびに次の1回を予約し直す。
  $effect(() => {
    const current = report;
    if (current === null || !hasLoadingFace(current)) {
      return;
    }
    const timer = setTimeout(() => {
      void load();
    }, pollIntervalMs);
    return () => {
      clearTimeout(timer);
    };
  });

  void load();
</script>

<section>
  <!--
    **見出しは `labelFor` から出す**（作業単位 D-9 のレビュー是正 / 2026-09-08）。
    **綴りを直書きすると、ナビ・会話の画面の案内・ここの3つが写しになる。**
    会話の画面は「**上流モデルの画面で見られます**」と名乗るので、
    **ここが別の名前を名乗ると、案内だけが黙って嘘になる。**
    `StatsPanel.svelte` と同じ形。
  -->
  <h1>{labelFor({ route: "upstream" })}</h1>
  <p class="note">
    personacore が、本文と裏方をそれぞれどこへ、どのモデルの綴りで依頼しているかを出します。
    upstreams.json に書いたプロファイルを選んで、走ったまま上流を切り替えられます。
  </p>

  <section class="now mt-6 flex flex-col gap-4" aria-labelledby="upstream-now-heading">
    <h2 id="upstream-now-heading" class="m-0">いまの上流</h2>
    {#if error !== null}
      <p class="error" role="alert">
        上流の様子を取得できませんでした: {error}
        <button type="button" onclick={() => void load()}>再試行</button>
      </p>
    {:else if report === null}
      <p class="muted" role="status">読み込み中…</p>
    {:else}
      <!--
        **この起動が上流プロファイルの見本を書いたときだけ、その場所と次の一手を言う**
        （作業単位 F-4c / Issue origin#23）。**書いていない起動（`null`）では何も言わない。**

        **画面で選べるようになった**（作業単位 F-4b-3 / Issue origin#24）ので、次の一手は画面の操作を案内する。
        **見本を書くのは既定の置き場所だけ**（`main.rs` の `ProfilesPath::Default`）で、
        **そこは画面から直せる置き場所でもある**（`with_editable_profiles_file`）。
        だから「下の一覧で直す」「切り替える」は、見本を書いた起動では必ず画面に在る操作である。
        **`--upstream <名前>` は今も在る手段なので、並べて言ってよい。**
        見張り番は `tests/upstream-panel.test.ts` の「上流プロファイルの見本」。
      -->
      {#if report.profile_sample !== null}
        <p class="sample-notice m-0">
          上流プロファイルの見本を <code>{report.profile_sample}</code> に置きました。下の一覧の「直す」で
          gguf のパスを自分の機のものに書き換えてから、「このプロファイルに切り替える」で選べます。起動するときに
          --upstream &lt;名前&gt; を付けても選べます。scripts/start.ps1 の -Upstream
          &lt;名前&gt; は、このファイルではなく scripts/upstreams.json（無ければ
          scripts/upstreams.sample.json）を読みます。
        </p>
        <!--
          **⚠ 2つ目の文は「start.ps1 なら -Upstream」と言い換えない**（レビュー1周目の提案3 / 2026-09-23）。
          **`start.ps1 -Upstream` は `PERSONACORE_UPSTREAMS` を `scripts/` の下へ向ける**ので、
          **ここで書き換えたファイルは読まれない。** 並べて言い換えると、利用者は
          書き換えたのに効かない形に入る。綴りの権威は `scripts/start.ps1` の `$UpstreamProfileFile`。
        -->
      {/if}
      <!--
        **面ごとのカード**（作業単位 D-12b-2 / 絵の Main）。**面は `FACES` の順（本文 → 裏方）に、`report[face]` で引く。**
        見出しの綴りは `faceLabel`（照らし合わせの表の列の見出し・下の一覧の欄の組と同じ）。

        **「起こした上流の状態」の行**は、着手前の表（見出し「この実行ファイルが上流をどうしているか」）の1行を
        カードへ移したもの（ADR-0042 §決定3 / 作業単位 F-4b-3）。**載せている → 準備完了 / 失敗** の移り変わり・
        経過・失敗の理由とログの名前は、ここが言う（載せている間は読み直し続ける）。印は短く、詳しくはこの行。
        **「この実行ファイルが何をしたか」と「上流が何と答えたか」は別の行・別の欄から読む**（`upstreamStatus.ts`）。

        **コンテキスト長の出どころは「上流が申告した値」だけ** —— 書いたプロファイルの長さは下の一覧が出す
        （書いた値と観測した値を混ぜない）。
      -->
      <!--
        **いま確かめる**（作業単位 B-66 / Issue origin#94 の決定3・4）。両方の面の上流に、いま照会する。
        **観測を書き換える操作ではない** —— サーバが5分ごとにしている照会を、いま1回頼むだけ（結果はカードと帯に出る）。
        応答を待つ間は押せない（サーバも2本目を並べない）。失敗はこの節の作法（`role="alert"`）で言う。
      -->
      <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
        <button type="button" disabled={checking} onclick={() => void checkNow()}>いま確かめる</button>
        {#if checking}
          <p class="muted m-0" role="status">確かめています…</p>
        {/if}
        {#if checkError !== null}
          <p class="error m-0" role="alert">{checkError}</p>
        {/if}
      </div>
      <div class="flex flex-wrap gap-4">
        {#each FACES as face (face)}
          {@const view = report[face]}
          {@const status = faceStatus(view)}
          {@const name = cardName(view)}
          {@const context = cardContext(view)}
          <article
            class="card flex min-w-0 flex-1 basis-80 flex-col gap-3.5"
            aria-labelledby={`upstream-card-${face}`}
          >
            <div class="flex flex-wrap items-center justify-between gap-2">
              <h3 id={`upstream-card-${face}`} class="m-0 text-base font-semibold text-ink-2">{faceLabel(face)}</h3>
              <span
                class={[
                  "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-sm font-semibold wrap-anywhere",
                  BADGE_LOOK[status.tone].badge,
                ]}
                ><span aria-hidden="true" class={["size-2 shrink-0 rounded-full", BADGE_LOOK[status.tone].dot]}
                ></span>{status.badge}</span
              >
            </div>
            <!-- **名乗った名前は等幅、名乗らなかった理由は補足の字**（綴りに見えないように / `upstream.ts`）。 -->
            <p class={["m-0 break-all", name.named ? "font-mono text-xl" : "text-muted"]}>{name.text}</p>
            <dl class="m-0 grid grid-cols-[max-content_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-sm">
              <div class="contents">
                <dt class="text-muted">起こし方</dt>
                <dd class="m-0">{launchKindLabel(launchKindOf(view.launch))}</dd>
              </div>
              <div class="contents">
                <dt class="text-muted">宛先</dt>
                <dd class="m-0 font-mono break-all">{view.endpoint}</dd>
              </div>
              <div class="contents">
                <dt class="text-muted">起こした上流の状態</dt>
                <dd class="m-0 wrap-anywhere">{launchLabel(view.launch, readAt, readerZone)}</dd>
              </div>
              <!--
                **いつ・どう確かめたか**（作業単位 B-66）。**時刻は読み手の時間帯の `HH:MM:SS`**（レビュー是正 —— 経過は
                画面を読み直さない間に古びるので出さない）。
              -->
              <div class="contents">
                <dt class="text-muted">最後の確認</dt>
                <dd class="m-0 wrap-anywhere">{checkLabel(view, readerZone)}</dd>
              </div>
              <div class="contents">
                <dt class="text-muted">コンテキスト長</dt>
                <dd class="m-0">
                  {#if context.source === null}{context.value}{:else}<span class="font-mono">{context.value}</span
                    ><span class="text-muted">（{context.source}）</span>{/if}
                </dd>
              </div>
              <!--
                **鍵を暗号化せずに送っている面だけ、送り方の印を出す**（作業単位 A-13b-3 / `upstreamStatus.ts` の `keyMark`）。
                **異常ではない**ので帯にせず、暖色の枠だけの印にする（送っていない `not_approved` は印と帯が言う）。
              -->
              {#if keyMark(view) !== null}
                <div class="contents">
                  <dt class="text-muted">鍵の送り方</dt>
                  <dd class="m-0">
                    <span class="inline-flex rounded-full border border-warn px-2 text-xs wrap-anywhere text-warn"
                      >{keyMark(view)}</span
                    >
                  </dd>
                </div>
              {/if}
            </dl>
          </article>
        {/each}
      </div>
      {#if hasLoadingFace(report)}
        <p class="note m-0" role="status">
          載せている間は、この節を読み直し続けます。大きな gguf は載るまで数分かかることがあります。
        </p>
      {/if}
      <!--
        **異常の帯**（作業単位 D-12b-2）。**どの状態を異常とするかは `upstreamStatus.ts` の `faceStatus`**
        （失敗・止めた・鍵を断られた・鍵を送っていない・繋がらない の5つだけ。鍵を送っていないは作業単位 A-13b-3、
        繋がらないは B-66 で足した）。帯は面を名乗る（本文 → 裏方の順）。
        **まだ照会していない面は帯にしない**（D-12b-2 の2周目は「名乗りが無いこと」を帯にしなかった —— 照会していない面と
        区別できなかったので。B-66 でサーバが `check_result` を言うようになり、照会したが答えない外の上流だけを帯に上げた）。

        **容れ物は、異常が無くても置いておく**（中身が変わる前から在る告げる領域 / `tests/live-region.test.ts` の冒頭）。
        載せている → 失敗 に読み直しで移ったとき、現れた帯が告げられる。

        **⚠ 空の容れ物が、節の間隔（`gap-4`）を1つ余分に取らないようにしてある** —— 容れ物は `-mt-4` で手前の間隔を打ち消し、
        帯の1本目が `first:mt-4` で取り戻す（2本目からは `mt-2`）。空なら間隔は1つ、帯があれば帯の前後に1つずつ。
        **見張り番は無い**（単体テストは配置を計算しない。画面で見るしかない）。

        **⚠ 帯に押せるものを置かない**（この節に置く操作は「起こす」と「再試行」だけ）。鍵の帯は、直す場所
        （下の節のプロファイルの「直す」）を文で名指す（作業単位 A-13b-3 で文面を差し替えた / `upstreamStatus.ts`）。
        絵の「プロファイルを直す」ボタンは置かない —— どのプロファイルが動いているかを、観測は言わないことがある
        （環境変数で起こした起動）。
      -->
      <div class="-mt-4 flex flex-col" role="status" aria-label="上流の異常">
        <!-- **理由も宛先も同じ2つの面の帯は1本にまとまる**（`upstreamBands`。見出しは両方の面を言う）。 -->
        {#each upstreamBands(report, readerZone) as band (band.label)}
          <div
            class="mt-2 flex flex-wrap items-baseline gap-x-3 gap-y-1 rounded-lg border border-warn bg-warn-bg px-4.5 py-3.5 text-sm first:mt-4"
          >
            <strong class="text-warn">{band.label}</strong>
            <span>{band.message}</span>
            {#if band.endpoint !== null}
              <span class="min-w-0 break-all">宛先: <code>{band.endpoint}</code></span>
            {/if}
          </div>
        {/each}
      </div>
      <!--
        **本文の面を起こしていないなら、起こすかを聞く**（作業単位 F-10b / ADR-0055 §決定3）。
        **この節に置く唯一の操作**（失敗したときの「再試行」を除く）。観測を書き換えるものではなく、
        いま選ばれているプロファイルと同じ名前の差し替えを頼むだけ。**状態のカードの隣で聞く** ——
        選ばれている1件の枠では、下の一覧は「切り替える」の代わりに「切り替え直す」を出す（作業単位 D-19）。
        押せば同じ差し替えになるが、起こしていないことを言って起こすかを聞くのは、ここだけ。
        見張り番は `tests/upstream-panel-start.test.ts` と、`tests/upstream-panel.test.ts` の
        「本文の面を起こしていないときに、書いた節の外で押せるのは「起こす」だけ」。
      -->
      {#if profileToStart(report) !== null}
        {@const name = profileToStart(report) ?? ""}
        <div class="start-upstream flex flex-col items-start gap-2 wrap-anywhere">
          <p class="note m-0">本文のモデルは、まだ起こしていません。{START_CAUTION}</p>
          <button type="button" disabled={starting !== null} onclick={() => void start(name)}
            >{startButtonLabel(name)}</button
          >
          {#if starting !== null}
            <p class="muted m-0" role="status">{startingLabel(starting)}</p>
          {/if}
          {#if startError !== null}
            <p class="error m-0" role="alert">{startError}</p>
          {/if}
        </div>
      {/if}

      <!--
        **カードのコンテキスト長が何であるかを、カードのすぐ下で言う**（照らし合わせに畳まない）。
        カードは「上流が申告した値」を出しているので、それを予算と読ませない注記は畳むと届かない。
      -->
      <p class="note m-0">
        コンテキスト長は上流が申告した値です。personacore がいま使っている予算とは別で、
        食い違うことがあります。
      </p>

      <!--
        **照らし合わせ**（作業単位 D-12b-2 / 絵の Main）。着手前の「上流に聞いた値」の表と、その注記を畳む。
        **常に閉じて出す（`open` を渡さない）。** 見出しが一致しない行の数を面ごとに言い、読み直しで更新する
        （`upstreamStatus.ts` の `comparisonSummary`）。開閉は利用者の操作だけで変わる（冒頭の ⚠）。

        **表の中身は着手前のまま**（`reportRows`）—— 頼んだ名前と答えた名前の食い違いの一言・答え方の4つの文面・
        突き合わせの5つの内訳は、ここにだけ出る。

        **⚠ `<summary>` は開け閉めするだけで、観測を書き換えない**（この節に置く操作の例外に数えない）。
      -->
      <details class="border-t border-rule pt-3 text-sm">
        <summary class="cursor-pointer text-ink-2">{comparisonSummary(report)}</summary>
        <!--
          **行のセルを、そのまま読ませる。**
          `data-*` はテストが置いた印であって、読み手が何を見るかの権威ではない。
        -->
        <table class="observed my-4">
          <caption class="text-left font-bold">上流に聞いた値</caption>
          <thead>
            <tr>
              <th scope="col">項目</th>
              <th scope="col">本文生成</th>
              <th scope="col">裏方（抽出・変容）</th>
            </tr>
          </thead>
          <tbody>
            {#each reportRows(report) as row (row.label)}
              <tr>
                <!-- **左端は項目名で、値ではない。** 見出しほど強く出さない。 -->
                <th scope="row" class="font-normal text-ink-2">{row.label}</th>
                <td>{row.main}</td>
                <td>{row.sub}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <!--
          **⚠ 頼んだ名前と答えた名前は別物である**（作業単位 B-45 / `C-18` の実測）。
          **上流によっては、こちらが指定した綴りを無視して、
          いま載っているモデルが答える**（`llama serve` がそれ）。
          **食い違いは異常ではない**ので、警告の見た目にしない（帯にも出さない）。
          食い違っている面の一言は、その面のセルの中に出る（`upstream.ts`）。
        -->
        <p class="note my-1">
          依頼するモデルの綴りは、personacore が上流に指定した名前です。上流によっては
          その指定を見ずに、いま載っているモデルが答えます。答えたモデルの綴りは、
          上流が自分で名乗った名前で、名乗らない上流もあります。
        </p>
        <!--
          **⚠ 切り落としたことを、画面が自分で言う**（レビュー是正 / 2026-09-17）。
          **`llama-server` が名乗る `id` は gguf の絶対パス**で、
          **サーバは継ぎ目でファイル名だけにしている**（絶対制約: 内部パスを外へ出さない）。
          **黙っていると、すぐ上の「上流が自分で名乗った名前」だけが嘘になる**
          ——読み手は「上流はこの短い綴りで名乗った」と読む。
        -->
        <p class="note my-1">
          上流がモデルの置き場所ごと名乗ったときは、ファイル名だけを出します。
          置き場所そのものは画面に出しません。
        </p>
        <!--
          **⚠ 名乗らなかった2つの理由を、同じ文面にしない**（レビュー是正 / 同日）。
          **上流が黙っている**のと**一覧は読めたが名指しできない**のは、
          **調べる先がまるで違う**（前者は上流が生きているか、
          後者は依頼する綴りが一覧の綴りと合っているか）。
          文面を決めるのは `upstream.ts`。ここは、その違いが在ることを言う。
        -->
        <p class="note my-1">
          上流が一覧を出していても、そこに依頼した綴りが無く、候補が2つ以上あるときは、
          どれが答えたかを決められません。そのときと、上流が黙っているときとでは、
          答えたモデルの綴りの欄に別の文面が出ます。
        </p>
        <!--
          **⚠ 起動時に測っているものは1つも無い**（作業単位 D-9 のレビュー是正 / 2026-09-08）。
          着手前ここには「**起動時に測った相手といまの上流を照らした結果**」と書いてあったが、
          **事実ではない。** 突き合わせの相手は `src/measured.rs` の定数が名乗る
          `measured_on` で、**過去にこの機で測ってソースへ焼き込んだ綴り**である
          （ADR-0033）。**照合も起動時だけではない**——`server::upstream` は
          **5分ごとの引き直し**（ADR-0020）でも更新される。

          **⚠ 誤読の実害**——`見た行 10 / ずれ 3` を見た利用者が
          「起動し直せば測り直されて 10/0 になる」と読み、
          **再起動して数が変わらないことに困る。** 要るのは定数の測り直しのほう。

          **この画面は「事実を読むため」に在るので、事実でないことを言うのは他の画面より重い。**
          見張り番は `tests/upstream-panel.test.ts` の「注記が、事実を言っている」。
        -->
        <p class="note my-1">
          突き合わせは、ソースに焼き込んである実測由来の定数が名乗る「測ったときの相手」と、
          いま上流が載せているモデルの一覧を照らした結果です。1行が定数1つで、
          見た行は、その下の4つの和です。
        </p>
        <p class="note my-1">
          定数は過去に測って書いた値で、personacore が測り直すことはありません。
          照らす作業だけを、上流を引き直すたびにやり直しています。
          だから、ずれが出ていても再起動では消えません。消すには定数を測り直します。
        </p>
        <!--
          **「上流が答えませんでした」は、聞いたのに答えなかった面だけに出る**（作業単位 B-66 / Issue origin#94）。
          D-12b-2 の2周目から B-66 まで、ここは「personacore がその面に聞いていません（コンテキスト長を設定した面には
          聞きません）」とも言っていた —— 当時は設定した面に照会しなかったので。**いまは設定した面にも照会し**、
          まだ聞いていない面は別の文面（「まだ確かめていません」）で出る（`upstream.ts` の `NOT_ASKED_LABEL`）。
          **届いたが一覧が読めない面（一覧の口を持たない別の実装など）も別の文面**で出る（レビュー是正 / `NO_MODEL_LIST_LABEL`）。
          見張り番は `tests/upstream-panel-cards.test.ts` の「答えなかった面の注記は、聞いたのに答えなかった面のことで…」。
        -->
        <p class="note my-1">
          「上流が答えませんでした」と出ている面は、personacore が聞いたのに上流が黙っていた（止まっている、
          宛先が違う、など）面です。まだ確かめていない面には「まだ確かめていません」と出ます。上流が黙っていても
          personacore は直前の予算のまま動き続けるので、この欄が手掛かりになります。
        </p>
      </details>
    {/if}
  </section>

  <!--
    **書いたプロファイル**（作業単位 F-4b-3）。**観測の節の外に置く**（書いた値と観測した値を混ぜない）。
    **観測が読めなくても出す** —— ファイルを直す理由は、上流が黙っていることが多い。

    **`selected` は「どれを選んでいるか」の印にだけ使う**（作業単位 D-19）。上流に聞いた値ではなく、
    この実行ファイルの切り替えの状態（`selected_profile`）。**観測が読めない間は `null`**（印を付けない）。
    切り替えの応答を [`adopt`] で採ると、印もその名前へ移る。
  -->
  <UpstreamProfiles onswitched={adopt} selected={report?.selected_profile ?? null} />
</section>
