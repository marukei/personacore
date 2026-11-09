<script lang="ts">
  import AgreementsPanel from "./lib/AgreementsPanel.svelte";
  import AttributesPanel from "./lib/AttributesPanel.svelte";
  import CharacterList from "./lib/CharacterList.svelte";
  import ChatView from "./lib/ChatView.svelte";
  import MemoryList from "./lib/MemoryList.svelte";
  import PortShiftNotice from "./lib/PortShiftNotice.svelte";
  import SettingsPanel from "./lib/SettingsPanel.svelte";
  import StatsPanel from "./lib/StatsPanel.svelte";
  import UpstreamPanel from "./lib/UpstreamPanel.svelte";
  import {
    currentScreen,
    hrefFor,
    labelFor,
    navScreens,
    startRouting,
    type Screen,
  } from "./lib/router";

  let screen = $state<Screen>(currentScreen());

  // **ハッシュだけを聞く**（`src/lib/router.ts`）。ルータの依存は足していない。
  $effect(() =>
    startRouting((next) => {
      screen = next;
    }),
  );
</script>

<!--
  **骨格に名前を付ける**（作業単位 D-11b）。**属性と、囲みが1つ増えただけ。**

  **⚠ 帯を `main` の中に置かない。** 画面本体の目印は「いま見ている画面の中身」で、
  **どの画面へも行ける帯は、その外に在る。**

  見張り番は `tests/live-region.test.ts` の「画面の骨格が、名前を持っている」。

  **帯の見た目**（作業単位 D-12b-1 / 利用者が承認した絵の Main）——
  **地を一段濃く（`paper-2`）、左に製品名（等幅・薄く）、右へ画面のタブ。**
  **帯は画面の端から端まで地を敷き、中身だけを本体と同じ幅に揃える**
  （着手前の `.page` は帯ごと幅を絞っていたので、地を敷けなかった）。

  **⚠ いまいる画面の印（太字・下の read の線）は `aria-[current=page]:` の変位で書く。**
  **読み上げに告げる属性と目に見える印が、同じ1つの条件から出る** ——
  いまいる画面だけに別のクラスを足す形にすると、片方だけが古びる。
  **全部のタブの `class` は一字一句同じ**で、見張り番（`tests/app-nav.test.ts`）がそれを見ている。

  **⚠ タブの押せる高さは 48px**（`min-h-12` / 絵のまま。下限 44px は同じ見張り番）。
  **⚠ リンクの既定の色**（土台の `a` の read）**を `text-ink-2` で上書きしている。**
  **これが効くのは、土台を層に入れたから**（`src/app.css` の `@layer` の注記 / ADR-0040 の追記）。

  **⚠ 帯の右端の状態の表示（本文・裏方）は、この回は作らない**（Issue origin#88 の「触らないもの」）。
-->
<!--
  **`block` は土台の `header`（画面ごとの見出しの帯 / 横並び）を打ち消すため。**
  帯の `header` は中身を1つしか持たず、横並びにすると中身が幅いっぱいに広がらない。
