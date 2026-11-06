// @vitest-environment jsdom
/**
 * **セッションの固定と、保存されたかどうか**（作業単位 B-8 /
 * D-0b のレビューが列挙した穴の3つ目）。
 *
 * # ここで見るもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 中止したときの結末 | `chat-abort.test.ts` |
 * | `rejected` / `upstream-error` / `unreadable` の経路 | `chat-outcomes.test.ts` |
 * | 後始末・「中止」ボタン | `chat-view-teardown.test.ts` / `chat-view-abort.test.ts` |
 * | **`sessionWarning` と `isPersisted` が画面のどこに出るか** | **ここ** |
 *
 * # なぜ、いま要るのか
 *
 * **`sessionWarning` は、ADR-0004 がいちばん避けたい壊れ方の唯一の出口。**
 * 応答が別のキャラクターのセッションを名乗ったとき、
 * **黙って固定すると別キャラクターの記憶に書き込みに行く。**
 * それでも `ChatView.svelte` の側は**1本も測られていなかった。**
 *
 * **`isPersisted` も同じ。** 「この回は保存されていません」は
 * **画面と履歴の食い違いを利用者に伝える唯一の文**（ADR-0019 §4）で、
 * **どの回に付くか**が意味そのもの。**出ているかだけを見ても足りない。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が選ぶキャラクター。**`handle` と `display_name` を別の値にしてある。** */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/**
 * **このキャラクターのセッション。** 番号は件数（1・2）とも別の数にしてある。
 */
const OWN_SESSION = `${HANDLE}@41`;

/** **別のキャラクターのセッション。** 綴りが全く違う。 */
const OTHER_SESSION = "midori@7";

/**
 * **接頭辞だけが同じ、別のキャラクターのセッション。**
 *
 * `aoi-2` は `aoi` とは**別のキャラクター**（`Handle` は `-` を許す / ADR-0017）。
 * **`startsWith(handle)` だけを見る実装はここで落ちる**——
 * `aoi-2@9` を `aoi` のセッションとして固定し、
 * **以後この画面の発言が全部 `aoi-2` の履歴へ入る。**
 */
const PREFIX_SESSION = "aoi-2@9";

/** 送る本文。**2回ぶんとも別の文字列。** */
const FIRST_DRAFT = "ただいま";
const SECOND_DRAFT = "おなかすいた";

/** 応答の本文。**送った本文のどちらとも別の文字列。** */
const FIRST_REPLY = "おかえりなさい";
const SECOND_REPLY = "何か作ろうか";

/** 画面に出る文の目印。**綴りを1箇所にまとめる**（探す側と出す側を揃えるため）。 */
const NOT_PERSISTED = "サーバに保存されていません";
const EMPTY_RESPONSE = "応答が空でした。";
const TRUNCATED_NOTICE = "応答が完結する前に接続が終わりました";

/** `elapsed >= 5` のときだけ出る但し書き。**綴りを1箇所にまとめる。** */
const HINT_SLOW_FIRST_TOKEN = "数十秒かかることがあります";

/**
 * **台詞を1つ含む本文。** 門の内と外で見え方が変わることを見るのに使う
 * （`[DONE]` まで届けば `「やあ」` が着色され、途中で切れれば生のまま）。
 */
const PARTIAL_REPLY = "「やあ」微笑んだ";

/** 送り先の意味（`outgoingMeaning`）。 */
const MEANING_LATEST = "最新の会話の続き";
const MEANING_PINNED = "このセッションの続き";

/**
 * 1回の送信で流すもの。
 *
 * 流し切ったら**そのまま終わる**ので、
 * `[DONE]` を入れなければ `truncated`、入れれば `done` になる。
 *
 * **`null` は「その読み取りが永久に返らない」**——送信中のまま止める。
 * 受信中の表示を見るには、**終わらない回**が要る。
 */
type Script = (string | null)[];

/** これから流す台本（送信のたびに先頭から1つ取る）。 */
let scripts: Script[];
let target: HTMLElement;

/** 1件ぶんの SSE イベント。 */
function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

/** 本文の増分1件。 */
function delta(model: string, text: string): string {
  return event({ model, choices: [{ delta: { content: text } }] });
}

