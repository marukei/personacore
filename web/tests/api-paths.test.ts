/**
 * **識別子をパスへ差し込むとき、必ず符号化しているか**（`src/lib/api.ts`）。
 *
 * # なぜ要るのか
 *
 * `api.ts` は **「`handle` は必ずエスケープする」と4箇所で名指ししている**が、
 * **D-3 のレビューで、外しても全部緑になると実測された**
 * （`listMemories` / `correctMemory` はもちろん、**既存の `getStats` も同じ穴**だった）。
 * **doc が「守った」と書いている場所を、誰も見ていない**形（CLAUDE.md）。
 *
 * # 識別子は**利用者の入力**である
 *
 * 記憶の画面の識別子は**ハッシュ由来**（`router.ts` の `screenOf` が
 * `decodeURIComponent` して返す）。したがって `#/memories/%2E%2E%2Fx` と打てば
 * `handle` は `../x` になり、**符号化を外すと
 * `/admin/characters/../x/memories` が組み上がる。**
 *
 * **I/O は `fetch` の差し替えだけ**なので jsdom は要らない（既定の `node` で走る）。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  adoptAgreement,
  correctMemory,
  createAgreement,
  deleteCharacter,
  endAgreement,
  listAgreementTransitions,
  listAgreements,
  getAttributes,
  getDrift,
  getStats,
  listAttributeRevisions,
  listMemories,
  listSessionMessages,
  listSessions,
  rejectAgreement,
  replaceAttributes,
  replaceStats,
  saveUpstreamProfile,
  switchUpstreamProfile,
  updateCharacter,
} from "../src/lib/api";

/** **パスを変えようとする識別子。** `/` も `.` も `%` も含む。 */
const HOSTILE = "../x y%z";

let urls: string[];

