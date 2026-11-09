// @vitest-environment jsdom
/**
 * **「設定」の画面**（作業単位 F-12b / ADR-0057 §決定5 / `src/lib/SettingsPanel.svelte`）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **応答の各状態が、最終的な DOM の行（`<dt>` と `<dd>` の組）として出ること** | **ここ** |
 * | **保存できない起動では、入力と操作を出さず、理由を言うこと** | **ここ** |
 * | **送る本文**（`{"port": n}` / `{"port": null}`）と、**数字でない入力は送らないこと** | **ここ** |
 * | **保存の後の文と、応答で行が置き換わること**（読み直しの GET を足さない） | **ここ** |
 * | **失敗の文が `role="alert"` の段落に出ること** | **ここ** |
 * | **周期で読み直さないこと** | **ここ**（偽の時計） |
 * | 文の中身そのもの（全部の枝） | `tests/server-settings.test.ts` |
 *
 * **帰属は DOM の中身で見る**（`data-*` で代えない / CLAUDE.md「見張り番の書き方」）。
 * **応答の番号は欄ごとに全部違う値**にしてあり、保存の応答は読んだ応答と1つも同じ番号を持たない
 * —— 応答を採らずに古い行を残す実装も、欄を取り違える実装も赤くなる。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SavePortView, ServerView } from "../src/lib/api";
import SettingsPanel from "../src/lib/SettingsPanel.svelte";

/** 開いたときに読む応答（保存した番号で起き、次も保存した番号）。 */
const READ: ServerView = {
  listening_port: 8123,
  listening_port_source: "saved",
  saved_port: 8456,
  saved_port_unreadable: false,
  next_port: 8789,
  next_port_source: "saved",
  editable: true,
};

/** 保存の応答（**読んだ応答と1つも同じ番号を持たない**。いまの待ち受けだけは変わらない）。 */
const SAVED: SavePortView = {
  listening_port: 8123,
  listening_port_source: "saved",
  saved_port: 9001,
  saved_port_unreadable: false,
  next_port: 9001,
  next_port_source: "saved",
  editable: true,
  saved_port_in_use: false,
};

interface Sent {
  url: string;
  method: string;
  body: string | null;
  contentType: string | null;
}

let target: HTMLElement;
let sent: Sent[];
/** 次の GET の応答。 */
let readBody: unknown;
/** 次の PUT の応答（`status` が 200 以外なら失敗の本文）。 */
let putResponse: { status: number; body: unknown } | "unreachable";

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const headers = (init?.headers ?? {}) as Record<string, string>;
    sent.push({
      url,
      method: init?.method ?? "GET",
      body: typeof init?.body === "string" ? init.body : null,
      contentType: headers["content-type"] ?? null,
    });
    const respond = (status: number, body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);
    if (url === "/admin/server") {
      return respond(200, readBody);
    }
    if (url === "/admin/server/port") {
      if (putResponse === "unreachable") {
        return Promise.reject(new TypeError("Failed to fetch"));
      }
      return respond(putResponse.status, putResponse.body);
    }
    return respond(404, {});
  });
}

function errorBody(code: string, message: string): unknown {
  return { error: { message, type: "invalid_request_error", code } };
}