/** 本文を持たない先頭チャンク（`model` だけ分かる）。 */
function roleOnly(model: string): string {
  return event({ model, choices: [{ delta: { role: "assistant" } }] });
}

/** 終端の番兵。 */
const DONE_EVENT = "data: [DONE]\n\n";

/** `[DONE]` まで届く、ふつうの1回。 */
function completed(model: string, text: string): Script {
  return [delta(model, text), DONE_EVENT];
}

/**
 * `fetch` の代わり。**本物の `Response` を組み立てない**
 * （`chat-view-teardown.test.ts` と同じ理由——jsdom 側の実装差を測定に混ぜない）。
 */
function stubFetch(): void {
  const encoder = new TextEncoder();
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);

    // **会話の一覧**（作業単位 D-12c-2）。**この測定は過去の会話を持たない**（空の一覧）。
    // 足さないと、画面を開いたときの一覧の要求が下の台本を1本食う。
    // **本文の面の状態**（作業単位 D-16）。**外の上流を返す**（送信を止めない）。
    // 足さないと、画面を開いたときの状態の要求が下の台本を1本食う（`/sessions` と同じ）。
    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
    }
    // **約束・決まりの提案**（作業単位 A-12c-2）。足さないと、開いたときの読みが台本を1本食う。
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }

    if (url.endsWith("/sessions")) {
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () =>
          Promise.resolve(
            JSON.stringify({
              character: "",
              sessions: [],
              total_sessions: 0,
              sessions_limit: 100,
            }),
          ),
      } as unknown as Response);
    }

    if (url === "/admin/characters") {
      const body = JSON.stringify({
        characters: [
          {
            id: 1,
            handle: HANDLE,
            display_name: DISPLAY_NAME,
            base_persona: "静かな声で話す。",
            output_format_instruction: null,
            stat_scale: 5,
            created_at: "2026-08-27T00:00:00Z",
          },
        ],
        unreadable: [],
      });
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(body),
      } as unknown as Response);
    }

    const script = scripts.shift() ?? [];
    let index = 0;
    return Promise.resolve({
      ok: true,
      status: 200,
      body: {
        getReader: () => ({
          read: () => {
            const next = script[index];
            index += 1;
            if (next === undefined) {
              return Promise.resolve({ done: true, value: undefined });
            }
            if (next === null) {
              // **返らない読み取り。** その回は送信中のまま止まる。
              return new Promise<never>(() => undefined);
            }
            return Promise.resolve({ done: false, value: encoder.encode(next) });
          },
          cancel: () => Promise.resolve(undefined),
        }),
      },
    } as unknown as Response);
  });
}

/** キャラクターが1件届くまで待つ（届かないと送信の口が開かない）。 */
async function waitForCharacters(): Promise<void> {
  const select = await vi.waitFor(() => {
    const found = target.querySelector("select");
    expect(found).not.toBeNull();
    return found as HTMLSelectElement;
  });
  expect(select.value).toBe(HANDLE);
}

/** 本文を入れて送信するところまで。**結末は待たない。** */
function typeAndSubmit(text: string): void {
  const textarea = target.querySelector("textarea");
  expect(textarea).not.toBeNull();
  const input = textarea as HTMLTextAreaElement;
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();

  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
}

/** 本文を入れて送信し、**その回の結末が画面に出るまで**待つ。 */
async function submit(text: string): Promise<void> {
  const before = assistantTurns().length;
  typeAndSubmit(text);

  // **「受信中…」が消えるまで**待つ。消えていなければ結末が入っていない。
  await vi.waitFor(() => {
    const turns = assistantTurns();
    expect(turns.length).toBe(before + 1);
    const last = turns[turns.length - 1];
    expect(last).toBeDefined();
    expect(notesOf(last as HTMLElement).join("")).not.toContain("秒");
  });
}

/** 相手の側の行だけ。**利用者の行は数に入れない。** */
function assistantTurns(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("ol.turns > li.assistant")];
}

/** **その行の中の**注記。 */
function notesOf(turn: HTMLElement): string[] {
  return [...turn.querySelectorAll("p.note")].map(
    (note) => note.textContent?.trim() ?? "",
  );
}

