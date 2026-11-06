import { mount } from "svelte";

import App from "./App.svelte";

// **画面の土台**（色・字・共通の部品 / 作業単位 D-10）。
// **`index.html` の `<link>` ではなくここから取り込む** —— 束ねる対象になるので、
// **読めなくなったら**ビルドが落ちる（理由は `app.css` の冒頭）。
//
// **⚠ この1行を消しても、ビルドもテストも通る。** 土台が丸ごと効かなくなるだけ。
// **見張っているのは `tests/design-tokens.test.ts` の
// 「土台を、束ねる対象として取り込んでいる」1本だけ**（D-10 のレビューで実測）。
import "./app.css";

const target = document.querySelector("#app");
if (!(target instanceof HTMLElement)) {
  throw new Error("#app が見つかりません（index.html を確認してください）");
}

export default mount(App, { target });
