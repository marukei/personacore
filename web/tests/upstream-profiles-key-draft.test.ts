/**
 * **下書きの鍵の欄と、保存の前の確認**（作業単位 A-13b-3 / Issue origin#86 / `src/lib/upstreamProfiles.ts`）。
 *
 * # 見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **鍵の3通り**（新しい鍵 / 消す / 触らない）**が、依頼の3通り**（文字列 / `null` / 送らない）**になる** | 1つ取り違えると、鍵が黙って消えるか、残るはずのない鍵が残る |
 * | **承認の3通り**（承認する / 外す / 送らない）**が、依頼の `true` / `false` / 送らない になる** | 送らないと今の規則（宛先が同じなら残す）になる |
 * | **宛先だけを変えた鍵のある面で、保存の前に聞く**（利用者の決定1） | 聞かないと、前の宛先の鍵が新しい宛先へ送られる |
 * | **外の `http://` へ鍵を送る面で、保存の前に承認を求める** | ADR-0056 §決定5 |
 * | **裏方のチェックを外すと裏方の鍵も消えることを言う**（利用者の決定2） | 振る舞いは変えない。言うだけ |
 * | **帰属**（どちらの面の鍵・承認・確認か） | 面を取り違えると、別の面の鍵が消える・送られる |
 *
 * # ⚠ フィクスチャは、面同士で1つも同じ値にしない
 *
 * 本文と裏方で、宛先・末尾4文字・承認の有無を全部違える（取り違えの変異を観測できるように）。
 * **期待値の文面は直書き。**
 *
 * **jsdom は要らない**（既定の `node` 環境で走る）。
 */

import { describe, expect, it } from "vitest";

import { ApiRequestError, type WrittenFace, type WrittenProfile } from "../src/lib/api";
import {
  KIND_FIELDS,
  answerKeyQuestion,
  buildSaveRequest,
  clearKey,
  describeSaveFailure,
  draftOfProfile,
  emptyProfileDraft,
  fieldLabel,
  keepKey,
  keySendingNote,
  keySendingState,
  keyStatusText,
  grantApproval,
  nextKeyQuestion,
  resetApproval,
  revokeApproval,
  subKeyLossNote,
} from "../src/lib/upstreamProfiles";

function connect(endpoint: string, model: string, key: { hint: string | null; approved: boolean } | null): WrittenFace {
  return {
    endpoint,
    model,
    gguf: null,
    args: null,
    command: null,
    context_tokens: null,
    api_key_set: key !== null,
    api_key_hint: key?.hint ?? null,
    allow_key_over_plain_http: key?.approved ?? false,
    sampling: null,
    response_tokens: null,
  };
}

/** **本文はループバックの鍵で守られた中継（鍵あり）、裏方はローカルネットワーク（鍵あり・末尾は別）。** */
const KEY_PROTECTED_RELAY: WrittenProfile = {
  name: "relay",
  main: connect("http://127.0.0.1:46001", "relay-main", { hint: "k3Qz", approved: false }),
  sub: connect("http://192.168.0.7:47832", "relay-sub", { hint: "S9x2", approved: false }),
};

/** **本文は外の `http://`（鍵あり・承認あり）、裏方は外の `http://`（鍵あり・承認なし・短い鍵）。** */
const FAR: WrittenProfile = {
  name: "far",
  main: connect("http://203.0.113.5:47833", "far-main", { hint: "F4r1", approved: true }),
  sub: connect("http://203.0.113.6:47834", "far-sub", { hint: null, approved: false }),
};

/**
 * **起こす面に鍵を書いたプロファイル**（`args` の `--api-key` と組で書く構成 —— llama-server の鍵）。
 * 承認は書いていない（起こす面の宛先はループバックだけなので要らない）。
 */
const LAUNCHED: WrittenProfile = {
  name: "launched",
  main: {
    ...connect("http://127.0.0.1:47835", "launched-main", { hint: "L4un", approved: false }),
    gguf: "~/models/launched.gguf",
    args: ["--api-key", "pc-launched-key-L4un"],
  },
  sub: null,
};

