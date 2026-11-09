/**
 * **「設定」の画面が言うこと**（作業単位 F-12b / ADR-0057 §決定5）。**I/O を持たない。**
 *
 * `GET /admin/server` / `PUT /admin/server/port` の応答（`api.ts` の `ServerView` / `SavePortView`）から、
 * 行・断り書き・保存の後の文・失敗の文を組む。**画面（`SettingsPanel.svelte`）は並べるだけ。**
 * 見張り番は `tests/server-settings.test.ts`（文の全部の枝）と `tests/settings-panel.test.ts`（画面）。
 *
 * # 範囲の判定はサーバが持つ
 *
 * **保存できる番号（1024〜65535）をここで判定しない** —— 判定が2つあると片方だけが古びる。
 * ここが入力に言うのは「空か」「半角の数字だけか」まで（数でない本文は送っても `invalid_body` で断られ、
 * 理由が番号の範囲に結びつかない）。範囲の外はそのまま送り、400 `invalid_listening_port` を文にする。
 *
 * # 「いまの待ち受けは移らない」を、保存の後の文に必ず入れる
 *
 * **保存は次の起動から効く**（サーバは動いている間に待ち受けを移さない）。
 * 保存した直後に、いまの画面の URL が変わると読まれないよう、移らないことを毎回言う。
 */

import { ApiRequestError, type SavePortView, type ServerView } from "./api";

/**
 * **失敗の読み分けに使う `code`**（`src/server/error.rs` の写し。突き合わせは `tests/server-settings.test.ts`）。
 */
export const SAVE_FAILURE_CODES = {
  /** 400: 1024〜65535 の外。 */
  outOfRange: "invalid_listening_port",
  /** 400: 本文が読めない（`port` が整数でない等）。 */
  unreadableBody: "invalid_body",
  /** 409: この起動では保存できない。 */
  readOnly: "listening_port_read_only",
  /** 500: ファイルを書き換えられなかった。 */
  writeFailed: "listening_port_write_failed",
} as const;

/** **出どころの言い方。** 知らない綴りは丸めずにそのまま見せる（サーバが綴りを足した日に嘘をつかない）。 */
export function portSourceLabel(source: string): string {
  switch (source) {
    case "environment":
      return "環境変数 PERSONACORE_PORT";
    case "saved":
      return "保存した番号";
    case "default":
      return "既定の番号";
    default:
      return `知らない出どころ: ${source}`;
  }
}

/** 画面の1行（見出しと値）。 */
export interface SettingRow {
  readonly label: string;
  readonly value: string;
}

/** 番号と、その出どころ。 */
function portWithSource(port: number, source: string): string {
  return `${port}（${portSourceLabel(source)}）`;
}

/**
 * **保存した番号の行の値。**
 *
 * **3つの「`saved_port` が `null`」を見分ける** —— 保存できない起動（読みもしない）・壊れたファイルが在る・
 * 何も保存していない。**保存できない起動で「保存していません」と言うと嘘になる**（ファイルは在りうる）。
 */
function savedPortText(view: ServerView): string {
  if (!view.editable) {
    return "この起動では読みません";
  }
  if (view.saved_port_unreadable) {
    return "読めません（保存のファイルが壊れています）";
  }
  if (view.saved_port !== null) {
    return String(view.saved_port);
  }
  return "保存していません";
}

/** **次の起動で使う番号の行の値。** `null` は `PERSONACORE_PORT` が番号として読めない（次の起動は止まる）。 */
function nextPortText(view: ServerView): string {
  if (view.next_port === null) {
    return "決まりません（PERSONACORE_PORT を番号として読めないので、次の起動は止まります）";
  }
  return portWithSource(view.next_port, view.next_port_source);
}

/**
 * **画面の行**（いま → 保存 → 次の起動の順）。
 *
 * **いまの出どころと次の出どころは別の欄から読む**（保存した直後は、いまは既定・次は保存、と食い違うのが正しい）。
 */
export function serverRows(view: ServerView): SettingRow[] {
  return [
    {
      label: "いま待ち受けている番号",
      value: portWithSource(view.listening_port, view.listening_port_source),
    },
    { label: "保存した番号", value: savedPortText(view) },
    { label: "次の起動で使う番号", value: nextPortText(view) },
  ];
}

/** **保存できない起動の断り書き**（`editable: false`）。画面は入力と操作を出さずに、これを言う。 */
export const READ_ONLY_NOTICE =
  "この起動では番号を保存できません。環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しているためです" +
  "（scripts/start.ps1 を -Upstream 付きで起動したときも、これに当たります）。この起動は保存した番号を読みません。";

/**
 * **保存のファイルが読めないときの断り書き**（`saved_port_unreadable: true`）。
 *
 * **「既定で起きています」とは言わない** —— `PERSONACORE_PORT` が効いている起動もあり、
 * 起動の後にファイルが壊れた場合もある。いまの番号の出どころは行が言う。
 */
export const UNREADABLE_NOTICE =
  "保存のファイル（listening-port.txt）が読めません。読めない間は、保存した番号を使いません。" +
  "番号を保存し直すとファイルが置き換わり、「保存を消して既定に戻す」を押すとファイルが消えます。";