beforeEach(() => {
  urls = [];
  vi.stubGlobal("fetch", (input: unknown) => {
    urls.push(String(input));
    return Promise.resolve({
      ok: true,
      status: 200,
      text: () => Promise.resolve("{}"),
    } as unknown as Response);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

/** 送られたパスが、識別子を1段として閉じ込めているか。 */
function expectContained(url: string, suffix: string): void {
  // **段を増やしていない**（`..` も空白も生のまま出ていない）。
  expect(url).toBe(`/admin/characters/..%2Fx%20y%25z${suffix}`);
  expect(url).not.toContain("/../");
  expect(url).not.toContain(" ");
  // 対照: 符号化された形は実際に入っている（別物へ書き換えていない）。
  expect(url).toContain("%2F");
}

describe("記憶の2本（作業単位 D-3）", () => {
  it("一覧の識別子を符号化する", async () => {
    await listMemories(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/memories");
  });

  /**
   * **問い合わせを足しても、識別子は1段のまま**（作業単位 D-3c）。
   *
   * **`?` を手で繋ぐ実装は、識別子の符号化を巻き添えにしやすい**
   * （テンプレートを組み直すときに `encodeURIComponent` が落ちる）。
   */
  it("上書き済みを足して引くときも、識別子を符号化する", async () => {
    await listMemories(HOSTILE, "superseded");
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/memories?include=superseded");
  });

  it("訂正の識別子を符号化する（記憶の id はそのまま1段）", async () => {
    await correctMemory(HOSTILE, 22, { subject: "user", content: "本文" });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/memories/22/corrections");
  });
});

describe("既存の入口（同じ穴だと実測された分）", () => {
  it("性格ステータスの識別子を符号化する", async () => {
    await getStats(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/stats");
  });

  it("更新の識別子を符号化する", async () => {
    await updateCharacter(HOSTILE, { display_name: "澪" });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "");
  });
});

describe("性格値と変容の2本（作業単位 D-4）", () => {
  /**
   * **置き換える側こそ外せない。** 段を増やせる識別子で
   * `/admin/characters/../x/stats` が組み上がると、
   * **指したつもりのないキャラクターの性格値が丸ごと置き換わる**
   * （`PUT` は全消し→挿入）。
   */
  it("置き換えの識別子を符号化する", async () => {
    await replaceStats(HOSTILE, { stats: [{ axis: "勇気", value: 1 }] });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/stats");
  });

  it("変容の識別子を符号化する", async () => {
    await getDrift(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/drift");
  });
});

describe("属性の3本（作業単位 D-12a）", () => {
  it("いまの値の識別子を符号化する", async () => {
    await getAttributes(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/attributes");
  });

  /**
   * **置き換える側こそ外せない。** 段を増やせる識別子で
   * `/admin/characters/../x/attributes` が組み上がると、
   * **指したつもりのないキャラクターの属性が丸ごと置き換わる**
   * （`PUT` は全消し→挿入）——**しかも属性は削れない枠に毎ターン載る。**
   */
  it("置き換えの識別子を符号化する", async () => {
    await replaceAttributes(HOSTILE, {
      attributes: [{ subject: "character", key: "一人称", value: "僕" }],
    });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/attributes");
  });

  /** **段が2つ増える形**（`/attributes/revisions`）でも、識別子は1段のまま。 */
  it("変更の記録の識別子を符号化する", async () => {
    await listAttributeRevisions(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/attributes/revisions");
  });
});

/**
 * **会話履歴の2本**（作業単位 D-12c-2）。
 *
 * 会話の画面の識別子は**キャラクターの一覧から選ぶ**ので、いまは利用者が打った綴りではない。
 * **それでも外さない** —— 外すと、この2本だけが「たまたま安全」で守られる形になる。
 */
describe("会話履歴の2本（作業単位 D-12c-2）", () => {
  it("会話の一覧の識別子を符号化する", async () => {
    await listSessions(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/sessions");
  });

  it("1つの会話のメッセージの識別子を符号化する（セッションの id はそのまま1段）", async () => {
    await listSessionMessages(HOSTILE, 41);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/sessions/41/messages");
  });

  /** **遡る問い合わせを足しても、識別子は1段のまま**（`listMemories` の `include` と同じ）。 */
  it("遡るときも、識別子を符号化する", async () => {
    await listSessionMessages(HOSTILE, 41, 7);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/sessions/41/messages?before=7");
  });
});

/**
 * **約束・決まりの4本**（作業単位 A-12a）。
 *
 * **書く側・終わらせる側こそ外せない。** 段を増やせる識別子で、指したつもりのない
 * キャラクターに約束が書かれると、**そのキャラクターの以後の全ターンに載る。**
 */
describe("約束・決まりの4本（作業単位 A-12a）", () => {
  it("一覧の識別子を符号化する", async () => {
    await listAgreements(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements");
  });

  it("書く識別子を符号化する", async () => {
    await createAgreement(HOSTILE, {
      content: "毎朝あいさつする",
      kind: "rule",
      keeper: "both",
    });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements");
  });

  it("終わらせる識別子を符号化する（取り決めの id はそのまま1段）", async () => {
    await endAgreement(HOSTILE, 13, { state: "fulfilled" });
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements/13/transitions");
  });

  it("記録の識別子を符号化する", async () => {
    await listAgreementTransitions(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements/transitions");
  });

  // **提案を採る・捨てるの2本**（作業単位 A-12c-2）。**採る側こそ外せない** —— 採ると以後の全ターンに載る。
  it("提案を採る識別子を符号化する（取り決めの id はそのまま1段）", async () => {
    await adoptAgreement(HOSTILE, 32, {});
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements/32/adoption");
  });

  it("提案を捨てる識別子を符号化する", async () => {
    await rejectAgreement(HOSTILE, 32);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "/agreements/32/rejection");
  });
});

describe("削除（作業単位 D-7）", () => {
  /**
   * **消す側こそ外せない。** 段を増やせる識別子で
   * `/admin/characters/../x/...` が組み上がると、
   * **指したつもりのない資源へ `DELETE` が飛ぶ。**
   */
  it("削除の識別子を符号化する", async () => {
    await deleteCharacter(HOSTILE);
    expect(urls).toHaveLength(1);
    expectContained(urls[0] as string, "");
  });
});

/**
 * **書いたプロファイルの保存**（作業単位 F-4b-3）。
 *
 * **名前は利用者が画面で打った綴り**なので、`/` や `..` を含みうる。
 * 符号化を外すと `/admin/upstream/profiles/../x y%z` が組み上がり、
 * **指したつもりのない資源へ `PUT` が飛ぶ。**
 */
describe("上流プロファイルの保存（作業単位 F-4b-3）", () => {
  it("保存の名前を符号化する", async () => {
    await saveUpstreamProfile(HOSTILE, {
      revision: null,
      main: { endpoint: "e", model: "m", gguf: null, args: null, command: null, context_tokens: null },
      sub: null,
    });
    expect(urls).toEqual(["/admin/upstream/profiles/..%2Fx%20y%25z"]);
  });

  /** **差し替えは名前を本文で運ぶ**（パスに入れない）。 */
  it("差し替えの名前は、本文で運ぶ（パスは固定）", async () => {
    await switchUpstreamProfile(HOSTILE);
    expect(urls).toEqual(["/admin/upstream/profile"]);
  });
});