describe("鍵の欄は両方の種類の欄（作業単位 A-13b-3 のレビュー是正）", () => {
  it("起こす面も繋ぐ面も、鍵の欄をいつも出す（詳細に畳まない）", () => {
    expect(KIND_FIELDS.connect).toEqual({ shown: ["endpoint", "model", "api_key", "context_tokens"], folded: [] });
    expect(KIND_FIELDS.launch).toEqual({
      shown: ["gguf", "model", "endpoint", "api_key"],
      folded: ["args", "command", "context_tokens"],
    });
    expect([fieldLabel("connect", "api_key"), fieldLabel("launch", "api_key")]).toEqual(["鍵（api_key）", "鍵（api_key）"]);
  });

  it("下書きは、面ごとに開いたときの鍵と承認を持つ（新しい鍵の欄は空）", () => {
    const draft = draftOfProfile(FAR);
    expect([draft.main.api_key, draft.sub.api_key]).toEqual(["", ""]);
    expect(draft.keys.main.written).toEqual({
      set: true,
      hint: "F4r1",
      approved: true,
      endpoint: "http://203.0.113.5:47833",
    });
    expect(draft.keys.sub.written).toEqual({
      set: true,
      hint: null,
      approved: false,
      endpoint: "http://203.0.113.6:47834",
    });
    // **足す下書き・書いていない裏方には、開いたときの鍵が無い。**
    expect(emptyProfileDraft().keys.main.written).toBeNull();
    expect(draftOfProfile(LAUNCHED).keys.sub.written).toBeNull();
  });
});