/** **その行の中の**エラー文。 */
function errorsOf(turn: HTMLElement): string[] {
  return [...turn.querySelectorAll("p.error")].map(
    (error) => error.textContent?.trim() ?? "",
  );
}

/** **画面の上部に出る**エラー文（`sessionWarning` / `charactersError`）。 */
function bannerErrors(): string[] {
  return [...target.querySelectorAll("section > p.error")].map(
    (error) => error.textContent?.trim() ?? "",
  );
}

/** いま送ろうとしている `model`。**`.wire` の中の `<code>` だけを見る。** */
function outgoingModel(): string {
  const code = target.querySelector("p.wire code");
  expect(code).not.toBeNull();
  return (code as HTMLElement).textContent?.trim() ?? "";
}

/** その `model` が何を意味すると画面が言っているか。 */
function outgoingMeaning(): string {
  const muted = target.querySelector("p.wire .meaning");
  expect(muted).not.toBeNull();
  return (muted as HTMLElement).textContent?.trim() ?? "";
}

beforeEach(() => {
  scripts = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("応答が名乗ったセッション", () => {
  it("自分のセッションなら固定する（対照）", async () => {
    scripts = [completed(OWN_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    // 固定する前は「最新の会話の続き」。
    expect(outgoingModel()).toBe(HANDLE);
    expect(outgoingMeaning()).toContain(MEANING_LATEST);

    await submit(FIRST_DRAFT);

    // **固定された。** 次の送信はこのセッションへ行く。
    expect(outgoingModel()).toBe(OWN_SESSION);
    expect(outgoingMeaning()).toContain(MEANING_PINNED);
    // **警告は出ない。**
    expect(bannerErrors()).toEqual([]);

    await unmount(view, { outro: false });
  });

  it("別のキャラクターのセッションを名乗ったら、固定せずに画面へ出す", async () => {
    scripts = [completed(OTHER_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    // **警告が1本、画面の上部に出る。** 名乗った側の綴りが入っている。
    const banners = bannerErrors();
    expect(banners.length).toBe(1);
    expect(banners[0]).toContain(OTHER_SESSION);

    // **そして固定していない。** 送り先は素の識別子のまま。
    expect(outgoingModel()).toBe(HANDLE);
    expect(outgoingModel()).not.toBe(OTHER_SESSION);
    expect(outgoingMeaning()).toContain(MEANING_LATEST);

    await unmount(view, { outro: false });
  });

  it("接頭辞が同じだけの別キャラクターも、固定しない", async () => {
    // **`aoi-2@9` は `aoi` のセッションではない。**
    // `@` を見ない実装はここで落ちる（綴りが全く違う `midori@7` では落ちない）。
    scripts = [completed(PREFIX_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const banners = bannerErrors();
    expect(banners.length).toBe(1);
    expect(banners[0]).toContain(PREFIX_SESSION);
    expect(outgoingModel()).toBe(HANDLE);

    await unmount(view, { outro: false });
  });

  it("警告が出た回でも、届いた本文は画面に残る", async () => {
    // **固定しないことと、本文を捨てることは別の判断。**
    scripts = [completed(OTHER_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const turns = assistantTurns();
    expect(turns.length).toBe(1);
    expect(turns[0]?.textContent).toContain(FIRST_REPLY);

    await unmount(view, { outro: false });
  });
});

describe("その回は保存されたか（isPersisted）", () => {
  it("保存されていない回**だけ**に注記が付く", async () => {
    // **1回目は `[DONE]` まで届く。2回目は届かない。**
    // 「常に付く」「常に付かない」の**どちらの変異も**、この1本で落ちる。
    scripts = [
      completed(OWN_SESSION, FIRST_REPLY),
      [delta(OWN_SESSION, SECOND_REPLY)],
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    await submit(SECOND_DRAFT);

    const turns = assistantTurns();
    expect(turns.length).toBe(2);
    const first = turns[0] as HTMLElement;
    const second = turns[1] as HTMLElement;

    // **どちらの行に出たか**まで見る。画面全体を見ると、
    // 片方に出ているだけで両方の主張が緑になる。
    expect(first.textContent).toContain(FIRST_REPLY);
    expect(notesOf(first).join("\n")).not.toContain(NOT_PERSISTED);
    expect(errorsOf(first)).toEqual([]);

    expect(second.textContent).toContain(SECOND_REPLY);
    expect(notesOf(second).join("\n")).toContain(NOT_PERSISTED);
    // **理由も同じ行に出る。**
    expect(errorsOf(second).join("\n")).toContain(TRUNCATED_NOTICE);

    await unmount(view, { outro: false });
  });

  it("保存されていない回でも、届いていた本文は消さない", async () => {
    scripts = [[delta(OWN_SESSION, FIRST_REPLY)]];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const turns = assistantTurns();
    const only = turns[0] as HTMLElement;
    expect(only.querySelector("p.body")?.textContent).toContain(FIRST_REPLY);
    expect(notesOf(only).join("\n")).toContain(NOT_PERSISTED);

    await unmount(view, { outro: false });
  });
});

/**
 * **空の応答** —— **`ParseOutcome::Empty` へ「寄せた」。一致はしていない**（作業単位 B-8）。
 *
 * 画面の条件は `turn.raw.trim() === ""`。**Rust の受理集合と同じではない**ので、
 * **残差を両向きとも、この describe の中で観測する。**
 * 列挙は `ChatView.svelte` の doc にあり、**その doc が名指ししているのがここ。**
 *
 * **⚠ 見えない文字は必ず `\u` の綴りで書くこと。**
 * 生の字で置くと、差分でも画面でも消えて、何を測っているのか誰にも分からなくなる
 * （このファイルを書くときに実際に1度そうなった）。
 */
describe("空の応答（`trim()` へ寄せた。一致はしていない）", () => {
  /**
   * その生出力が `[DONE]` まで届いた回として画面に出たときの、相手の行を調べる。
   *
   * **1件ずつ mount して unmount する。** 同じ画面に積むと、
   * 「どの行に出たか」を見るのに添字が要り、取り違えの余地が増える。
   */
  async function withCompletedTurn(
    raw: string,
    inspect: (turn: HTMLElement) => void,
  ): Promise<void> {
    scripts = [[delta(OWN_SESSION, raw), DONE_EVENT]];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await submit(FIRST_DRAFT);

    const turn = assistantTurns()[0];
    expect(turn, JSON.stringify(raw)).toBeDefined();
    inspect(turn as HTMLElement);

    await unmount(view, { outro: false });
  }

  it("1文字も来ないまま `[DONE]` に至った回は「応答が空でした。」", async () => {
    // **本文のチャンクが1件も無い形**（`role` だけの先頭チャンク）。
    scripts = [[roleOnly(OWN_SESSION), DONE_EVENT]];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    expect(notesOf(only).join("\n")).toContain(EMPTY_RESPONSE);
    // **保存はされている**ので、そちらの注記は付かない。
    expect(notesOf(only).join("\n")).not.toContain(NOT_PERSISTED);

    await unmount(view, { outro: false });
  });

  it("空白だけの応答も「応答が空でした。」になる", async () => {
    // **寄せた本体。** どれも画面には何も見えないので、
    // 言わなければ「見出しだけが並んで無言に見える」——この注記の存在理由そのもの。
    const blanks: [label: string, raw: string][] = [
      ["U+0020 半角スペース", " "],
      ["U+000A 改行", "\n"],
      ["U+3000 全角空白", "\u3000"],
      ["U+0009 タブ", "\t"],
      ["U+00A0 NBSP", "\u00A0"],
      ["混在", " \u3000\n\t"],
    ];

    for (const [label, raw] of blanks) {
      await withCompletedTurn(raw, (turn) => {
        expect(notesOf(turn).join("\n"), label).toContain(EMPTY_RESPONSE);
        // **本文の枠ごと出さない。** 見えない1文字の段落を残すと、
        // 注記と本文が並んで、何が起きたのかがかえって読み取れない。
        expect(turn.querySelector("p.body"), label).toBeNull();
      });
    }
  });

  it("**対照** —— 見える文字が1つでもあれば「空でした」にならない", async () => {
    // **これが無いと「常に空とみなす」実装が上を満たして緑になる。**
    await withCompletedTurn(FIRST_REPLY, (turn) => {
      expect(notesOf(turn).join("\n")).not.toContain(EMPTY_RESPONSE);
      expect(turn.querySelector("p.body")?.textContent).toContain(FIRST_REPLY);
    });

    // **空白に囲まれていても、見える文字があれば本文。**
    await withCompletedTurn(` ${FIRST_REPLY}\n`, (turn) => {
      expect(notesOf(turn).join("\n")).not.toContain(EMPTY_RESPONSE);
      expect(turn.querySelector("p.body")?.textContent).toContain(FIRST_REPLY);
    });
  });

  /**
   * **残差①（画面のほうが狭い）** —— Rust は `Empty`、画面は本文として出す。
   *
   * `ChatView.svelte` の doc が「空の `「」` は拾えない」と書いている件。
   * **`trim()` は括弧を落とさない**ので、`split` が台詞の区間として着色する
   * （その着色の是非は `dialogue.ts` の doc が持っている。ここは注記の話）。
   *
   * **2通りある。** `"「」"` は Rust 側でも中身がゼロだが、
   * **`"「 」"` は `join_dialogues` の `trim_blank` が効いて初めて空になる別の枝**
   * （`parse::tests::empty_quotes_produce_no_dialogue` が固定している）。
   * **doc の列挙と見張り番を揃えるため、両方を通す。**
   */
  it("**残差** —— 空の `「」` だけの応答は「空でした」にならない（Rust は `Empty`）", async () => {
    for (const raw of ["「」", "「 」"]) {
      await withCompletedTurn(raw, (turn) => {
        expect(notesOf(turn).join("\n"), raw).not.toContain(EMPTY_RESPONSE);
        expect(turn.querySelector("p.body")?.textContent, raw).toBe(raw);
      });
    }
  });

  /**
   * **残差②（画面のほうが狭い）** —— `trim()` が落とさない行区切りが1つある。
   *
   * **U+0085（NEL）は Rust の `LINE_BREAKS` に入っているが、JS の `trim()` は落とさない**
   * **BMP 全域（サロゲート域を除く）を走査して測った**（node v26）。
   * **Rust の和集合 11 点のうち、JS の `trim()` が落とさないのはこの1点だけ。**
   */
  it("**残差** —— U+0085 だけの応答は「空でした」にならない（Rust は `Empty`）", async () => {
    await withCompletedTurn("\u0085", (turn) => {
      expect(notesOf(turn).join("\n")).not.toContain(EMPTY_RESPONSE);
      expect(turn.querySelector("p.body")?.textContent).toBe("\u0085");
    });
  });

  /**
   * **残差③（画面のほうが広い / 向きが逆）** —— Rust は `NarrationOnly`、画面は「空でした」。
   *
   * **U+2000 は Rust の `TRIMMABLE` にも `LINE_BREAKS` にも無い**ので、
   * サーバはこれを地の文として保存する。**`trim()` は落とす。**
   * **ずれが両向きにあることを、この1本が観測している**
   * （②だけだと「画面のほうが狭い」としか読めない）。
   */
  it("**残差（逆向き）** —— U+2000 だけの応答は「空でした」になる（Rust は `NarrationOnly`）", async () => {
    await withCompletedTurn("\u2000", (turn) => {
      expect(notesOf(turn).join("\n")).toContain(EMPTY_RESPONSE);
      expect(turn.querySelector("p.body")).toBeNull();
    });
  });

  /**
   * **⚠ 受信中の側の `turn.raw === ""` には `trim()` を入れない。**
   *
   * あちらの問いは「**1文字でも届いたか**」で、`ParseOutcome` とは関係が無い。
   * **`trim()` を入れると、空白しか届いていない回が「応答を待っています…」に戻り、
   * 「届き始めている」という事実が画面から消える**——
   * しかも `elapsed >= 5` の但し書き（「最初の一文字まで数十秒かかることがあります」）が
   * 一緒に出るので、**利用者は「まだ何も来ていない」と読む。**
   *
   * **B-8 の採否は `done` 側だけの話だった。**
   * ここが無いと、**寄せた側に合わせてあちらにも `trim()` を足す変異が全緑を通る**
   * （実際に当てて 99 全緑だったので足した）。
   */
  it("**受信中の側は寄せない** —— 空白しか届いていなくても「受信中…」", async () => {
    // **終わらない回。** 空白を1つ届けたところで止める。
    scripts = [[delta(OWN_SESSION, " "), null]];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    typeAndSubmit(FIRST_DRAFT);

    const notes = await vi.waitFor(() => {
      const turn = assistantTurns()[0];
      expect(turn).toBeDefined();
      const found = notesOf(turn as HTMLElement).join("\n");
      expect(found).toContain("秒");
      return found;
    });

    // **届いている。** 待っている、ではない。
    expect(notes).toContain("受信中");
    expect(notes).not.toContain("応答を待っています");
    // 結末はまだ無いので、`done` 側の注記はどれも出ない。
    expect(notes).not.toContain(EMPTY_RESPONSE);
    expect(notes).not.toContain(NOT_PERSISTED);

    await unmount(view, { outro: false });
  });

  /**
   * **⚠ 但し書きの側にも `trim()` を入れない**（`ChatView.svelte` の `elapsed >= 5` の行）。
   *
   * **上の1本では半分しか守れない。** 受信中の `turn.raw === ""` は**2箇所**にあり、
   * **「受信中…」と「応答を待っています…」を選ぶ側**と、
   * **但し書き（「最初の一文字まで数十秒かかることがあります」）を出す側**に割れている。
   * **前者を塞いだ時点で「守った」と書いたので、後者が見えなくなっていた**
   * （レビューが実測: 但し書き側に `trim()` を足す変異が **100 全緑**）。
   *
   * **実害**: 空白が届き始めているのに
   * **「受信中… 10 秒（モデルの読み込みや…最初の一文字まで数十秒かかることがあります）」**
   * と出る。**利用者は「まだ何も来ていない」と読む。**
   *
   * # なぜ今まで観測できなかったか
   *
   * **`elapsed >= 5` に到達するテストが1本も無かった。**
   * 値の同値化ではなく**状態**——`elapsed` は実時間で進むので、
   * **時計を握らないとその状態に入れない。**
   */
  describe("5秒を過ぎたときの但し書き", () => {
    /**
     * **終わらない回**を作り、時計だけを進める。
     *
     * `advanceMs` は**偽の時計**で進める（実時間では待たない）。
     * `Date.now()` も一緒に偽になるので `elapsed` が動く。
     */
    async function withInFlightTurn(
      chunks: Script,
      settled: string,
      steps: (advance: (ms: number) => Promise<string>) => Promise<void>,
    ): Promise<void> {
      vi.useFakeTimers();
      try {
        scripts = [chunks];
        const view = mount(ChatView, { target });
        await waitForCharacters();
        typeAndSubmit(FIRST_DRAFT);

        // **その回が「送信中」の形に落ち着くまで。**
        await vi.waitFor(() => {
          const turn = assistantTurns()[0];
          expect(turn).toBeDefined();
          expect(notesOf(turn as HTMLElement).join("\n")).toContain(settled);
        });

        const advance = async (ms: number): Promise<string> => {
          await vi.advanceTimersByTimeAsync(ms);
          flushSync();
          const turn = assistantTurns()[0];
          expect(turn).toBeDefined();
          return notesOf(turn as HTMLElement).join("\n");
        };

        await steps(advance);
        await unmount(view, { outro: false });
      } finally {
        vi.useRealTimers();
      }
    }

    it("1文字も届いていない回は、5秒を過ぎたら但し書きが出る", async () => {
      // **これが「出る側」。** 無いと「但し書きを一切出さない」実装が下を満たす。
      await withInFlightTurn(
        [roleOnly(OWN_SESSION), null],
        "応答を待っています",
        async (advance) => {
          // 4秒では**まだ出ない**（閾値そのものを押さえる）。
          const early = await advance(4000);
          expect(early).toContain("応答を待っています");
          expect(early).toContain("4 秒");
          expect(early).not.toContain(HINT_SLOW_FIRST_TOKEN);

          // 6秒で**出る**。
          const late = await advance(2000);
          expect(late).toContain("応答を待っています");
          expect(late).toContain("6 秒");
          expect(late).toContain(HINT_SLOW_FIRST_TOKEN);
        },
      );
    });

    it("**空白しか届いていなくても、但し書きは出ない**（届き始めている）", async () => {
      // **ここが指摘1の本体。** 但し書き側に `trim()` を足すと、
      // 空白1文字は「1文字も届いていない」に化け、この但し書きが出る。
      await withInFlightTurn(
        [delta(OWN_SESSION, " "), null],
        "受信中",
        async (advance) => {
          const late = await advance(6000);
          expect(late).toContain("受信中");
          expect(late).toContain("6 秒");
          expect(late).not.toContain("応答を待っています");
          // **同じ 6 秒でも、上の回とはここが逆になる。**
          expect(late).not.toContain(HINT_SLOW_FIRST_TOKEN);
        },
      );
    });
  });
});

/**
 * **結末が `done` でない回は、門の内側に入らない**（`ChatView.svelte` の
 * `turn.outcome !== null && turn.outcome.kind === "done"`）。
 *
 * # なぜ要るのか（レビューが実測: 門を落としても **100 全緑**）
 *
 * | 入力 | 門があるとき | 門を落とすと |
 * |---|---|---|
 * | 0文字で打ち切られた回 | 「保存されていません」だけ | **「応答が空でした。」が増える** |
 * | 途中で切れた `「やあ」微笑んだ` | 生のまま | **台詞として着色される** |
 *
 * **前者は、同じ行の `p.error`（「応答が完結する前に接続が終わりました」）と正面から矛盾する。**
 * **「空だった」と「完結する前に切れた」は別の事実**で、利用者は原因を取り違える。
 * **後者は、途中で切れた出力を「分解できた」と見せる**——
 * `「やあ」` の後ろがまだ来ていないだけなのに、確定した台詞として色が付く。
 *
 * **⚠ 門は正しい。要るのは見張り番だけ**（作業単位 B-8 のレビュー指摘2）。
 */
describe("`done` でない回は、分解も「空でした」もしない", () => {
  it("0文字で打ち切られた回に「応答が空でした。」は出ない", async () => {
    const view = mount(ChatView, { target });
    // 本文のチャンクが1件も無いまま、`[DONE]` も来ずに終わる。
    scripts = [[roleOnly(OWN_SESSION)]];
    await waitForCharacters();
    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    const notes = notesOf(only).join("\n");
    // **起きた事実はこちら。**
    expect(notes).toContain(NOT_PERSISTED);
    expect(errorsOf(only).join("\n")).toContain(TRUNCATED_NOTICE);
    // **こちらは起きていない。** 両方出ると、同じ行が2つの違う原因を名乗る。
    expect(notes).not.toContain(EMPTY_RESPONSE);

    await unmount(view, { outro: false });
  });

  it("途中で切れた回は、台詞として着色しない", async () => {
    const view = mount(ChatView, { target });
    scripts = [[delta(OWN_SESSION, PARTIAL_REPLY)]];
    await waitForCharacters();
    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    // **生のまま1つの段落。** 区間に割らない。
    const body = only.querySelector("p.body");
    expect(body).not.toBeNull();
    expect((body as HTMLElement).textContent).toBe(PARTIAL_REPLY);
    expect(only.querySelector("span.dialogue")).toBeNull();
    expect(only.querySelector("span.narration")).toBeNull();
    expect(notesOf(only).join("\n")).toContain(NOT_PERSISTED);

    await unmount(view, { outro: false });
  });

  it("**対照** —— 同じ本文でも `[DONE]` まで届けば台詞として着色する", async () => {
    // **これが無いと「一度も着色しない」実装が上を満たして緑になる。**
    const view = mount(ChatView, { target });
    scripts = [[delta(OWN_SESSION, PARTIAL_REPLY), DONE_EVENT]];
    await waitForCharacters();
    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    const dialogue = only.querySelector("span.dialogue");
    expect(dialogue).not.toBeNull();
    expect((dialogue as HTMLElement).textContent).toBe("「やあ」");
    expect(notesOf(only).join("\n")).not.toContain(NOT_PERSISTED);

    await unmount(view, { outro: false });
  });
});
