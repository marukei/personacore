// @vitest-environment jsdom
/**
 * **一覧からの削除**（作業単位 D-7 / `src/lib/CharacterList.svelte`）。
 *
 * # 何を見るのか
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 失敗の種別ごとの文言と次の一手（純粋関数） | `character-delete.test.ts` |
 * | **どの行を押したら、どの識別子へ `DELETE` が飛ぶか** | **ここ** |
 * | **確認を挟むこと・成功後に一覧から消えること** | **ここ** |
 * | **拒否の案内が `fetch` から画面まで通ること** | **ここ** |
 *
 * **文言だけを純粋関数で見ても足りない**——「文言は見られているが、経路は
 * 通っていない」形になる（`B-8` の主題 / CLAUDE.md）。ここは
 * **本物の 409 の本文を `fetch` から返して、画面の要素まで通す。**
 *
 * # ⚠ **押す行を端に置かない**
 *
 * 削除は「選ぶ」操作である。**選ばれる対象が端にあると
 * 「常に先頭」「常に最終」の片方が通り抜ける**（D-2 のレビューで実測）。
 * 3件のうち**真ん中（MIO）**を押す本命を置き、
 * **識別子の決め打ち**（`deleteCharacter("mio")`）を殺すために
 * **別の行（NAR）を押す腕**も1本置いてある。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView } from "../src/lib/api";

/** 1件目（先頭）。 */
const AOI: CharacterView = {
  id: 1,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** 2件目。**真ん中に居ることに意味がある**（ファイル冒頭の ⚠）。 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 7,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:10:00Z",
};

/** 3件目（最終）。 */
const NAR: CharacterView = {
  id: 3,
  handle: "nar",
  display_name: "奈落",
  base_persona: "ゆっくり話す。",
  output_format_instruction: null,
  stat_scale: 3,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:20:00Z",
};

const ALL = [AOI, MIO, NAR];

interface Call {
  method: string;
  url: string;
}

/** サーバが返す失敗（`src/server/error.rs` の綴りそのまま）。 */
interface Refusal {
  status: number;
  code: string | null;
  message: string;
}

const HAS_SESSIONS: Refusal = {
  status: 409,
  code: "character_has_sessions",
  message: "会話のあるキャラクターは削除できません",
};

const NOT_FOUND: Refusal = {
  status: 404,
  code: "character_not_found",
  message: "指定されたキャラクターが存在しません",
};

const BROKEN: Refusal = {
  status: 500,
  code: null,
  message: "キャラクターを削除できませんでした",
};

let calls: Call[];
let target: HTMLElement;
/** サーバの側で消えた識別子。**一覧はここを引いた結果を返す。** */
let deleted: Set<string>;
/** 識別子ごとの拒否（`DELETE` がこれを返す）。 */
let refusals: Map<string, Refusal>;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    calls.push({ method, url });

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters" && method === "GET") {
      return respond({
        characters: ALL.filter(
          (character) => !deleted.has(character.handle),
        ),
        unreadable: [],
      });
    }
    if (url.endsWith("/stats")) {
      return respond({ character: "?", stat_scale: 5, stats: [] });
    }
    if (method === "DELETE") {
      const handle = url.slice("/admin/characters/".length);
      const refusal = refusals.get(handle);
      if (refusal !== undefined) {
        return respond(
          {
            error: {
              message: refusal.message,
              type: "invalid_request_error",
              ...(refusal.code === null ? {} : { code: refusal.code }),
            },
          },
          refusal.status,
        );
      }
      deleted.add(handle);
      // **成功は 204。本文は無い**（`admin.rs` の `delete_character`）。
      return Promise.resolve({
        ok: true,
        status: 204,
        text: () => Promise.resolve(""),
      } as unknown as Response);
    }
    return respond({ id: MIO.id, character: MIO });
  });
}

/** いま一覧に並んでいる識別子。 */
function handlesShown(): string[] {
  return [...target.querySelectorAll("tbody tr")].map(
    (row) => row.querySelector("code")?.textContent?.trim() ?? "",
  );
}

/** その行の「削除」ボタン。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function deleteButtonOf(character: CharacterView): HTMLButtonElement {
  const row = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === character.handle,
  );
  expect(row, `${character.handle} の行があること`).toBeDefined();
  const found = [...(row as HTMLElement).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "削除",
  );
  expect(found, `${character.handle} の行に「削除」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

/** 画面内の確認。**`confirm()` は使わない**（jsdom で測れない / 画面の作法）。 */
function confirmPanel(): HTMLElement {
  const found = target.querySelector("[data-confirm-delete]");
  expect(found, "確認が開いていること").not.toBeNull();
  return found as HTMLElement;
}