describe("鍵の3通りと、依頼の3通り", () => {
  it("触らなければ、どちらの面も `api_key` も承認も送らない（＝残す・今の規則）", () => {
    const request = buildSaveRequest(draftOfProfile(KEY_PROTECTED_RELAY), "r");
    for (const face of [request.main, request.sub]) {
      expect(Object.keys(face ?? {})).not.toContain("api_key");
      expect(Object.keys(face ?? {})).not.toContain("allow_key_over_plain_http");
    }
  });

  it("新しい鍵を書いた面だけ、その鍵を送る（もう一方の面は送らない）", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.sub.api_key = "pc-new-sub-key-NS05";
    const request = buildSaveRequest(draft, "r");
    expect(request.sub?.api_key).toBe("pc-new-sub-key-NS05");
    expect(Object.keys(request.main)).not.toContain("api_key");
  });

  it("消すと `null` を送る。消すのをやめれば送らない", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    clearKey(draft, "main");
    expect(buildSaveRequest(draft, "r").main.api_key).toBeNull();
    expect(Object.keys(buildSaveRequest(draft, "r").sub ?? {})).not.toContain("api_key");
    keepKey(draft, "main");
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("api_key");
  });

  it("消すと、書いた新しい鍵の欄も空にする（消すほうが勝つ）", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.api_key = "typed-then-cleared";
    clearKey(draft, "main");
    expect(draft.main.api_key).toBe("");
    expect(buildSaveRequest(draft, "r").main.api_key).toBeNull();
  });

  it("承認のある面の鍵を消すと、承認も外す（`false`）", () => {
    const draft = draftOfProfile(FAR);
    clearKey(draft, "main");
    expect(buildSaveRequest(draft, "r").main).toMatchObject({ api_key: null, allow_key_over_plain_http: false });
    // **承認の無い面は、承認を送らない。**
    clearKey(draft, "sub");
    expect(Object.keys(buildSaveRequest(draft, "r").sub ?? {})).not.toContain("allow_key_over_plain_http");
  });

  /**
   * **起こす面の鍵も、ほかの欄だけを直して保存すれば残る**（作業単位 A-13b-3 のレビュー是正 / ADR-0056 §決定3）。
   *
   * 着手時は鍵を繋ぐ面だけの欄にして、起こす面の鍵を `null` で送っていた —— `--api-key` と組で書いた
   * llama-server の面で、モデル名を直しただけで鍵が消え、次の起動から 401 になった。
   * **種類を切り替えても残る。** 起こす面でも、新しい鍵・消すは繋ぐ面と同じに送る。
   */
  it("起こす面の鍵は、ほかの欄を直しても・種類を切り替えても残り、新しい鍵・消すは送る", () => {
    const draft = draftOfProfile(LAUNCHED);
    expect(draft.kind.main).toBe("launch");
    draft.main.model = "launched-main-2";
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("api_key");
    draft.kind.main = "connect";
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("api_key");
    draft.kind.main = "launch";
    draft.main.api_key = "pc-launched-new-LN05";
    expect(buildSaveRequest(draft, "r").main.api_key).toBe("pc-launched-new-LN05");
    clearKey(draft, "main");
    expect(buildSaveRequest(draft, "r").main.api_key).toBeNull();
  });

  /** **起こす面にも、宛先を変えたときの鍵の確認が効く**（利用者の決定1）。承認は聞かない（宛先はループバック）。 */
  it("起こす面で宛先だけを変えても、鍵を新しい宛先へ送るかを聞く", () => {
    const draft = draftOfProfile(LAUNCHED);
    draft.main.endpoint = "http://127.0.0.1:47999";
    const question = nextKeyQuestion(draft);
    expect(question).toEqual({
      kind: "carry",
      face: "main",
      hint: "L4un",
      from: "http://127.0.0.1:47835",
      to: "http://127.0.0.1:47999",
    });
    answerKeyQuestion(draft, question as NonNullable<typeof question>, "carry");
    expect(nextKeyQuestion(draft)).toBeNull();
    expect(keySendingState(draft, "main")).toEqual({ kind: "none" });
  });

  it("鍵の欄の状態を、面ごとに言う", () => {
    const draft = draftOfProfile(FAR);
    expect([keyStatusText(draft, "main"), keyStatusText(draft, "sub")]).toEqual([
      "設定済みです（…F4r1）。",
      "設定済みです（短い鍵なので、末尾は出しません）。",
    ]);
    draft.main.api_key = "x";
    clearKey(draft, "sub");
    expect([keyStatusText(draft, "main"), keyStatusText(draft, "sub")]).toEqual([
      "保存すると、新しい鍵に置き換えます。",
      "保存すると、鍵を消します。",
    ]);
    const fresh = emptyProfileDraft();
    fresh.kind.main = "connect";
    expect(keyStatusText(fresh, "main")).toBe("書いていません。");
    fresh.main.api_key = "y";
    expect(keyStatusText(fresh, "main")).toBe("保存すると、この鍵を書きます。");
  });
});

