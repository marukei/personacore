/**
 * **ハッシュによるルーティング**
 * （`#/chat` / `#/characters` / `#/upstream` / `#/settings` / `#/memories/{識別子}` /
 * `#/stats/{識別子}` / `#/attributes/{識別子}` / `#/agreements/{識別子}`）。
 *
 * # なぜハッシュか。なぜライブラリを足さないか
 *
 * **axum は SPA フォールバックを持っていない**（`index.html` を常に返さない）。
 * 入れない理由は `src/server/web.rs` のモジュール doc にある——**ここに写さない。**
 *
 * その帰結として **`ServeDir` は `/chat` のような深いパスを配れない**
 * （そのファイルが無いので 404 になる）。
 * **ハッシュはサーバへ送られない**ので、この制約と衝突しない。
 *
 * 必要なのは `hashchange` を聞くことだけで、**依存を1つも足していない。**
 *
 * # 知らないハッシュは、黙って既定へ落とさない
 *
 * 綴りを間違えた `#/chatt` で会話の画面が出ると、**アドレス欄と画面が食い違う。**
 * ここでは **URL の側を既定へ書き換える**（`location.replace` なので履歴を汚さない）ので、
 * 「いま見ている画面の URL」が常に本当のことを言う。
 *
 * # 画面は「名前」ではなく `Screen`（作業単位 D-3 で変えた）
 *
 * **記憶の画面はキャラクターごと**（`GET /admin/characters/{handle}/memories`）なので、
 * 名前だけでは行き先が決まらない。**識別子を経路が運ぶ。**
 * 名前と識別子を別々に持たせると（`route` ＋ どこかの `handle`）、
 * **画面はキャラクター A を名乗ったまま、中身が B のもの**という形で食い違いうる
 * ——**帰属の壊れ方**（CLAUDE.md）なので、1つの値にまとめてある。
 */

/** 画面。**識別子が要る画面は、それを自分で持つ。** */
export type Screen =
  | { readonly route: "chat" }
  | { readonly route: "characters" }
  // **上流はキャラクターに紐づかない**（作業単位 D-9）。
  // 面（本文 / 裏方）は personacore の設定で決まっていて、
  // **どのキャラクターを開いていても同じ**なので、識別子を運ばない。
  | { readonly route: "upstream" }
  // **personacore 自身の設定**（作業単位 F-12b / ADR-0057 §決定5。いまは待ち受けのポート）。
  // **上流の画面には混ぜない**（推論サーバの話と、personacore 自身の待ち受けの話が混ざる）。
  // キャラクターに紐づかないので、識別子を運ばない。
  | { readonly route: "settings" }
  | { readonly route: "memories"; readonly handle: string }
  | { readonly route: "stats"; readonly handle: string }
  // **属性もキャラクターごと**（作業単位 D-12a / `GET .../attributes`）。
  // **変更の記録（`.../attributes/revisions`）は画面ではない** ——
  // 同じ画面の中の節なので、経路を生やさない（2つあると、どちらが権威か読めない）。
  | { readonly route: "attributes"; readonly handle: string }
  // **約束・決まりもキャラクターごと**（作業単位 A-12a / `GET .../agreements`）。
  // **履歴（`.../agreements/transitions`）は画面ではない** —— 同じ画面の中の節。
  | { readonly route: "agreements"; readonly handle: string };

/**
 * **識別子を運ぶ画面の名前**（`#/{名前}/{識別子}`）。
 *
 * **`screenOf` の受理集合はこの配列が決める。** 増やすときはここに足す
 * ——判定を `if` で書き足すと、`hrefFor` との食い違いが静かに生まれる。
 */
const SCOPED: readonly ["memories", "stats", "attributes", "agreements"] = [
  "memories",
  "stats",
  "attributes",
  "agreements",
];

/** 識別子を運ぶ画面の名前か。 */
function isScoped(name: string): name is (typeof SCOPED)[number] {
  return (SCOPED as readonly string[]).includes(name);
}

/**
 * **識別子を運ばない画面の名前**（`#/{名前}`）。
 *
 * **`SCOPED` と同じ理由でここに集める**（作業単位 D-9 で3つ目が増えた）——
 * 判定を `if` で書き足すと、**`hrefFor` との食い違いが静かに生まれる。**
 */
const FLAT: readonly ["chat", "characters", "upstream", "settings"] = [
  "chat",
  "characters",
  "upstream",
  "settings",
];

/** 識別子を運ばない画面の名前か。 */
function isFlat(name: string): name is (typeof FLAT)[number] {
  return (FLAT as readonly string[]).includes(name);
}

/** 画面の名前。 */
export type Route = Screen["route"];

/** 既定の画面。**知らないハッシュはここへ落ちる。** */
export const DEFAULT_SCREEN: Screen = { route: "chat" };