/** **いまの待ち受けは移らない**（保存の後の文の最後に必ず添える）。 */
function staysText(view: ServerView): string {
  return `いま待ち受けている ${view.listening_port} は、この起動のあいだ移りません。`;
}

/**
 * **保存・消去の後の文。** `requested` は送った番号（`null` は消した）。
 *
 * | 場合 | 言うこと |
 * |---|---|
 * | 保存した | 次の起動から効く。**環境変数が効いていれば、次の起動では使われない** |
 * | 保存した番号を、いま他のプログラムが使っている | そう添える（**保存はしてある**） |
 * | 消した | 次の起動で使う番号と出どころ。**環境変数が効いていれば、次の起動もその番号** |
 * | どれでも | いまの待ち受けは移らない |
 */
export function describeSaved(requested: number | null, view: SavePortView): string {
  const environment = view.next_port_source === "environment";
  const parts: string[] = [];
  if (requested === null) {
    parts.push("保存を消しました。");
    if (environment) {
      parts.push("PERSONACORE_PORT が効いているので、次の起動もその番号を使います。");
    } else if (view.next_port !== null) {
      parts.push(`次の起動から ${portWithSource(view.next_port, view.next_port_source)}を使います。`);
    }
  } else {
    parts.push(`${requested} を保存しました。`);
    if (view.saved_port_in_use === true) {
      parts.push("この番号はいま他のプログラムが使っています（保存はしてあります）。");
    }
    parts.push(
      environment
        ? "PERSONACORE_PORT が効いているので、保存しても次の起動では使われません。"
        : "次の起動から効きます。",
    );
  }
  parts.push(staysText(view));
  return parts.join("");
}

/**
 * **保存・消去の失敗の文。** `requested` は送った番号（`null` は消そうとした）。
 *
 * **繋がらなかったときは、保存できたかどうかが分からない**（届いて書いた後に切れたかもしれない）ので、そう言う。
 */
export function describeSaveFailure(thrown: unknown, requested: number | null): string {
  if (!(thrown instanceof ApiRequestError)) {
    return "予期しない失敗です";
  }
  const refused = requested === null ? "保存を消しませんでした" : "保存しませんでした";
  switch (thrown.code) {
    case SAVE_FAILURE_CODES.outOfRange:
      return `${refused}。番号は 1024〜65535 の範囲で入れてください。`;
    case SAVE_FAILURE_CODES.unreadableBody:
      return `${refused}。番号として読めませんでした（1024〜65535 の半角の数字で入れてください）。`;
    case SAVE_FAILURE_CODES.readOnly:
      return `${refused}。この起動では番号を保存できません（環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しています）。`;
    case SAVE_FAILURE_CODES.writeFailed:
      return requested === null
        ? "保存のファイルを消せませんでした。前に保存した番号はそのまま残っています。"
        : "保存のファイルを書き換えられませんでした。前に保存した番号はそのまま残っています。";
    default:
      break;
  }
  if (thrown.status === 0) {
    return `${thrown.message}。保存できたかどうかは、この画面を開き直すと分かります。`;
  }
  return `${refused}: ${thrown.message}`;
}

/** 入力を読んだ結果。 */
export type PortInput =
  | { readonly ok: true; readonly port: number }
  | { readonly ok: false; readonly message: string };

/**
 * **入力を番号として読む。** 前後の空白を落とし、**半角の数字だけ**なら番号にする。
 *
 * **範囲は見ない**（このファイルの冒頭）。`+9001` や全角の数字は断る —— サーバは整数の JSON しか受けず、
 * こちらで黙って直すと、送った値と見えている値が食い違う。
 *
 * **⚠ 正確な整数として持てない大きさ（`Number.isSafeInteger` でない）は送らない**（作業単位 F-12b のレビュー）。
 * 309 桁以上は `Infinity` になり、`JSON.stringify` が `{"port":null}` に変える ——**サーバはそれを
 * 「保存を消す」と読み、画面は「Infinity を保存しました」と言っていた。** 16〜17 桁でも丸めが起き、
 * 打った数と違う数を送る。これは範囲の判定ではない（送れる値かどうか。範囲は今もサーバが決める）。
 */
export function parsePortInput(text: string): PortInput {
  const trimmed = text.trim();
  if (trimmed === "") {
    return { ok: false, message: "番号を入れてください。" };
  }
  if (!/^[0-9]+$/u.test(trimmed)) {
    return { ok: false, message: "番号は半角の数字だけで入れてください。" };
  }
  const port = Number(trimmed);
  if (!Number.isSafeInteger(port)) {
    return { ok: false, message: "番号が大きすぎます（1024〜65535 の番号を入れてください）。" };
  }
  return { ok: true, port };
}

/** **入力欄の初めの値**——保存した番号（無ければ空）。 */
export function initialDraft(view: ServerView): string {
  return view.saved_port === null ? "" : String(view.saved_port);
}

/**
 * **「保存を消して既定に戻す」を押せるか。** 保存しているか、壊れたファイルが在るときだけ
 * （何も無いときに押せると、何も変わらないのに「消しました」と言うことになる）。
 */
export function canClear(view: ServerView): boolean {
  return view.editable && (view.saved_port !== null || view.saved_port_unreadable);
}