describe("宛先だけを変えたときの鍵の確認（利用者の決定1）", () => {
  it("何も変えなければ聞かない（ループバックとローカルネットワークには承認も要らない）", () => {
    expect(nextKeyQuestion(draftOfProfile(KEY_PROTECTED_RELAY))).toBeNull();
  });

  it("鍵に触れずに宛先を変えた面について聞く（変えていない面は聞かない）", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.sub.endpoint = "http://192.168.0.8:47832";
    expect(nextKeyQuestion(draft)).toEqual({
      kind: "carry",
      face: "sub",
      hint: "S9x2",
      from: "http://192.168.0.7:47832",
      to: "http://192.168.0.8:47832",
    });
  });

  it("「送る」と答えると、その宛先については聞かない。宛先をまた変えれば聞き直す", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.endpoint = "http://127.0.0.1:47999";
    const question = nextKeyQuestion(draft);
    expect(question).toMatchObject({ kind: "carry", face: "main" });
    answerKeyQuestion(draft, question as NonNullable<typeof question>, "carry");
    expect(nextKeyQuestion(draft)).toBeNull();
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("api_key");
    draft.main.endpoint = "http://127.0.0.1:48000";
    expect(nextKeyQuestion(draft)).toMatchObject({ kind: "carry", face: "main", to: "http://127.0.0.1:48000" });
  });

  it("「鍵を消す」と答えると、その面の鍵を消して送る（もう一方の面は残す）", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.endpoint = "http://127.0.0.1:47999";
    const question = nextKeyQuestion(draft);
    answerKeyQuestion(draft, question as NonNullable<typeof question>, "clear");
    expect(nextKeyQuestion(draft)).toBeNull();
    const request = buildSaveRequest(draft, "r");
    expect(request.main.api_key).toBeNull();
    expect(Object.keys(request.sub ?? {})).not.toContain("api_key");
  });

  it("新しい鍵を書いた・消すにした面では聞かない（鍵の欄に触れた）", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.endpoint = "http://127.0.0.1:47999";
    draft.main.api_key = "pc-new-main-key-NM06";
    expect(nextKeyQuestion(draft)).toBeNull();
    const cleared = draftOfProfile(KEY_PROTECTED_RELAY);
    cleared.main.endpoint = "http://127.0.0.1:47999";
    clearKey(cleared, "main");
    expect(nextKeyQuestion(cleared)).toBeNull();
  });

  it("裏方を別の上流にしないなら、裏方については聞かない", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.sub.endpoint = "http://192.168.0.8:47832";
    draft.hasSub = false;
    expect(nextKeyQuestion(draft)).toBeNull();
  });

  it("両方の面で宛先を変えたら、本文から聞く", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.endpoint = "http://127.0.0.1:47999";
    draft.sub.endpoint = "http://192.168.0.8:47832";
    const first = nextKeyQuestion(draft);
    expect(first).toMatchObject({ kind: "carry", face: "main" });
    answerKeyQuestion(draft, first as NonNullable<typeof first>, "carry");
    expect(nextKeyQuestion(draft)).toMatchObject({ kind: "carry", face: "sub" });
  });
});