-->
<header class="block border-b border-rule bg-paper-2">
  <div class="mx-auto flex max-w-240 flex-wrap items-stretch gap-x-6 gap-y-2 px-6">
    <span class="self-center py-3.5 font-mono text-sm text-muted">personacore</span>
    <!-- **並べ方（横並び・狭い幅で折り返す）は土台の `nav`**（`src/app.css`）。ここは伸びるだけ。 -->
    <nav aria-label="画面の切り替え" class="grow">
      {#each navScreens() as item (item.route)}
        <a
          class="flex min-h-12 items-center border-b-2 border-transparent px-3.5 text-ink-2 no-underline aria-[current=page]:border-read aria-[current=page]:font-bold aria-[current=page]:text-ink"
          href={hrefFor(item)}
          aria-current={screen.route === item.route ? "page" : undefined}
        >
          {labelFor(item)}
        </a>
      {/each}
    </nav>
  </div>
  <!--
    **待ち受けのポートをずらして起きたら、ナビの下に1行の帯を出す**（作業単位 F-11c / ADR-0057 §決定4）。
    **`main` の外**——どの画面を開いても見える。ずらしていない・読めないときは何も出さない
    （`PortShiftNotice.svelte`）。見張り番は `tests/port-shift-notice.test.ts`（この配線も見る）。
  -->
  <PortShiftNotice />
</header>

<!--
  **本体の幅と余白**（着手前の `.page` の 60rem と 1.5rem をユーティリティへ移した）。
  **`max-w-240` は 240 × 0.25rem = 60rem**（v4 の間隔の目盛り）。
-->
<main class="mx-auto max-w-240 p-6">
    {#if screen.route === "chat"}
      <ChatView />
    {:else if screen.route === "characters"}
      <CharacterList />
    {:else if screen.route === "upstream"}
      <!--
        **上流について観測できたこと**（作業単位 D-9）。

        **`{#key}` は要らない** —— この画面は識別子を運ばない（`router.ts` の `Screen`）ので、
        **作り直す理由になる値が1つも無い。**
        記憶や性格値の側に `{#key}` が要るのは、**別のキャラクターへ移ったときに
        見出しだけが新しい相手を名乗る**からで、ここではその状態を作れない。
      -->
      <UpstreamPanel />
    {:else if screen.route === "settings"}
      <!--
        **personacore 自身の設定**（作業単位 F-12b。いまは待ち受けのポート）。
        **`{#key}` は要らない** —— 上流の画面と同じく、識別子を運ばない。
      -->
      <SettingsPanel />
    {:else if screen.route === "memories"}
      <!--
        **識別子は経路が運ぶ**（`router.ts` の `Screen`）。
        **`{#key}` で作り直す** —— 別のキャラクターの記憶へ移ったときに
        前の一覧と、開きかけの訂正の下書きを持ち越さないため。
      -->
      {#key screen.handle}
        <MemoryList handle={screen.handle} />
      {/key}
    {:else if screen.route === "stats"}
      <!--
        **性格値と変容**（作業単位 D-4）。

        **⚠ `{:else}` にしないこと**（レビューの指摘 / 2026-08-28）。
        受け皿にすると、**識別子を運ぶ画面をもう1つ足した日に、
        その画面が黙って `StatsPanel` として描かれる。**
        綴りを明示しておけば、その日は**何も描かれない**（＝すぐ気付く）。

        **`{#key}` で作り直す** ——
        作り直さないと、**見出しだけが新しいキャラクターを名乗り、
        表には前のキャラクターの値が残る**（`StatsPanel` は
        作り直されなければ読み直さない）。
        **その状態で「置き換える」を押すと、B の画面から A の性格値へ `PUT` が飛び、
        しかも `PUT` は丸ごと置き換えなので A の軸が B の軸で上書きされる。**
      -->
      {#key screen.handle}
        <StatsPanel handle={screen.handle} />
      {/key}
    {:else if screen.route === "attributes"}
      <!--
        **属性と変更の記録**（作業単位 D-12a）。

        **⚠ `{:else}` にしないこと**（`stats` の側と同じ理由）。
        受け皿にすると、**識別子を運ぶ画面をもう1つ足した日に、
        その画面が黙って `AttributesPanel` として描かれる。**

        **`{#key}` で作り直す** ——
        作り直さないと、**見出しだけが新しいキャラクターを名乗り、
        表には前のキャラクターの属性が残る**（`AttributesPanel` は
        作り直されなければ読み直さない）。
        **その状態で「置き換える」を押すと、B の画面から A の属性へ `PUT` が飛び、
        しかも `PUT` は丸ごと置き換えなので A の属性が B の属性で上書きされる。**
        **そして属性は削れない枠に毎ターン載る**ので、
        **A の以後の全ターンが B の事実で喋る。**
      -->
      {#key screen.handle}
        <AttributesPanel handle={screen.handle} />
      {/key}
    {:else if screen.route === "agreements"}
      <!--
        **約束・決まり**（作業単位 A-12a / ADR-0051）。

        **⚠ `{:else}` にしないこと**（`stats` の側と同じ理由）。

        **`{#key}` で作り直す** —— 作り直さないと、見出しだけが新しいキャラクターを名乗り、
        一覧には前のキャラクターの取り決めが残る。**その状態で「終わらせる」を押すと、
        B の画面から A の取り決めの id へ送られる**（id はキャラクターをまたいで別物なので
        404 で断られるが、**書く欄は B の画面から A へ書く**）。
      -->
      {#key screen.handle}
        <AgreementsPanel handle={screen.handle} />
      {/key}
    {/if}
</main>

<!--
  **`<style>` は持たない**（作業単位 D-12b-1）。帯と本体の幅・余白・色はユーティリティで書いた
  （ADR-0040 §決定4 —— 色・字・間隔は `<style>` に書かない）。
  **色と字の土台は `src/app.css` にある**（作業単位 D-10）。**`:global(body)` もそちらへ移してある** ——
  部品の中から全体を触ると、その部品を読まない画面（テストが直接 `mount` する形）で効かなくなる。

  **⚠ 着手前は「いまいる画面は色を持たない（色は行き先の印）」としていた。**
  **D-12b-1 で、利用者が承認した絵に合わせ、いまいる画面の下に read の線を引くよう改めた**
  （字の色は墨のまま。read は線だけに使う）。
-->