/** 確認の中のボタン。**全体から引かない**（一覧の「削除」に当たる）。 */
function panelButton(label: string): HTMLButtonElement {
  const found = [...confirmPanel().querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `確認に「${label}」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

function deleteCalls(): Call[] {
  return calls.filter((call) => call.method === "DELETE");
}

function listGets(): number {
  return calls.filter(
    (call) => call.url === "/admin/characters" && call.method === "GET",
  ).length;
}

async function waitForRows(): Promise<void> {
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(
      3,
    );
  });
}

/** 確認を開いて、押すところまで。 */
async function openAndConfirm(character: CharacterView): Promise<void> {
  await waitForRows();
  deleteButtonOf(character).click();
  flushSync();
  panelButton("削除する").click();
}

beforeEach(() => {
  calls = [];
  deleted = new Set();
  refusals = new Map();
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("取り消しのきかない操作なので、確認を挟む", () => {
  it("「削除」を押しただけでは、何も送らない", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();

    expect(deleteCalls()).toEqual([]);
    // 行も消えていない（画面の側で先に消してしまっていない）。
    expect(handlesShown()).toEqual([AOI.handle, MIO.handle, NAR.handle]);

    await unmount(view, { outro: false });
  });

  /**
   * **確認が、押した行のキャラクターを名乗っている。**
   *
   * ここが「常に先頭」「常に最終」の両向きを殺す本命
   * （MIO は真ん中なので、どちらの決め打ちでも外れる）。
   */
  it("確認は、押した行のキャラクターを名乗る", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();

    const panel = confirmPanel();
    expect(panel.dataset["confirmDelete"]).toBe(MIO.handle);
    expect(panel.textContent).toContain(MIO.display_name);
    // **両隣のどちらでもない。**
    expect(panel.dataset["confirmDelete"]).not.toBe(AOI.handle);
    expect(panel.dataset["confirmDelete"]).not.toBe(NAR.handle);
    expect(panel.textContent).not.toContain(AOI.display_name);
    expect(panel.textContent).not.toContain(NAR.display_name);

    await unmount(view, { outro: false });
  });

  /**
   * **確認が、実際に警告しているか**（レビューの是正 1）。
   *
   * # 「確認を挟む」と「確認が警告する」は別の主張
   *
   * 即送信の変異は赤くなる（構造は守られている）が、
   * **文面はレビューの実測で丸ごと無防備だった** ——
   * **「取り消せません」を消す／警告の段落ごと消す／
   * 「押してみるまで分かりません」を「会話が無いので確実に消せます」に置換する／
   * 「削除します」を「選びました」に置換する**、いずれも全緑。
   *
   * **3つ目がいちばん重い。** ADR-0016 §2 の
   * **「事前に知ることができない」を、真逆に断言する文へ静かに置換できる。**
   *
   * **⚠ 案内（`guidance`）の側は同種の変異が赤になっていた。**
   * **同じ画面で守り方が食い違っていた**ので、こちらを揃える。
   */
  it("確認は、取り消せないことと、押してみるまで分からないことを警告する", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();
    const text = confirmPanel().textContent ?? "";

    // **何をするのかを言っている**（「選びました」等に薄めない）。
    expect(text).toContain("削除します");
    // **取り返しがつかないことを言っている。**
    expect(text).toContain("取り消せません");
    // **事前に分からないことを言っている**（ADR-0016 §2）。
    expect(text).toContain("押してみるまで");
    expect(text).toContain("会話");
    expect(text).toContain("拒否");

    // **⚠ 逆向き: 消せると断言していない。**
    // **サーバに訊く前に「消せる」と言えることは、何も無い。**
    expect(text).not.toContain("確実");
    expect(text).not.toContain("必ず消せ");
    expect(text).not.toContain("会話が無い");
    expect(text).not.toContain("会話はありません");

    await unmount(view, { outro: false });
  });

  it("「やめる」で閉じ、何も送らない", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();
    panelButton("やめる").click();
    flushSync();

    expect(target.querySelector("[data-confirm-delete]")).toBeNull();
    expect(deleteCalls()).toEqual([]);
    expect(handlesShown()).toEqual([AOI.handle, MIO.handle, NAR.handle]);

    await unmount(view, { outro: false });
  });
});

describe("確認して初めて送られる", () => {
  it("押した行の識別子へ `DELETE` が飛ぶ（真ん中の行）", async () => {
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    const sent = await vi.waitFor(() => {
      const found = deleteCalls();
      expect(found).toHaveLength(1);
      return found[0] as Call;
    });
    expect(sent.url).toBe(`/admin/characters/${MIO.handle}`);
    expect(sent.url).not.toContain(AOI.handle);
    expect(sent.url).not.toContain(NAR.handle);

    await unmount(view, { outro: false });
  });

  /**
   * **別の識別子で回る腕。**
   *
   * これが無いと **`deleteCharacter("mio")` という決め打ちが通り抜ける**
   * （D-3 のレビューで実測された形 / CLAUDE.md）。
   */
  it("別の行を押せば、別の識別子へ飛ぶ", async () => {
    const view = mount(CharacterList, { target });
    await openAndConfirm(NAR);

    const sent = await vi.waitFor(() => {
      const found = deleteCalls();
      expect(found).toHaveLength(1);
      return found[0] as Call;
    });
    expect(sent.url).toBe(`/admin/characters/${NAR.handle}`);
    expect(sent.url).not.toContain(MIO.handle);
    expect(sent.url).not.toContain(AOI.handle);

    await unmount(view, { outro: false });
  });
});

describe("二重に押させない", () => {
  /**
   * **送っている最中の再押下は、状態の側で止まる。**
   *
   * `disabled` が付くのは**描き直しの後**なので、
   * **同じティックで2回押された分はこちらでしか止められない。**
   * 止まらないと **`DELETE` が2回飛び、2回目は 404 になって
   * 「消せませんでした」と出る**——**消えているのに、である。**
   */
  it("同じティックで2回押しても、送るのは1回", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();
    const button = panelButton("削除する");
    button.click();
    button.click();

    await vi.waitFor(() => {
      expect(handlesShown()).toEqual([AOI.handle, NAR.handle]);
    });
    expect(deleteCalls()).toHaveLength(1);

    await unmount(view, { outro: false });
  });

  /** **描き直した後は、押せる形ですらない。** */
  it("送っている間は、ボタンが押せない形になる", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    deleteButtonOf(MIO).click();
    flushSync();
    // 対照: **押す前は押せる**（常に `disabled` なのではない）。
    expect(panelButton("削除する").disabled).toBe(false);
    expect(panelButton("やめる").disabled).toBe(false);

    panelButton("削除する").click();
    flushSync();
    expect(panelButton("削除する").disabled).toBe(true);
    expect(panelButton("やめる").disabled).toBe(true);

    await unmount(view, { outro: false });
  });
});

describe("成功したあと", () => {
  it("その行だけが一覧から消える", async () => {
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    await vi.waitFor(() => {
      expect(handlesShown()).toEqual([AOI.handle, NAR.handle]);
    });
    // **消えたのがその行だけ**（両隣は残っている）。
    expect(handlesShown()).toContain(AOI.handle);
    expect(handlesShown()).toContain(NAR.handle);
    expect(handlesShown()).not.toContain(MIO.handle);

    await unmount(view, { outro: false });
  });

  it("一覧を読み直している（画面の側で消しただけにしない）", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();
    const before = listGets();

    deleteButtonOf(MIO).click();
    flushSync();
    panelButton("削除する").click();

    await vi.waitFor(() => {
      expect(listGets()).toBe(before + 1);
    });

    await unmount(view, { outro: false });
  });

  it("確認が閉じる", async () => {
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    await vi.waitFor(() => {
      expect(target.querySelector("[data-confirm-delete]")).toBeNull();
    });

    await unmount(view, { outro: false });
  });
});

describe("消せなかったとき（この作業単位の本体）", () => {
  /** 案内の要素。**画面全体の `toContain` を使わない。** */
  function reasonElement(): HTMLElement {
    const found = confirmPanel().querySelector("[data-delete-reason]");
    expect(found, "失敗の理由が出ていること").not.toBeNull();
    return found as HTMLElement;
  }

  function guidanceText(): string | null {
    const found = confirmPanel().querySelector("[data-delete-guidance]");
    return found === null ? null : (found.textContent?.trim() ?? "");
  }

  it("会話があると、その理由と次の一手が出る", async () => {
    refusals.set(MIO.handle, HAS_SESSIONS);
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    await vi.waitFor(() => {
      expect(reasonElement().dataset["deleteReason"]).toBe("has-sessions");
    });
    expect(reasonElement().textContent).toContain(HAS_SESSIONS.message);
    const guidance = guidanceText();
    expect(guidance).not.toBeNull();
    // **実在する導線だけを案内している**（無い操作を案内しない）。
    expect(guidance as string).toContain("編集");
    expect(guidance as string).not.toContain("会話を削除");
    // 404 側の一手とは別（**対処が正反対**）。
    expect(guidance as string).not.toContain("再読み込み");
    // **行は消えていない。**
    expect(handlesShown()).toEqual([AOI.handle, MIO.handle, NAR.handle]);

    await unmount(view, { outro: false });
  });

  it("居ないと言われたときは、別の案内になる", async () => {
    refusals.set(MIO.handle, NOT_FOUND);
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    await vi.waitFor(() => {
      expect(reasonElement().dataset["deleteReason"]).toBe("not-found");
    });
    expect(reasonElement().textContent).toContain(NOT_FOUND.message);
    const guidance = guidanceText();
    expect(guidance).not.toBeNull();
    expect(guidance as string).toContain("再読み込み");
    expect(guidance as string).not.toContain("編集");

    await unmount(view, { outro: false });
  });

  it("それ以外は、サーバが言ったこと以上を出さない", async () => {
    refusals.set(MIO.handle, BROKEN);
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);

    await vi.waitFor(() => {
      expect(reasonElement().dataset["deleteReason"]).toBe("unknown");
    });
    expect(reasonElement().textContent).toContain(BROKEN.message);
    // **次の一手を捏造しない。**
    expect(guidanceText()).toBeNull();

    await unmount(view, { outro: false });
  });

  /**
   * **前の行の失敗を、次の行の確認へ持ち越さない**（レビューの是正 2）。
   *
   * # ⚠ 下の「拒否されても、もう一度別の行を消せる」では観測できない
   *
   * あちらは**先に「やめる」を押している**ので `deleting` が `null` になり、
   * **持ち越しの枝に一度も入らない**（レビューの実測:
   * `failure: null` → `failure: deleting?.failure ?? null` の変異が全緑）。
   *
   * **確認が開いている間も、各行の「削除」は押せる**（`disabled` にしていない）ので、
   * **「やめるを押さずに隣の行を押す」は実際にできる操作**である。
   *
   * 持ち越すと、**A が拒否された理由が、B の確認に出たまま**になる ——
   * **B は消せるのに「会話があるので消せません」と書いてある**状態で、
   * **帰属の壊れ方**（CLAUDE.md）。
   */
  it("やめるを押さずに隣の行を押しても、前の行の失敗が残らない", async () => {
    refusals.set(MIO.handle, HAS_SESSIONS);
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);
    await vi.waitFor(() => {
      expect(reasonElement().dataset["deleteReason"]).toBe("has-sessions");
    });

    // **「やめる」を押さない。** 確認を開いたまま、隣の行の「削除」を押す。
    deleteButtonOf(NAR).click();
    flushSync();

    expect(confirmPanel().dataset["confirmDelete"]).toBe(NAR.handle);
    // **前の行の失敗が消えている**（理由も案内も出ていない）。
    expect(confirmPanel().querySelector("[data-delete-reason]")).toBeNull();
    expect(confirmPanel().querySelector("[data-delete-guidance]")).toBeNull();
    expect(confirmPanel().textContent).not.toContain(HAS_SESSIONS.message);

    await unmount(view, { outro: false });
  });

  it("拒否されても、もう一度別の行を消せる", async () => {
    refusals.set(MIO.handle, HAS_SESSIONS);
    const view = mount(CharacterList, { target });
    await openAndConfirm(MIO);
    await vi.waitFor(() => {
      expect(reasonElement().dataset["deleteReason"]).toBe("has-sessions");
    });

    panelButton("やめる").click();
    flushSync();
    deleteButtonOf(NAR).click();
    flushSync();
    // **前の行の失敗を持ち越さない。**
    expect(confirmPanel().dataset["confirmDelete"]).toBe(NAR.handle);
    expect(confirmPanel().querySelector("[data-delete-reason]")).toBeNull();

    panelButton("削除する").click();
    await vi.waitFor(() => {
      expect(handlesShown()).toEqual([AOI.handle, MIO.handle]);
    });

    await unmount(view, { outro: false });
  });
});