describe("平文送信の承認（ADR-0056 §決定5）", () => {
  it("鍵を書いた面の宛先が IP アドレスで外の `http://` なら、承認を求める", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.sub.api_key = "pc-new-sub-key-NS05";
    draft.sub.endpoint = "http://203.0.113.9:47832";
    expect(nextKeyQuestion(draft)).toEqual({
      kind: "approve",
      face: "sub",
      endpoint: "http://203.0.113.9:47832",
    });
  });

  /**
   * **ホスト名の宛先では、保存の前に止めない**（作業単位 A-13b-3 のレビュー是正 / 司令塔の決定）。
   * 着手時は止めていたので、承認の無い名前の面はほかの欄を直すたびに聞かれた。
   * 欄の隣で言い（`name`）、「承認する」で答えたら、それを送る。
   */
  it("ホスト名なら保存の前には聞かず、欄の隣で言い、承認すれば `true` を送る", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.main.api_key = "pc-new-main-key-NM06";
    draft.main.endpoint = "http://relay.example:46001";
    // 新しい鍵を書いたので鍵の確認は聞かず、ホスト名なので承認も保存の前には聞かない。
    expect(nextKeyQuestion(draft)).toBeNull();
    expect(keySendingState(draft, "main")).toEqual({ kind: "name" });
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("allow_key_over_plain_http");
    grantApproval(draft, "main");
    expect(keySendingState(draft, "main")).toEqual({ kind: "granting" });
    expect(buildSaveRequest(draft, "r").main.allow_key_over_plain_http).toBe(true);
    expect(nextKeyQuestion(draft)).toBeNull();
    // **承認は宛先に付く** —— 名前を変えれば、また欄の隣で言う。
    draft.main.endpoint = "http://other.local:46001";
    expect(keySendingState(draft, "main")).toEqual({ kind: "name" });
  });

  /** **承認済みのホスト名の面は「承認しています」**（「承認を外す」が出る）。 */
  it("承認を書いたホスト名の面は、宛先を変えなければ承認済みと言う", () => {
    const draft = draftOfProfile({
      ...KEY_PROTECTED_RELAY,
      main: connect("http://relay.example:46001", "relay-main", { hint: "k3Qz", approved: true }),
    });
    expect(keySendingState(draft, "main")).toEqual({ kind: "kept" });
    expect(nextKeyQuestion(draft)).toBeNull();
  });

  it("https・ループバック・ローカルネットワーク・鍵の無い面には承認を求めない", () => {
    for (const endpoint of ["https://203.0.113.9", "http://127.0.0.2:1", "http://10.1.2.3:1"]) {
      const draft = draftOfProfile(KEY_PROTECTED_RELAY);
      draft.main.api_key = "pc-new-main-key-NM06";
      draft.main.endpoint = endpoint;
      expect(nextKeyQuestion(draft), endpoint).toBeNull();
    }
    const keyless = emptyProfileDraft();
    keyless.kind.main = "connect";
    keyless.main.endpoint = "http://203.0.113.9:1";
    expect(nextKeyQuestion(keyless)).toBeNull();
  });

  it("承認すると `true`、承認せずに保存すると `false` を送り、もう聞かない", () => {
    for (const [answer, sent] of [
      ["grant", true],
      ["deny", false],
    ] as const) {
      const draft = draftOfProfile(KEY_PROTECTED_RELAY);
      draft.sub.api_key = "pc-new-sub-key-NS05";
      draft.sub.endpoint = "http://203.0.113.9:47832";
      const question = nextKeyQuestion(draft);
      answerKeyQuestion(draft, question as NonNullable<typeof question>, answer);
      expect(nextKeyQuestion(draft), answer).toBeNull();
      const request = buildSaveRequest(draft, "r");
      expect(request.sub?.allow_key_over_plain_http, answer).toBe(sent);
      expect(Object.keys(request.main), answer).not.toContain("allow_key_over_plain_http");
    }
  });

  it("承認は宛先に付く —— 承認した後に宛先を変えれば、聞き直し、承認は送らない", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    draft.sub.api_key = "pc-new-sub-key-NS05";
    draft.sub.endpoint = "http://203.0.113.9:47832";
    const question = nextKeyQuestion(draft);
    answerKeyQuestion(draft, question as NonNullable<typeof question>, "grant");
    draft.sub.endpoint = "http://203.0.113.10:47832";
    expect(nextKeyQuestion(draft)).toMatchObject({ kind: "approve", endpoint: "http://203.0.113.10:47832" });
    expect(Object.keys(buildSaveRequest(draft, "r").sub ?? {})).not.toContain("allow_key_over_plain_http");
  });

  it("承認のある面は、宛先を変えなければ聞かない（今の規則で残る）。変えれば、鍵の確認の後に承認を聞き直す", () => {
    expect(nextKeyQuestion({ ...draftOfProfile(FAR), hasSub: false })).toBeNull();
    const draft = draftOfProfile(FAR);
    draft.hasSub = false;
    draft.main.endpoint = "http://203.0.113.7:47833";
    const carry = nextKeyQuestion(draft);
    expect(carry).toMatchObject({ kind: "carry", face: "main" });
    answerKeyQuestion(draft, carry as NonNullable<typeof carry>, "carry");
    expect(nextKeyQuestion(draft)).toMatchObject({ kind: "approve", face: "main", endpoint: "http://203.0.113.7:47833" });
  });

  it("承認の無い外の宛先の面は、宛先を変えなくても承認を聞く（裏方）", () => {
    const draft = draftOfProfile(FAR);
    expect(nextKeyQuestion(draft)).toEqual({
      kind: "approve",
      face: "sub",
      endpoint: "http://203.0.113.6:47834",
    });
  });

  it("鍵の欄の隣に出す、送り方の状態（面ごと）", () => {
    const draft = draftOfProfile(FAR);
    expect([keySendingState(draft, "main"), keySendingState(draft, "sub")]).toEqual([
      { kind: "kept" },
      { kind: "ask" },
    ]);
    const keyProtectedRelay = draftOfProfile(KEY_PROTECTED_RELAY);
    expect([keySendingState(keyProtectedRelay, "main"), keySendingState(keyProtectedRelay, "sub")]).toEqual([
      { kind: "none" },
      { kind: "local_network" },
    ]);
  });

  /**
   * **状態ごとに別の一言**（直書き）。**ローカルネットワークは警告**（暗号化されないこと・同じネットワークから読めること）、
   * 外は**保存の前に承認を求める**こと、ホスト名は**送るたびに名前解決して判定する**ことを言う。`none` は何も言わない。
   */
  it("送り方の状態ごとに、別の一言を言う", () => {
    expect([
      keySendingNote({ kind: "none" }),
      keySendingNote({ kind: "local_network" }),
      keySendingNote({ kind: "ask" }),
      keySendingNote({ kind: "name" }),
      keySendingNote({ kind: "kept" }),
      keySendingNote({ kind: "granting" }),
      keySendingNote({ kind: "denying" }),
    ]).toEqual([
      null,
      "この宛先は http:// のローカルネットワークなので、鍵は暗号化されずに送られます（承認は要りません）。同じネットワークの機器は、その鍵を読めます。",
      "この宛先は http:// でローカルネットワークの外なので、保存の前に、鍵を暗号化せずに送ることの承認を求めます。",
      "名前で書いた宛先は、送るたびに名前解決して判定します。ローカルネットワークの外に解決したら、承認が無ければ送りません。",
      "この宛先へ、鍵を暗号化せずに送ることを承認しています。",
      "保存すると、この宛先へ鍵を暗号化せずに送ることを承認します。",
      "承認せずに保存します（ローカルネットワークの外へは鍵を送りません）。",
    ]);
  });

  it("「承認を外す」は `false` を送り、「取り消す」で今の規則に戻る", () => {
    const draft = draftOfProfile(FAR);
    revokeApproval(draft, "main");
    expect(keySendingState(draft, "main")).toEqual({ kind: "denying" });
    expect(buildSaveRequest(draft, "r").main.allow_key_over_plain_http).toBe(false);
    resetApproval(draft, "main");
    expect(keySendingState(draft, "main")).toEqual({ kind: "kept" });
    expect(Object.keys(buildSaveRequest(draft, "r").main)).not.toContain("allow_key_over_plain_http");
    // **承認の答えも状態に出る。**
    const question = nextKeyQuestion(draft);
    answerKeyQuestion(draft, question as NonNullable<typeof question>, "grant");
    expect(keySendingState(draft, "sub")).toEqual({ kind: "granting" });
  });
});

