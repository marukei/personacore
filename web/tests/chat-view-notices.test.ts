// @vitest-environment jsdom
/**
 * **画面に残る文と、中身の無い文**（作業単位 B-14 / B-8 の作業者が見つけた欠陥2件）。
 *
 * # ここで見るもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | `sessionWarning` が**どの回に出るか** | `chat-view-session.test.ts` |
 * | **`sessionWarning` が、いつ消えるか** | **ここ** |
 * | `rejected` に至る道と、3つの欄 | `chat-outcomes.test.ts` |
 * | **`rejected` の文言が空白だけだったとき** | **ここ** |
 *
 * # なぜ、いま要るのか
 *
 * **どちらも「そのターンの結末」ではなく「画面の状態」の側の欠陥。**
 *
 * * **警告は「いまの状態」**（`ChatView.svelte` の `sessionWarning` の doc）。
 *   **後の回で正しく固定できたら、その文はもう偽になる。**
 *   赤い文が居座ったまま実際には固定されている、は**画面が嘘をついている**状態。
 * * **中身が空の `<p class="error">`** は、「エラーがあった」という枠だけを出して
 *   **何も言わない**——いちばん悪い見え方。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ApiErrorDetail } from "../src/lib/api";
import ChatView from "../src/lib/ChatView.svelte";
import { outcomeNotice } from "../src/lib/chat";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が最初に選ぶキャラクター。**`handle` と `display_name` を別の値にしてある。** */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/**
 * **選び直す先のキャラクター。** 綴りも表示名も上とは全く違う。
 *
 * `selectCharacter` が警告を落とすことを見るのに、**2件目が要る。**
 */
const OTHER_HANDLE = "kaede";
const OTHER_DISPLAY_NAME = "楓";

/** **このキャラクターのセッション。** 番号は件数（1・2・3）とも別の数。 */
const OWN_SESSION = `${HANDLE}@41`;

/** **別のキャラクターのセッション。** 綴りが全く違う。 */
const STRAY_SESSION = "midori@7";

/**
 * **接頭辞だけが同じ、別のキャラクターのセッション**（`aoi-2` は別キャラクター / ADR-0017）。
 *
 * **2度目の異常**に使う。`STRAY_SESSION` と綴りが違うので、
 * **どちらの文面が出ているか**が区別できる。
 */
const PREFIX_SESSION = "aoi-2@9";

/** 送る本文。**3回ぶんとも別の文字列。** */
const FIRST_DRAFT = "ただいま";
const SECOND_DRAFT = "おなかすいた";
const THIRD_DRAFT = "何か手伝う";

/** 応答の本文。**送った本文のどれとも別の文字列。** */
const FIRST_REPLY = "おかえりなさい";
const SECOND_REPLY = "何か作ろうか";

/** 警告の文面の目印（`ChatView.svelte`）。**何を拒んだか**だけを言う。 */
const WARNING_TAIL = "そこへは切り替えませんでした。";

/**
 * **警告が名乗ってはいけない語**（B-14 のレビュー / 是正1）。
 *
 * **「いま固定しているか」を言う権威は送り先の表示（`p.wire`）の側にある。**
 * 警告が状態を名乗ると**権威が2つになり、実際に食い違う** ——
 * **正しく着地した後に別キャラを名乗られた回**で、
 * 「セッションを固定しませんでした」と
 * 「送り先: `aoi@41`（このセッションの続き）」が**同じ画面に同時に出る。**
 */
const PINNING_WORD = "固定";

/** 保存されていない回に付く注記。 */
const NOT_PERSISTED = "サーバに保存されていません";

/** 送り先の意味（`outgoingMeaning`）。 */
const MEANING_LATEST = "最新の会話の続き";
const MEANING_PINNED = "このセッションの続き";

/**
 * **断られたときの状態番号。4つとも別の数**（`chat-outcomes.test.ts` の 503 / 502 とも別）。
 *
 * 同じ数だと「その回の応答から取っているか」が測れない。
 */
const BLANK_MESSAGE_STATUS = 429;
const SPOKEN_MESSAGE_STATUS = 418;
const SCREEN_BLANK_STATUS = 507;
const SCREEN_SPOKEN_STATUS = 405;

