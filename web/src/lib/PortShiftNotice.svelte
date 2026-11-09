<script lang="ts">
  /**
   * **待ち受けのポートをずらして起きたことを言う帯**（作業単位 F-11c / ADR-0057 §決定4「画面」）。
   *
   * **`App.svelte` が全部の画面の上の帯（`header` の中・`main` の外）に置く。** 文は `portShift.ts` が組み、
   * ここは並べて「設定」の画面へのリンクを添えるだけ。
   *
   * # 読むのは開いたときに1回だけ
   *
   * **待ち受けの番号は起動の間に変わらない**ので、周期で読み直さない。
   *
   * # 読めなかったら、何も出さない
   *
   * **`GET /admin/server` が失敗しても（繋がらない・形が違う）帯を出さず、他の画面を壊さない。**
   * 帯は「直すきっかけ」を渡すためのもので、出せなくても画面は使える。失敗を言う段落も出さない
   * （どの画面にも毎回出る帯で、別の画面の失敗と紛れる）。
   *
   * **ユーティリティのクラスで組む**（色・字・間隔は `<style>` に書かない）。**`<style>` は持たない。**
   */

  import { getServer } from "./api";
  import { portShiftNotice, type PortShiftNotice } from "./portShift";
  import { hrefFor, labelFor } from "./router";

  let notice = $state<PortShiftNotice | null>(null);

  /** **この帯が生きている間**の中断ハンドル。破棄されたら落とす。 */
  const lifetime = new AbortController();

  $effect(() => {
    return () => {
      lifetime.abort();
    };
  });

  async function load(): Promise<void> {
    try {
      const view = await getServer(lifetime.signal);
      notice = portShiftNotice(view);
    } catch {
      // **出さない**（上の doc）。形の違う応答で文を組めなかったときも同じ。
      notice = null;
    }
  }

  void load();

  const settings = { route: "settings" } as const;
</script>

{#if notice !== null}
  <div class="border-t border-rule bg-warn-bg">
    <p class="mx-auto max-w-240 px-6 py-2 text-sm text-ink">
      {notice.lead}<a href={hrefFor(settings)}>{labelFor(settings)}</a>{notice.tail}
    </p>
  </div>
{/if}