describe("裏方のチェックの一言（利用者の決定2）", () => {
  it("チェックを外したとき、裏方の面に鍵があれば、消えることを言う", () => {
    const draft = draftOfProfile(KEY_PROTECTED_RELAY);
    expect(subKeyLossNote(draft)).toBeNull();
    draft.hasSub = false;
    expect(subKeyLossNote(draft)).toBe("保存すると裏方の鍵も消えます");
    // **振る舞いは変えない**（`sub: null` で面ごと消す）。
    expect(buildSaveRequest(draft, "r").sub).toBeNull();
  });

  it("裏方に鍵が無ければ言わない（本文にだけ鍵があっても）", () => {
    const draft = draftOfProfile({ ...KEY_PROTECTED_RELAY, sub: connect("http://127.0.0.1:47836", "s", null) });
    draft.hasSub = false;
    expect(subKeyLossNote(draft)).toBeNull();
    const none = draftOfProfile(LAUNCHED);
    none.hasSub = false;
    expect(subKeyLossNote(none)).toBeNull();
  });
});

describe("保存の失敗", () => {
  it("鍵を名指した 422 は、その面の鍵の欄に結びつける", () => {
    const thrown = new ApiRequestError("保存できません", 422, "invalid_request_error", "upstream_profile_invalid", {
      reason: "api_key_unusable",
      reason_message: "鍵に使えない字があります",
      face: "sub",
      key: "api_key",
    });
    expect(describeSaveFailure(thrown, true)).toEqual({
      kind: "field",
      face: "sub",
      key: "api_key",
      message: "保存しませんでした: 鍵に使えない字があります",
    });
  });
});