/** エラー本文の3つの欄。**全部別の文字列。** */
const ERROR_MESSAGE = "キャラクターが見つかりません";
const ERROR_TYPE = "invalid_request_error";
const ERROR_CODE = "unknown_character";

/**
 * **「読めません」と名乗ってよいのは `detail` が無い回だけ**（B-14 のレビュー）。
 *
 * 空白だけの回は**本文が読めている**ので、この綴りが出たらそれは
 * **起きていないことの断言**になる。**探す側と出す側で綴りを揃える。**
 */
const UNREADABLE_PHRASE = "読めません";

/** 1回の送信で流すもの（`null` は「返らない読み取り」）。 */
type Script = (string | null)[];

/** これから流す台本（送信のたびに先頭から1つ取る）。 */
let scripts: Script[];
/** **断り**。`null` でなければ、送信は 200 の前に断られる。 */
let rejection: { status: number; body: string } | null;
let target: HTMLElement;

/** 1件ぶんの SSE イベント。 */
function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

/** 本文の増分1件（**`model` 付き**）。 */
function delta(model: string, text: string): string {
  return event({ model, choices: [{ delta: { content: text } }] });
}

/**
 * 本文の増分1件（**`model` を載せない**）。
 *
 * **`onModel` が一度も呼ばれない回**を作るのに使う。
 */
function deltaWithoutModel(text: string): string {
  return event({ choices: [{ delta: { content: text } }] });
}

/** 終端の番兵。 */
const DONE_EVENT = "data: [DONE]\n\n";

/** `[DONE]` まで届く、ふつうの1回。 */
function completed(model: string, text: string): Script {
  return [delta(model, text), DONE_EVENT];
}

/** エラー本文（`error.rs` の `ErrorBody`）。**`message` は呼び出し側が決める。** */
function errorBody(message: string): string {
  return JSON.stringify({
    error: { message, type: ERROR_TYPE, code: ERROR_CODE },
  });
}

/**
 * `fetch` の代わり。**本物の `Response` を組み立てない**
 * （`chat-view-session.test.ts` と同じ理由——jsdom 側の実装差を測定に混ぜない）。
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
            id: 3,
            handle: HANDLE,
            display_name: DISPLAY_NAME,
            base_persona: "静かな声で話す。",
            output_format_instruction: null,
            stat_scale: 5,
            created_at: "2026-08-27T00:00:00Z",
          },
          {
            id: 11,
            handle: OTHER_HANDLE,
            display_name: OTHER_DISPLAY_NAME,
            base_persona: "早口で話す。",
            output_format_instruction: null,
            stat_scale: 7,
            created_at: "2026-08-27T01:00:00Z",
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

    if (rejection !== null) {
      const said = rejection;
      return Promise.resolve({
        ok: false,
        status: said.status,
        text: () => Promise.resolve(said.body),
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

/** キャラクターが届くまで待つ（届かないと送信の口が開かない）。 */
async function waitForCharacters(): Promise<HTMLSelectElement> {
  const select = await vi.waitFor(() => {
    const found = target.querySelector("select");
    expect(found).not.toBeNull();
    return found as HTMLSelectElement;
  });
  expect(select.value).toBe(HANDLE);
  return select;
}