/**
 * **ナビゲーションに並ぶ画面。**
 *
 * **記憶も性格値も属性も並ばない。** 並べるには識別子を1つ決め打つことになり、
 * **どのキャラクターのものかをナビが勝手に決める**ことになる。
 * 入口はキャラクターの一覧（`CharacterList.svelte` の行）。
 *
 * **⚠ 上流の画面は並ぶ**（作業単位 D-9）。**識別子を1つも決め打たない**ので、
 * 上の理由がそもそも当たらない ——上に書いた条件の、ちょうど裏側である。
 * **「設定」も同じ理由で並ぶ**（作業単位 F-12b / ADR-0057 §決定5「ナビに足す」）。最後に置く。
 */
const NAV: readonly Screen[] = [
  { route: "chat" },
  { route: "characters" },
  { route: "upstream" },
  { route: "settings" },
];

/** その画面へのリンク先。**`<a href>` に直接書かない**（綴りの写しが増える）。 */
export function hrefFor(screen: Screen): string {
  switch (screen.route) {
    case "chat":
    case "characters":
    case "upstream":
    case "settings":
      return `#/${screen.route}`;
    case "memories":
    case "stats":
    case "attributes":
    case "agreements":
      // **必ず符号化する**（`api.ts` の `getStats` と同じ理由 ——
      // サーバから来た値をそのままパスへ差し込まない）。
      return `#/${screen.route}/${encodeURIComponent(screen.handle)}`;
  }
}

/** 画面の見出し（リンクの文言）。 */
export function labelFor(screen: Screen): string {
  switch (screen.route) {
    case "chat":
      return "会話";
    case "characters":
      return "キャラクター";
    case "upstream":
      // **「モデル」という語をここだけが名乗る。**
      // 会話の画面の `送り先` はセッションの識別子であって、モデルではない
      // （作業単位 D-9 —— 両方が同じ語を使うと、誤読はそのまま残る）。
      return "上流モデル";
    case "settings":
      return "設定";
    case "memories":
      return `${screen.handle} の記憶`;
    case "stats":
      // **記憶の見出しと別の文言にする** —— 同じだと、どちらの画面を
      // 見ているのかが読み手に分からない（帰属の壊れ方）。
      return `${screen.handle} の性格値と変容`;
    case "attributes":
      // **他の2つと別の文言にする**（同上）。
      // **「変更の記録」まで名乗るのは意図的** —— この画面は
      // 「いまどうなっているか」と「いつ何から何に変わったか」の2つを出す。
      return `${screen.handle} の属性と変更の記録`;
    case "agreements":
      // **他の3つと別の文言にする**（同上）。
      return `${screen.handle} の約束・決まり`;
  }
}

/** ナビゲーションが並べる画面。 */
export function navScreens(): readonly Screen[] {
  return NAV;
}

/**
 * ハッシュを画面へ。**知らない綴りは `null`。**
 *
 * `#/chat` / `#/chat/` / `#chat` / `` （空）を受ける。空は既定と同じ扱いにせず、
 * `null` を返して呼び出し側に `#/chat` を書かせる。
 *
 * **`#/memories` は知らない綴り**（識別子が無い）。ここで空文字の識別子を作ると、
 * **`/admin/characters//memories` を引きに行く画面**ができる。
 * **`#/stats` も `#/attributes` も同じ**（識別子を運ぶ画面は `SCOPED`）。
 *
 * **符号化が壊れていても投げない**（`decodeURIComponent` は不正な `%` で例外を投げる）。
 * ルータが投げると、**アドレス欄を手で書き換えただけで画面が真っ白になる。**
 */
export function screenOf(hash: string): Screen | null {
  const path = hash.replace(/^#/, "").replace(/^\//, "").replace(/\/+$/, "");
  if (isFlat(path)) {
    return { route: path };
  }
  const segments = path.split("/");
  const head = segments[0] ?? "";
  if (segments.length !== 2 || !isScoped(head)) {
    return null;
  }
  const encoded = segments[1] ?? "";
  if (encoded === "") {
    return null;
  }
  let handle: string;
  try {
    handle = decodeURIComponent(encoded);
  } catch {
    return null;
  }
  return { route: head, handle };
}

/** いまのハッシュが指す画面。**知らない綴りなら既定。** */
export function currentScreen(): Screen {
  return screenOf(window.location.hash) ?? DEFAULT_SCREEN;
}

/**
 * ルーティングを始める。**返り値を呼ぶと購読をやめる。**
 *
 * 開始時にも1回 `onChange` を呼ぶので、呼び出し側が初期値を別途取らなくてよい
 * （2箇所で取ると、片方だけ古びる形になる）。
 */
export function startRouting(onChange: (screen: Screen) => void): () => void {
  const apply = (): void => {
    const known = screenOf(window.location.hash);
    if (known === null) {
      // **URL を直す。** これ自身が `hashchange` を起こすので、
      // ここで `onChange` は呼ばない（二重に呼ぶと画面が2回描き直される）。
      window.location.replace(hrefFor(DEFAULT_SCREEN));
      return;
    }
    onChange(known);
  };

  window.addEventListener("hashchange", apply);
  apply();
  return () => {
    window.removeEventListener("hashchange", apply);
  };
}