/** 行を `[見出し, 値]` の配列で（**表示されたままの文字**）。 */
function rows(): string[][] {
  return [...target.querySelectorAll("dl > div")].map((row) =>
    [...row.querySelectorAll("dt, dd")].map((cell) =>
      (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
    ),
  );
}

function text(): string {
  return (target.textContent ?? "").replace(/\s+/gu, " ");
}

function buttonNamed(name: string): HTMLButtonElement | undefined {
  return [...target.querySelectorAll<HTMLButtonElement>("button")].find(
    (button) => (button.textContent ?? "").trim() === name,
  );
}

/** 番号の入力欄（**見出しの文字から辿る**）。 */
function portInput(): HTMLInputElement | null {
  const label = [...target.querySelectorAll("label")].find((found) =>
    (found.textContent ?? "").includes("保存する番号"),
  );
  if (label === undefined) {
    return null;
  }
  return target.querySelector<HTMLInputElement>(`#${label.htmlFor}`);
}

function type(value: string): void {
  const input = portInput();
  if (input === null) {
    throw new Error("入力欄が無い");
  }
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function submit(): void {
  const form = portInput()?.closest("form");
  if (form === null || form === undefined) {
    throw new Error("フォームが無い");
  }
  form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

function alerts(): string[] {
  return [...target.querySelectorAll('[role="alert"]')].map((found) =>
    (found.textContent ?? "").replace(/\s+/gu, " ").trim(),
  );
}

function statuses(): string[] {
  return [...target.querySelectorAll('[role="status"]')].map((found) =>
    (found.textContent ?? "").replace(/\s+/gu, " ").trim(),
  );
}

async function opened(): Promise<ReturnType<typeof mount>> {
  const view = mount(SettingsPanel, { target });
  await vi.waitFor(() => {
    expect(rows().length).toBe(3);
  });
  return view;
}

beforeEach(() => {
  sent = [];
  readBody = READ;
  putResponse = { status: 200, body: SAVED };
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("開いたとき", () => {
  it("1回だけ読み、いま・保存・次の起動を出す", async () => {
    const view = await opened();
    try {
      expect(target.querySelector("h1")?.textContent?.trim()).toBe("設定");
      expect(rows()).toEqual([
        ["いま待ち受けている番号", "8123（保存した番号）"],
        ["保存した番号", "8456"],
        ["次の起動で使う番号", "8789（保存した番号）"],
      ]);
      expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual(["GET /admin/server"]);
      // **入力欄の初めの値は保存した番号。**
      expect(portInput()?.value).toBe("8456");
      expect(buttonNamed("保存")?.disabled).toBe(false);
      expect(buttonNamed("保存を消して既定に戻す")?.disabled).toBe(false);
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("出どころが環境変数・既定のときも、行がそう言う", async () => {
    readBody = {
      ...READ,
      listening_port_source: "environment",
      saved_port: null,
      next_port_source: "default",
    };
    const view = await opened();
    try {
      expect(rows()).toEqual([
        ["いま待ち受けている番号", "8123（環境変数 PERSONACORE_PORT）"],
        ["保存した番号", "保存していません"],
        ["次の起動で使う番号", "8789（既定の番号）"],
      ]);
      // **保存していなければ、消す操作は押せない**（押しても何も変わらない）。
      expect(buttonNamed("保存を消して既定に戻す")?.disabled).toBe(true);
      expect(portInput()?.value).toBe("");
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("保存のファイルが読めないときは、そう言って消せるようにする", async () => {
    readBody = { ...READ, saved_port: null, saved_port_unreadable: true, next_port_source: "default" };
    const view = await opened();
    try {
      expect(rows()[1]).toEqual(["保存した番号", "読めません（保存のファイルが壊れています）"]);
      expect(text()).toContain(
        "保存のファイル（listening-port.txt）が読めません。読めない間は、保存した番号を使いません。" +
          "番号を保存し直すとファイルが置き換わり、「保存を消して既定に戻す」を押すとファイルが消えます。",
      );
      expect(buttonNamed("保存を消して既定に戻す")?.disabled).toBe(false);
    } finally {
      await unmount(view, { outro: false });
    }
  });

  /** **対照: 読めるときは、読めないことを言わない。** */
  it("対照: 保存のファイルが読めるときは、読めないと言わない", async () => {
    const view = await opened();
    try {
      expect(text()).not.toContain("listening-port.txt");
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("保存できない起動では、理由を言い、入力と操作を出さない", async () => {
    readBody = { ...READ, saved_port: null, editable: false, listening_port_source: "default", next_port_source: "default" };
    const view = await opened();
    try {
      expect(rows()[1]).toEqual(["保存した番号", "この起動では読みません"]);
      expect(text()).toContain(
        "この起動では番号を保存できません。環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しているためです",
      );
      expect(portInput()).toBeNull();
      expect(target.querySelector("input")).toBeNull();
      expect(target.querySelector("form")).toBeNull();
      expect(buttonNamed("保存")).toBeUndefined();
      expect(buttonNamed("保存を消して既定に戻す")).toBeUndefined();
    } finally {
      await unmount(view, { outro: false });
    }
  });

  /** **対照: 保存できる起動では、保存できないと言わない。** */
  it("対照: 保存できる起動では、保存できないと言わない", async () => {
    const view = await opened();
    try {
      expect(text()).not.toContain("この起動では番号を保存できません");
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("読めなかったら断りを言い、再試行で読み直す", async () => {
    vi.stubGlobal("fetch", (input: unknown) => {
      sent.push({ url: String(input), method: "GET", body: null, contentType: null });
      return Promise.reject(new TypeError("Failed to fetch"));
    });
    const view = mount(SettingsPanel, { target });
    try {
      await vi.waitFor(() => {
        expect(alerts()).toEqual([
          "設定を取得できませんでした: サーバへ接続できませんでした（personacore は動いていますか） 再試行",
        ]);
      });
      stubFetch();
      buttonNamed("再試行")?.click();
      await vi.waitFor(() => {
        expect(rows().length).toBe(3);
      });
      expect(alerts()).toEqual([]);
      expect(sent.map((s) => s.url)).toEqual(["/admin/server", "/admin/server"]);
    } finally {
      await unmount(view, { outro: false });
    }
  });
});

describe("保存する", () => {
  it("`{\"port\": n}` を PUT で送り、応答で行を置き換え、次の起動から効くと言う", async () => {
    const view = await opened();
    try {
      type("9001");
      submit();
      await vi.waitFor(() => {
        expect(rows()[1]).toEqual(["保存した番号", "9001"]);
      });
      const put = sent[1];
      expect(put?.url).toBe("/admin/server/port");
      expect(put?.method).toBe("PUT");
      expect(put?.contentType).toBe("application/json");
      expect(JSON.parse(put?.body ?? "null")).toEqual({ port: 9001 });
      // **読み直しの GET を足さない**（保存の応答がサーバの読み直しである）。
      expect(sent).toHaveLength(2);
      expect(rows()).toEqual([
        ["いま待ち受けている番号", "8123（保存した番号）"],
        ["保存した番号", "9001"],
        ["次の起動で使う番号", "9001（保存した番号）"],
      ]);
      expect(statuses()).toContain(
        "9001 を保存しました。次の起動から効きます。いま待ち受けている 8123 は、この起動のあいだ移りません。",
      );
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("環境変数が効いているときは、保存しても次の起動では使われないと言う", async () => {
    readBody = { ...READ, listening_port_source: "environment", next_port_source: "environment" };
    putResponse = {
      status: 200,
      body: { ...SAVED, listening_port_source: "environment", next_port: 8123, next_port_source: "environment" },
    };
    const view = await opened();
    try {
      type("9001");
      submit();
      await vi.waitFor(() => {
        expect(statuses()).toContain(
          "9001 を保存しました。PERSONACORE_PORT が効いているので、保存しても次の起動では使われません。" +
            "いま待ち受けている 8123 は、この起動のあいだ移りません。",
        );
      });
      // **報せの段落に「次の起動から効きます」を並べない**（画面の冒頭の説明文は同じ語を含むので、報せだけを見る）。
      expect(statuses().join(" ")).not.toContain("次の起動から効きます");
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("その番号をいま他のプログラムが使っていれば、そう添える（保存はしてある）", async () => {
    putResponse = { status: 200, body: { ...SAVED, saved_port_in_use: true } };
    const view = await opened();
    try {
      type("9001");
      submit();
      await vi.waitFor(() => {
        expect(statuses()).toContain(
          "9001 を保存しました。この番号はいま他のプログラムが使っています（保存はしてあります）。" +
            "次の起動から効きます。いま待ち受けている 8123 は、この起動のあいだ移りません。",
        );
      });
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("対照: 使われていなければ、使われているとは言わない", async () => {
    const view = await opened();
    try {
      type("9001");
      submit();
      await vi.waitFor(() => {
        expect(rows()[1]).toEqual(["保存した番号", "9001"]);
      });
      expect(text()).not.toContain("他のプログラム");
    } finally {
      await unmount(view, { outro: false });
    }
  });

  /** **範囲はサーバが決める。** 範囲の外でも送り、400 の答えを文にする。 */
  it("範囲の外の番号は送って、400 `invalid_listening_port` を文にする", async () => {
    putResponse = {
      status: 400,
      body: errorBody("invalid_listening_port", "待ち受けのポートには 1024〜65535 の番号を指定してください"),
    };
    const view = await opened();
    try {
      type("80");
      submit();
      await vi.waitFor(() => {
        expect(alerts()).toEqual(["保存しませんでした。番号は 1024〜65535 の範囲で入れてください。"]);
      });
      expect(JSON.parse(sent[1]?.body ?? "null")).toEqual({ port: 80 });
      // **行は変わらない**（断られたので、読んだときのまま）。
      expect(rows()[1]).toEqual(["保存した番号", "8456"]);
    } finally {
      await unmount(view, { outro: false });
    }
  });

  it("409・500・繋がらないも、それぞれ文にする", async () => {
    const cases: [typeof putResponse, string][] = [
      [
        { status: 409, body: errorBody("listening_port_read_only", "この起動では待ち受けのポートを保存できません") },
        "保存しませんでした。この起動では番号を保存できません（環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しています）。",
      ],
      [
        { status: 500, body: errorBody("listening_port_write_failed", "書き換えられませんでした") },
        "保存のファイルを書き換えられませんでした。前に保存した番号はそのまま残っています。",
      ],
      [
        "unreachable",
        "サーバへ接続できませんでした（personacore は動いていますか）。保存できたかどうかは、この画面を開き直すと分かります。",
      ],
    ];
    for (const [response, expected] of cases) {
      putResponse = response;
      const view = await opened();
      try {
        type("9001");
        submit();
        await vi.waitFor(() => {
          expect(alerts()).toEqual([expected]);
        });
      } finally {
        await unmount(view, { outro: false });
        target.replaceChildren();
      }
    }
  });

  it("数字でない入力は送らず、そう言う", async () => {
    const view = await opened();
    try {
      type("90a1");
      submit();
      flushSync();
      expect(alerts()).toEqual(["番号は半角の数字だけで入れてください。"]);
      type("");
      submit();
      flushSync();
      expect(alerts()).toEqual(["番号を入れてください。"]);
      expect(sent.map((s) => s.url)).toEqual(["/admin/server"]);
    } finally {
      await unmount(view, { outro: false });
    }
  });

  /** **前の失敗を、次の成功の後に残さない。** */
  it("失敗の後に保存できたら、失敗の文を消す", async () => {
    const view = await opened();
    try {
      type("90a1");
      submit();
      flushSync();
      expect(alerts()).toHaveLength(1);
      type("9001");
      submit();
      await vi.waitFor(() => {
        expect(rows()[1]).toEqual(["保存した番号", "9001"]);
      });
      expect(alerts()).toEqual([]);
    } finally {
      await unmount(view, { outro: false });
    }
  });
});

describe("保存を消す", () => {
  it("`{\"port\": null}` を送り、次の起動から既定の番号を使うと言う", async () => {
    putResponse = {
      status: 200,
      body: {
        ...SAVED,
        saved_port: null,
        next_port: 8080,
        next_port_source: "default",
        saved_port_in_use: null,
      },
    };
    const view = await opened();
    try {
      buttonNamed("保存を消して既定に戻す")?.click();
      await vi.waitFor(() => {
        expect(rows()[1]).toEqual(["保存した番号", "保存していません"]);
      });
      expect(sent[1]?.method).toBe("PUT");
      expect(JSON.parse(sent[1]?.body ?? "undefined")).toEqual({ port: null });
      // **`port` を省かない**（省いた本文はサーバが 400 で断る）。
      expect(sent[1]?.body).toBe('{"port":null}');
      expect(sent).toHaveLength(2);
      expect(statuses()).toContain(
        "保存を消しました。次の起動から 8080（既定の番号）を使います。いま待ち受けている 8123 は、この起動のあいだ移りません。",
      );
      // **消した後は、もう一度消す操作を押せない。**
      expect(buttonNamed("保存を消して既定に戻す")?.disabled).toBe(true);
    } finally {
      await unmount(view, { outro: false });
    }
  });
});

describe("読み直し", () => {
  /**
   * **周期で読み直さない**（GET は軽いが、周期にする理由が無い / 作業単位 F-12b の指示）。
   * 偽の時計は `setTimeout` / `setInterval` の系だけを差し替える（Svelte の予約は触らない）。
   */
  it("開いた後、時間が経っても読み直さない", async () => {
    const realSetInterval = globalThis.setInterval;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
    try {
      // **対照: 偽の時計が、本物を置き換えている。**
      expect(globalThis.setInterval).not.toBe(realSetInterval);
      const view = mount(SettingsPanel, { target });
      await vi.advanceTimersByTimeAsync(0);
      expect(sent.map((s) => s.url)).toEqual(["/admin/server"]);
      await vi.advanceTimersByTimeAsync(10 * 60 * 1000);
      expect(sent.map((s) => s.url)).toEqual(["/admin/server"]);
      await unmount(view, { outro: false });
    } finally {
      vi.useRealTimers();
    }
  });
});