/** 本文を入れて送信し、**その回の結末が画面に出るまで**待つ。 */
async function submit(text: string): Promise<void> {
  const before = assistantTurns().length;

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

/** **画面のどこかにある**エラー文（枠だけの空文を探すのに使う）。 */
function allErrorTexts(): string[] {
  return [...target.querySelectorAll("p.error")].map(
    (error) => error.textContent ?? "",
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

/** 見出しがその綴りのボタン。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

beforeEach(() => {
  scripts = [];
  rejection = null;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

/**
 * **警告は「何を拒んだか」だけを言う**（作業単位 B-14 ＋ そのレビュー / 是正1）。
 *
 * # 権威を2つ置かない
 *
 * **「いま何を送ろうとしているか」を言うのは送り先の表示（`p.wire`）。**
 * **警告はそこへ踏み込まない。** 踏み込むと、両者が食い違う回が実在する ——
 * **固定した後に別キャラを名乗られた回**（下の「固定した後に…」の1本）。
 *
 * # それでも、居座らせない
 *
 * **異常が続いていないのに警告だけが残るのは、それ自体が古い情報。**
 * **正しく着地したら消す。**
 *
 * **⚠ 消える枝は3つある**（`ChatView.svelte`）。**3つとも、ここで観測する。**
 * 1つでも欠けると、その枝を落とす変異が緑を通る。
 */
describe("セッション警告は、何を拒んだかだけを言う", () => {
  it("異常のあと正しく着地したら、警告は消える", async () => {
    scripts = [
      completed(STRAY_SESSION, FIRST_REPLY),
      completed(OWN_SESSION, SECOND_REPLY),
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    // **1回目は異常。** 警告が出て、固定していない。
    const first = bannerErrors();
    expect(first.length).toBe(1);
    expect(first[0]).toContain(STRAY_SESSION);
    expect(outgoingModel()).toBe(HANDLE);

    await submit(SECOND_DRAFT);

    // **2回目で正しく着地した。** 警告はもう偽なので消える。
    expect(bannerErrors()).toEqual([]);
    // **そして固定されている**（消えただけ、ではない）。
    expect(outgoingModel()).toBe(OWN_SESSION);
    expect(outgoingMeaning()).toContain(MEANING_PINNED);

    // **本文はどちらも残っている。**
    const turns = assistantTurns();
    expect(turns.length).toBe(2);
    expect(turns[0]?.textContent).toContain(FIRST_REPLY);
    expect(turns[1]?.textContent).toContain(SECOND_REPLY);

    await unmount(view, { outro: false });
  });

  it("異常のあと、また異常なら、新しい文面に置き換わる", async () => {
    // **決定: 積み上げない。1本だけ、いちばん新しい着地について言う。**
    // 2本並べると「いま何が起きているのか」がかえって読み取れない。
    scripts = [
      completed(STRAY_SESSION, FIRST_REPLY),
      completed(PREFIX_SESSION, SECOND_REPLY),
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    expect(bannerErrors()[0]).toContain(STRAY_SESSION);

    await submit(SECOND_DRAFT);

    const banners = bannerErrors();
    expect(banners.length).toBe(1);
    // **新しい側の綴りが出て、古い側は消えている。**
    expect(banners[0]).toContain(PREFIX_SESSION);
    expect(banners[0]).not.toContain(STRAY_SESSION);
    expect(banners[0]).toContain(WARNING_TAIL);

    // **固定は、どちらの回でもしていない。**
    expect(outgoingModel()).toBe(HANDLE);
    expect(outgoingMeaning()).toContain(MEANING_LATEST);

    await unmount(view, { outro: false });
  });

  it("**固定した後に別キャラを名乗られても**、警告は送り先と食い違わない", async () => {
    // **⚠ 逆の順序。** 上の1本（異常 → 正常）の鏡像で、
    // **こちらは固定が先、異常が後。**
    //
    // **同じ回に、警告の文面と送り先の表示の両方を見る。**
    // 片方だけだと、いま起きている食い違いが観測できない ——
    // 「固定しませんでした」と「送り先: aoi@41（このセッションの続き）」が
    // **同時に出ていても、どちらの側からも気づけない。**
    scripts = [
      completed(OWN_SESSION, FIRST_REPLY),
      completed(STRAY_SESSION, SECOND_REPLY),
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    // **先に正しく着地している。**
    expect(bannerErrors()).toEqual([]);
    expect(outgoingModel()).toBe(OWN_SESSION);

    await submit(SECOND_DRAFT);

    // **固定はそのまま**（別キャラを名乗る回は固定を奪えない）。
    expect(outgoingModel()).toBe(OWN_SESSION);
    expect(outgoingMeaning()).toContain(MEANING_PINNED);

    // **警告は出る。** 何を拒んだかは言う。
    const banners = bannerErrors();
    expect(banners.length).toBe(1);
    expect(banners[0]).toContain(STRAY_SESSION);
    expect(banners[0]).toContain(WARNING_TAIL);

    // **⚠ ただし「固定しているか」は言わない。**
    // ここで状態を名乗ると、すぐ上で見た送り先の表示と正面から食い違う。
    expect(banners[0]).not.toContain(PINNING_WORD);
    // **状態の語彙は送り先の表示の持ち物。**
    // **語を1つ禁じるだけでは足りない**——「固定」を避けて
    // 「いまは最新の会話の続きを送ります」と書いても、同じ食い違いが起きる。
    expect(banners[0]).not.toContain(MEANING_PINNED);
    expect(banners[0]).not.toContain(MEANING_LATEST);

    await unmount(view, { outro: false });
  });

  it("`model` を名乗らない回では、警告は残る", async () => {
    // **消してよいのは「正しく着地した」と分かった回だけ。**
    // `onModel` が一度も呼ばれない回は、固定できたかどうかを何も言っていない
    // （200 の前に断られた回も同じ形になる）。
    scripts = [
      completed(STRAY_SESSION, FIRST_REPLY),
      [deltaWithoutModel(SECOND_REPLY), DONE_EVENT],
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    expect(bannerErrors().length).toBe(1);

    await submit(SECOND_DRAFT);

    // **2回目は `[DONE]` まで届いている**（保存されている回）。
    const turns = assistantTurns();
    expect(turns.length).toBe(2);
    const second = turns[1] as HTMLElement;
    expect(second.textContent).toContain(SECOND_REPLY);
    expect(notesOf(second).join("\n")).not.toContain(NOT_PERSISTED);

    // **それでも警告は残る。** 着地したセッションを誰も名乗っていない。
    const banners = bannerErrors();
    expect(banners.length).toBe(1);
    expect(banners[0]).toContain(STRAY_SESSION);
    // **固定もされていない。**
    expect(outgoingModel()).toBe(HANDLE);

    await unmount(view, { outro: false });
  });

  it("キャラクターを選び直したら、警告は消える", async () => {
    scripts = [completed(STRAY_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    const select = await waitForCharacters();

    await submit(FIRST_DRAFT);
    expect(bannerErrors().length).toBe(1);

    select.value = OTHER_HANDLE;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();

    // **別のキャラクターの話になった。** 前の相手についての警告はもう意味を持たない。
    expect(bannerErrors()).toEqual([]);
    expect(outgoingModel()).toBe(OTHER_HANDLE);
    expect(assistantTurns()).toEqual([]);

    await unmount(view, { outro: false });
  });

  it("新しい会話を始めたら、警告は消える", async () => {
    scripts = [completed(STRAY_SESSION, FIRST_REPLY)];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    expect(bannerErrors().length).toBe(1);

    buttonLabelled("新しい会話を始める").click();
    flushSync();

    expect(bannerErrors()).toEqual([]);
    expect(outgoingModel()).toBe(`${HANDLE}@new`);
    expect(assistantTurns()).toEqual([]);

    await unmount(view, { outro: false });
  });

  it("**対照** —— 3回とも正しく着地したら、警告は一度も出ない", async () => {
    // **これが無いと「そもそも警告を出さない」実装が上を全部満たす。**
    scripts = [
      completed(OWN_SESSION, FIRST_REPLY),
      completed(OWN_SESSION, SECOND_REPLY),
      completed(OWN_SESSION, FIRST_REPLY),
    ];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);
    expect(bannerErrors()).toEqual([]);
    await submit(SECOND_DRAFT);
    expect(bannerErrors()).toEqual([]);
    await submit(THIRD_DRAFT);
    expect(bannerErrors()).toEqual([]);

    expect(outgoingModel()).toBe(OWN_SESSION);

    await unmount(view, { outro: false });
  });
});

/**
 * **断られた回の文言は3つある**（作業単位 B-14 ＋ そのレビュー）。
 *
 * | 状態 | 何が起きたか | 出す文 |
 * |---|---|---|
 * | `detail === null` | **本文そのものが読めなかった** | 「本文を読めませんでした」 |
 * | `message` が空白だけ | **本文は読めた。理由が書かれていなかった** | 別の文 |
 * | `message` に見える文字がある | 理由が書かれていた | **その文をそのまま** |
 *
 * **`??` は空文字列を採ってしまう**ので、素のままだと
 * **中身の空の `<p class="error">`** が出る（`{#if notice !== null}` は空文字列を通す）。
 * **判定は `trim()`**——同じ画面の `turn.raw.trim() === ""` と同じ寄せ方。
 *
 * **⚠ ここで押さえるのは「3つが互いに違うこと」まで。**
 * **2つを同じ文言にする変異**と、**3つを1つに潰す変異**の**両方**が赤になる形にしてある
 * （片側だけだと決め打ちが通り抜ける）。
 */
describe("断られた回の文言", () => {
  it("空白だけなら「理由が書かれていなかった」と言う（「読めなかった」とは言わない）", () => {
    // **見えない文字は `\u` の綴りで書く。** 生の字で置くと、
    // 差分でも画面でも消えて、何を測っているのか誰にも分からなくなる。
    const blanks: [label: string, message: string][] = [
      ["空文字列", ""],
      ["U+0020 半角スペース", " "],
      ["U+000A 改行", "\n"],
      ["U+0009 タブ", "\t"],
      ["U+3000 全角空白", "\u3000"],
      ["U+00A0 NBSP", "\u00A0"],
      ["混在", " \u3000\n\t"],
    ];

    for (const [label, message] of blanks) {
      const notice = outcomeNotice({
        kind: "rejected",
        status: BLANK_MESSAGE_STATUS,
        detail: { message, kind: ERROR_TYPE, code: ERROR_CODE },
      });

      expect(notice, label).not.toBeNull();
      // **枠だけ出して何も言わない、にならない。**
      expect((notice ?? "").trim(), label).not.toBe("");
      // **その回の状態番号が出る**（別の数へ決め打つ変異はここで落ちる）。
      expect(notice, label).toContain(String(BLANK_MESSAGE_STATUS));
      expect(notice, label).not.toContain(String(SPOKEN_MESSAGE_STATUS));
      // **本文は読めている。** ここで「読めませんでした」と出すのは事実に反する。
      expect(notice, label).not.toContain(UNREADABLE_PHRASE);
    }
  });

  it("**対照** —— 見える文字があれば、サーバの文言をそのまま出す", () => {
    // **これが無いと「常に既定へ落とす」実装が上を満たして緑になる。**
    const notice = outcomeNotice({
      kind: "rejected",
      status: SPOKEN_MESSAGE_STATUS,
      detail: { message: ERROR_MESSAGE, kind: ERROR_TYPE, code: ERROR_CODE },
    });

    expect(notice).toBe(ERROR_MESSAGE);
    // **こちらの言い換えを混ぜない**（サーバが言ったこと以上を推測しない）。
    expect(notice).not.toContain(String(SPOKEN_MESSAGE_STATUS));
  });

  it("**文面は書き換えない** —— `trim()` は判定にだけ使う", () => {
    // **空白に囲まれていても、見える文字があればサーバの文言。**
    // **切り詰めた文字列を返す変異は、ここで落ちる。**
    const padded = ` ${ERROR_MESSAGE}\n`;
    const notice = outcomeNotice({
      kind: "rejected",
      status: SPOKEN_MESSAGE_STATUS,
      detail: { message: padded, kind: ERROR_TYPE, code: ERROR_CODE },
    });

    expect(notice).toBe(padded);
  });

  it("**本文が読めなかった回だけ**が「読めません」と名乗る", () => {
    // **本文が読めなかった回**（`chat-outcomes.test.ts` が至る道を持っている）。
    const notice = outcomeNotice({
      kind: "rejected",
      status: BLANK_MESSAGE_STATUS,
      detail: null,
    });

    expect(notice).toContain(String(BLANK_MESSAGE_STATUS));
    expect((notice ?? "").trim()).not.toBe("");
    // **この回だけが名乗ってよい。** 上の1本と向きが逆になっている。
    expect(notice).toContain(UNREADABLE_PHRASE);
  });

  it("3つの枝は、**同じ状態番号でも**互いに違う文になる", () => {
    // **⚠ 状態番号を揃えて並べる。** 番号が違うと
    // 「番号が違うから文も違う」で通ってしまい、
    // **2つの枝を同じ文言にする変異**が見えない。
    const noticeOf = (detail: ApiErrorDetail | null): string =>
      outcomeNotice({
        kind: "rejected",
        status: BLANK_MESSAGE_STATUS,
        detail,
      }) ?? "";

    const unreadable = noticeOf(null);
    const silent = noticeOf({
      message: " \u3000",
      kind: ERROR_TYPE,
      code: ERROR_CODE,
    });
    const spoken = noticeOf({
      message: ERROR_MESSAGE,
      kind: ERROR_TYPE,
      code: ERROR_CODE,
    });

    // **どれも枠だけにならない。**
    for (const text of [unreadable, silent, spoken]) {
      expect(text.trim()).not.toBe("");
    }
    // **⚠ この行が効くのは「状態番号を揃えている限りにおいて」。**
    //
    // **番号を散らすと、2つの枝を同じ文言にしても文字列は別物になり、この行は緑で通る。**
    // **実測**（「空白の枝も『本文を読めませんでした』を返す」変異を当てて）:
    //
    // | この検査の状態 | 落ちた場所 |
    // |---|---|
    // | 納品どおり（番号を揃える） | **この行**（`expected 2 to be 3`） |
    // | 番号を散らす | **下の帰属3行へ移る**（`silent` が「読めません」を名乗る） |
    // | 番号を散らす ＋ 帰属3行も外す | **この検査は緑になる** |
    //
    // **⚠ 3行目でも、その変異はこのファイルの別の2本が捕まえる**
    // （「空白だけなら…」と画面の1本。どちらも `UNREADABLE_PHRASE` を見ている）。
    // **測ったのはこの検査の中の話**であって、「全緑になる」ではない。
    expect(new Set([unreadable, silent, spoken]).size).toBe(3);

    // **⚠ この検査の中では、ここが最後の砦。** どれがどれかまで見る
    // （互いに違うだけでは、入れ替えも、番号を散らした同文言も通る）。
    expect(unreadable).toContain(UNREADABLE_PHRASE);
    expect(silent).not.toContain(UNREADABLE_PHRASE);
    expect(spoken).toBe(ERROR_MESSAGE);
  });
});

/**
 * **画面まで通す。** 文言を見るのと、その文言が画面のどこにどう出るかは別の主張
 * （`chat-outcomes.test.ts` の冒頭と同じ理由）。
 */
describe('空の `<p class="error">` を出さない', () => {
  it("空白だけの `message` で断られても、その行のエラー文は空にならない", async () => {
    rejection = { status: SCREEN_BLANK_STATUS, body: errorBody(" \u3000") };
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    const errors = errorsOf(only);
    expect(errors.length).toBe(1);
    expect(errors[0]).toContain(String(SCREEN_BLANK_STATUS));
    // **本文は読めている**ので、画面でも「読めませんでした」と言わない。
    expect(errors[0]).not.toContain(UNREADABLE_PHRASE);
    // **保存されていない回でもある。** 理由と、その事実は別の文。
    expect(notesOf(only).join("\n")).toContain(NOT_PERSISTED);

    // **画面のどこにも、中身の無いエラー文は無い。**
    for (const text of allErrorTexts()) {
      expect(text.trim()).not.toBe("");
    }

    await unmount(view, { outro: false });
  });

  it("**対照** —— サーバが何か言っていれば、その文がその行に出る", async () => {
    rejection = {
      status: SCREEN_SPOKEN_STATUS,
      body: errorBody(ERROR_MESSAGE),
    };
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await submit(FIRST_DRAFT);

    const only = assistantTurns()[0] as HTMLElement;
    expect(errorsOf(only)).toEqual([ERROR_MESSAGE]);
    // **上部の警告ではない**（`sessionWarning` と取り違えない）。
    expect(bannerErrors()).toEqual([]);

    await unmount(view, { outro: false });
  });
});
