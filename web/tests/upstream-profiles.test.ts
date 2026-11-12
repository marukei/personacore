/**
 * **書いたプロファイルを、画面の行と下書きに組む**（作業単位 F-4b-3 / `src/lib/upstreamProfiles.ts`）。
 *
 * # 見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **行の帰属**（どのプロファイルの、どちらの面の、どの鍵か） | 画面の表は全部ここで組む。入れ替えの変異を1本で押さえる |
 * | **下書き → 送る本文の往復** | 保存した後の一覧が、送った内容と一致すること（Issue origin#24 の閉じる条件）の前提 |
 * | **保存の失敗の読み分け** | 422 を欄に結びつける・409 で黙って上書きしない |
 * | **起こした上流の状態の文面** | 「載せている / 準備完了 / 失敗」が画面に出ること |
 * | **写し元の綴り**（`FACE_NAMES` / `FACE_KEYS` / `DEFAULT_COMMAND` / `FaceState::label`） | ずれると、欄に結びつかない・状態が生の綴りで出る |
 *
 * # ⚠ フィクスチャは、プロファイル同士・面同士で1つも同じ値にしない
 *
 * 同値だと、**帰属を取り違える変異が観測できない**（CLAUDE.md の最頻の穴）。
 *
 * **期待値は `upstreamProfiles.ts` の定数から作らない**（文面を緩める変異が、
 * 期待値も一緒に動かして緑になる）。
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it, vi } from "vitest";

import {
  ApiRequestError,
  type LaunchView,
  type ProfilesView,
  type UpstreamReport,
  type WrittenFace,
  type WrittenFaceRequest,
  type WrittenProfile,
} from "../src/lib/api";
import {
  DEFAULT_COMMAND,
  EDITABLE_FACE_KEYS,
  FACE_KEYS,
  FACES,
  KIND_FIELDS,
  LAUNCH_STATE,
  argsOfText,
  buildSaveRequest,
  canEdit,
  contextTokensFailure,
  describeSaveFailure,
  draftOfProfile,
  emptyProfileDraft,
  endpointEndsWithV1,
  endpointV1Failure,
  faceKindBadge,
  faceKindLabel,
  failureOnScreen,
  fieldLabel,
  hasLoadingFace,
  isProfilesView,
  joinedFlagFailure,
  keepOnShownField,
  launchLabel,
  launchWithoutGgufFailure,
  listingNotices,
  nameTaken,
  profileToStart,
  summaryLines,
} from "../src/lib/upstreamProfiles";

/** **3件。すべての値が互いに違う**（名前・面・鍵のどれを取り違えても値が変わる）。 */
/**
 * **`context_tokens` は `alpha` の本文と `beta` の裏方にだけ書く**（作業単位 A-13a-2）。
 * どちらも既定（8192）・落とし所（16384）と違い、互いにも違う。
 */
const ALPHA: WrittenProfile = {
  name: "alpha",
  main: {
    endpoint: "http://127.0.0.1:41001",
    model: "alpha-main",
    gguf: "~/models/alpha-main.gguf",
    args: ["--ctx-size", "4096"],
    command: "C:/tools/llama-a.exe",
    context_tokens: 28672,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
  },
  sub: {
    endpoint: "http://127.0.0.1:41002",
    model: "alpha-sub",
    gguf: "~/models/alpha-sub.gguf",
    args: ["-ngl", "10"],
    command: "C:/tools/llama-b.exe",
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
  },
};

const BETA: WrittenProfile = {
  name: "beta",
  main: {
    endpoint: "http://127.0.0.1:41003",
    model: "beta-main",
    gguf: "D:\\models\\beta main.gguf",
    args: ["--threads", "6", "--alias", "two words"],
    command: "D:/bin/llama-beta.exe",
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
  },
  sub: {
    endpoint: "http://127.0.0.1:41004",
    model: "beta-sub",
    gguf: null,
    args: null,
    command: null,
    context_tokens: 10240,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
  },
};

const GAMMA: WrittenProfile = {
  name: "gamma",
  main: {
    endpoint: "http://127.0.0.1:41005",
    model: "gamma-main",
    gguf: null,
    args: null,
    command: null,
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
  },
  sub: null,
};


describe("フィクスチャ", () => {
  it("プロファイル同士・面同士で、1つも同じ値が無い", () => {
    const values = [ALPHA, BETA, GAMMA].flatMap((profile) =>
      [profile.main, profile.sub].flatMap((face) =>
        face === null
          ? []
          : [
              face.endpoint,
              face.model,
              face.gguf,
              face.command,
              face.args?.join(" "),
              face.context_tokens === null ? null : String(face.context_tokens),
            ],
      ),
    );
    const present = values.filter((value) => value !== null && value !== undefined);
    expect(new Set(present).size).toBe(present.length);
  });
});

/**
 * **一覧の1行**（作業単位 D-19）。`[面の見出し, 種類の印, 要点, 補足]` の形に読む。
 * 「本文と同じ」は `[見出し, null, "本文と同じ", []]`。
 */
function linesOf(profile: WrittenProfile): unknown[][] {
  return summaryLines(profile).map((line) =>
    line.summary === null
      ? [line.label, null, SAME_AS_MAIN_SUMMARY, []]
      : [line.label, line.summary.kind, line.summary.target, line.summary.extras],
  );
}

/** `summaryLines` が「本文と同じ」と言うときの文面（**直書き**。定数から作らない）。 */
const SAME_AS_MAIN_SUMMARY = "本文と同じ";

describe("書いたプロファイルを、面ごとの要点の1行にする（作業単位 D-19）", () => {
  /**
   * **1件の2行を丸ごと突き合わせる。** 面を入れ替える変異・種類を取り違える変異・
   * 要点の素材（gguf / endpoint）を取り違える変異のどれも、この1本で赤になる。
   */
  it("起こす面は gguf のファイル名と待ち受けのポート、窓を書いていれば窓を言う", () => {
    expect(linesOf(ALPHA)).toEqual([
      ["本文", "launch", "alpha-main.gguf → :41001", ["窓 28,672"]],
      ["裏方", "launch", "alpha-sub.gguf → :41002", []],
    ]);
  });

  /** **面ごとに種類が違う1件**（本文は起こす・裏方は繋ぐ）。**バックスラッシュの区切りと、空白を含むファイル名。** */
  it("繋ぐ面は宛先を書いたまま言う。面ごとに種類が違っても取り違えない", () => {
    expect(linesOf(BETA)).toEqual([
      ["本文", "launch", "beta main.gguf → :41003", []],
      ["裏方", "connect", "http://127.0.0.1:41004", ["窓 10,240"]],
    ]);
  });

  it("裏方を書いていなければ、本文と同じだと言う（本文の値を写さない）", () => {
    expect(linesOf(GAMMA)).toEqual([
      ["本文", "connect", "http://127.0.0.1:41005", []],
      ["裏方", null, SAME_AS_MAIN_SUMMARY, []],
    ]);
  });

  /** **書いていない鍵を、決まる値で埋めない**（既定の実行ファイル名・既定の窓）。 */
  it("書いていない実行ファイル・窓を、既定値で埋めない", () => {
    const shown = JSON.stringify(linesOf(BETA));
    expect(shown).not.toContain(DEFAULT_COMMAND);
    expect(shown).not.toContain("8192");
    expect(shown).not.toContain("8,192");
  });

  /** **鍵は値を出さない**（応答にも載っていない）。**末尾4文字が届いていれば添える。** */
  it("鍵を書いた面は「鍵あり」と言い、末尾4文字があれば添える。窓より前に並ぶ", () => {
    const keyed: WrittenProfile = {
      name: "relay",
      main: { ...(BETA.sub as WrittenFace), api_key_set: true, api_key_hint: "k3Qz", context_tokens: 32768 },
      sub: { ...ALPHA.main, api_key_set: true, api_key_hint: null, context_tokens: null },
    };
    expect(linesOf(keyed)).toEqual([
      ["本文", "connect", "http://127.0.0.1:41004", ["鍵あり（…k3Qz）", "窓 32,768"]],
      ["裏方", "launch", "alpha-main.gguf → :41001", ["鍵あり"]],
    ]);
  });

  /** **ポートが読めない宛先は、書いたまま出す**（切り詰めた結果を作らない）。 */
  it("ポートを書いていない・読めない宛先なら、矢印の先は書いたままの宛先", () => {
    const face = (endpoint: string, gguf: string): WrittenProfile => ({
      name: "p",
      main: { ...ALPHA.main, endpoint, gguf, context_tokens: null },
      sub: null,
    });
    expect(summaryLines(face("http://localhost/", "x.gguf"))[0]?.summary?.target).toBe(
      "x.gguf → http://localhost/",
    );
    // **URL として読めない綴りは出さない**（作業単位 B-71。伏せ損ねた綴りをそのまま出さない —— ログと同じ）。
    // 書いたままの値は「直す」の欄に出る。
    expect(summaryLines(face("not a url", "~/m/y.gguf"))[0]?.summary?.target).toBe("y.gguf → <読めない宛先>");
  });

  /**
   * **宛先に書いた秘密（ユーザー情報・クエリ・フラグメント）は、一覧の要点に出さない**（作業単位 B-71）。
   *
   * **欄ごとに別の秘密**を入れる —— 1つを落とし忘れた実装を、残りが隠さない。
   * **肯定側**（ホストとポート・パスは残る）を同じ行で先に見る。**繋ぐ面**（宛先そのものを出す）と、
   * **起こす面のポートを読めない宛先**（書いたままの宛先へ落ちる枝）の両方を通す。
   */
  it("宛先のユーザー情報・クエリ・フラグメントは要点に出さず、ホスト・ポート・パスは出す", () => {
    const leaky = (endpoint: string, gguf: string | null): string | undefined =>
      summaryLines({ name: "p", main: { ...ALPHA.main, endpoint, gguf, context_tokens: null }, sub: null })[0]
        ?.summary?.target;
    const shown = [
      leaky("http://user:pw-secret@127.0.0.1:47941/v1?token=q-secret#f-secret", null),
      leaky("http://127.0.0.1:47942?token=q2-secret", null),
      leaky("https://user:pw2-secret@localhost/api?token=q3-secret#f3-secret", "x.gguf"),
    ];
    expect(shown).toEqual([
      "http://127.0.0.1:47941/v1",
      "http://127.0.0.1:47942/",
      "x.gguf → https://localhost/api",
    ]);
    for (const secret of ["user", "pw-secret", "q-secret", "f-secret", "q2-secret", "pw2-secret", "q3-secret", "f3-secret"]) {
      expect(JSON.stringify(shown), secret).not.toContain(secret);
    }
  });

  /**
   * **サーバのログ（`llm::endpoint_for_log`）と同じ綴りに揃える**（B-71 のレビュー是正）。
   *
   * - **ホストを持たない URL は出さない。** `http://` を書き忘れた `user:pw-secret@127.0.0.1:8080` は、
   *   WHATWG ではスキーム `user` のホストの無い URL として読めてしまい、秘密になりうる部分が空に見える。
   * - **中身の無いクエリ・フラグメント（`?` / `#` だけ）も落とす**（サーバは落としている）。
   * - **落とすものが無ければ書いたまま**（既定のポートも消さない）。
   */
  it("ホストの無い宛先は出さず、空のクエリ・フラグメントも落とし、落とすものが無ければ書いたまま", () => {
    const target = (endpoint: string): string | undefined =>
      summaryLines({ name: "p", main: { ...ALPHA.main, endpoint, gguf: null, context_tokens: null }, sub: null })[0]
        ?.summary?.target;
    expect([
      target("user:pw-secret@127.0.0.1:8080"),
      target("localhost:1234"),
      target("http://127.0.0.1:9/v1?"),
      target("http://127.0.0.1:9/v1#"),
      target("http://127.0.0.1:80"),
      target("http://127.0.0.1:9"),
    ]).toEqual([
      "<読めない宛先>",
      "<読めない宛先>",
      "http://127.0.0.1:9/v1",
      "http://127.0.0.1:9/v1",
      "http://127.0.0.1:80",
      "http://127.0.0.1:9",
    ]);
  });

  /**
   * **ポートは URL として読む**（D-19 のレビュー）。IPv6 の角括弧・https・既定のポートの3つで、
   * 綴りから `:数字` を拾う読み方（`/:(\d+)/`）と見分ける。
   * **既定のポート（http の 80 / https の 443）は URL がポートを持たないと読む**ので、書いたままの宛先を出す。
   */
  it("IPv6・https のポートも読み、既定のポートは書いたままの宛先を出す", () => {
    const target = (endpoint: string): string | undefined =>
      summaryLines({ name: "p", main: { ...ALPHA.main, endpoint, gguf: "x.gguf", context_tokens: null }, sub: null })[0]
        ?.summary?.target;
    expect([
      target("http://[::1]:18081"),
      target("https://127.0.0.1:8443"),
      target("http://127.0.0.1:80"),
      target("https://localhost:443/v1"),
    ]).toEqual([
      "x.gguf → :18081",
      "x.gguf → :8443",
      "x.gguf → http://127.0.0.1:80",
      "x.gguf → https://localhost:443/v1",
    ]);
  });

  /**
   * **窓の3桁区切りは、閲覧器の地域設定に依らない**（D-19 のレビュー）。
   *
   * Node の既定の地域は起動時に決まり、テストの中からは替えられない。**`Number.prototype.toLocaleString` を
   * `de-DE` の書き方（`28.672`）に差し替えて、既定の地域がドイツ語である閲覧器を模す。**
   * **⚠ 模しているのは `toLocaleString` の経路だけ**（`Intl.NumberFormat()` を直に使う実装はこの腕では見分けない）。
   */
  it("地域設定が de-DE の閲覧器でも、窓は「28,672」と区切る", () => {
    const german = new Intl.NumberFormat("de-DE");
    const spy = vi
      .spyOn(Number.prototype, "toLocaleString")
      .mockImplementation(function (this: number) {
        return german.format(this);
      });
    try {
      // **対照: 差し替えが効いている**（効いていなければ、この腕は何も見ていない）。
      expect((28672).toLocaleString()).toBe("28.672");
      expect(summaryLines(ALPHA)[0]?.summary?.extras).toEqual(["窓 28,672"]);
    } finally {
      spy.mockRestore();
    }
  });

  it("種類の印と、種類を選ぶボタンの文面", () => {
    expect([faceKindBadge("launch"), faceKindBadge("connect")]).toEqual(["起こす", "繋ぐ"]);
    expect([faceKindLabel("launch"), faceKindLabel("connect")]).toEqual([
      "この機で llama を起こす",
      "動いているサーバに繋ぐ",
    ]);
  });
});

describe("面の種類（作業単位 D-19）", () => {
  /** **直すときの種類は、その面に gguf があれば起こす、無ければ繋ぐ。** 裏方を書いていなければ繋ぐ。 */
  it("直す下書きを開いたときの種類は、gguf の有無で決まる", () => {
    expect([ALPHA, BETA, GAMMA].map((profile) => draftOfProfile(profile).kind)).toEqual([
      { main: "launch", sub: "launch" },
      { main: "launch", sub: "connect" },
      { main: "connect", sub: "connect" },
    ]);
  });

  /** **足すときは、両面とも起こすで開く**（利用者が承認した絵のとおり / D-19 の是正）。裏方はチェックを入れたときに出る。 */
  it("足す下書きは、本文も裏方も起こすで開く", () => {
    expect(emptyProfileDraft().kind).toEqual({ main: "launch", sub: "launch" });
  });

  /** **欄の見出しは種類で変わる**（起こす面の endpoint は「待ち受ける場所」）。 */
  it("起こす面と繋ぐ面で、endpoint の見出しが違う", () => {
    expect([fieldLabel("launch", "endpoint"), fieldLabel("connect", "endpoint")]).toEqual([
      "待ち受ける場所（endpoint）",
      "宛先（endpoint）",
    ]);
    expect(fieldLabel("connect", "context_tokens")).toBe("コンテキスト長（context_tokens）");
  });

  /**
   * **どの欄をどこに出すか**（起こす: 3つを出し、3つを詳細に畳む / 繋ぐ: 3つ）。
   * **起こす面にも `context_tokens` を書ける**（ADR-0054）ので、詳細に置く —— データが受け付ける欄を黙って落とさない。
   */
  /** **鍵は両方の種類の欄**（作業単位 A-13b-3 で足し、レビュー是正で起こす面にも出した）。 */
  it("起こす面は7つの欄を全部持ち、繋ぐ面は宛先・モデル・鍵・コンテキスト長だけを持つ", () => {
    expect(KIND_FIELDS).toEqual({
      launch: { shown: ["gguf", "model", "endpoint", "api_key"], folded: ["args", "command", "context_tokens"] },
      connect: { shown: ["endpoint", "model", "api_key", "context_tokens"], folded: [] },
    });
  });

  /**
   * **繋ぐにしたら、gguf・引数・実行ファイルを送らない。下書きには残る**（戻せば元の値を送る）。
   * **もう片方の面は動かない**（面の取り違えを見る）。
   */
  it("繋ぐにした面は gguf・引数・実行ファイルを送らず、コンテキスト長は送る。戻せば元に戻る", () => {
    const draft = draftOfProfile(ALPHA);
    draft.kind.main = "connect";
    const request = buildSaveRequest(draft, "r");
    expect(request.main).toEqual({
      endpoint: "http://127.0.0.1:41001",
      model: "alpha-main",
      gguf: null,
      args: null,
      command: null,
      context_tokens: 28672,
    });
    expect(request.sub).toEqual(requested(ALPHA.sub as WrittenFace));
    // **下書きには残っている。**
    expect(draft.main.gguf).toBe("~/models/alpha-main.gguf");
    draft.kind.main = "launch";
    expect(buildSaveRequest(draft, "r").main).toEqual(requested(ALPHA.main));
  });

  it("裏方だけを繋ぐにしても、本文は起こす面のまま送る", () => {
    const draft = draftOfProfile(ALPHA);
    draft.kind.sub = "connect";
    const request = buildSaveRequest(draft, "r");
    expect(request.main).toEqual(requested(ALPHA.main));
    expect(request.sub).toEqual({
      endpoint: "http://127.0.0.1:41002",
      model: "alpha-sub",
      gguf: null,
      args: null,
      command: null,
      context_tokens: null,
    });
  });

  /**
   * **起こすにした面は、選んだ種類の欄を全部送る**（gguf・引数・実行ファイル・コンテキスト長）。
   * **gguf を空のまま起こす面は送らない**（[`launchWithoutGgufFailure`] が保存の前に止める / D-19 のレビュー）。
   */
  it("起こすにした面は、6つの欄を全部送る（コンテキスト長も落とさない）", () => {
    const draft = draftOfProfile(BETA);
    draft.kind.sub = "launch";
    draft.sub.gguf = "E:\\m\\sub.gguf";
    draft.sub.args = "-ngl\n5";
    draft.sub.command = "E:/llama.exe";
    expect(buildSaveRequest(draft, "r").sub).toEqual({
      endpoint: "http://127.0.0.1:41004",
      model: "beta-sub",
      gguf: "E:\\m\\sub.gguf",
      args: ["-ngl", "5"],
      command: "E:/llama.exe",
      context_tokens: 10240,
    });
  });

  /** **繋ぐ面の引数は送らない**ので、旗と値を1行に書いた形が残っていても止めない。 */
  it("繋ぐ面に残った引数では、旗と値の1行書きを止めない", () => {
    const draft = draftOfProfile(ALPHA);
    draft.main.args = "--ctx-size 16384";
    draft.kind.main = "connect";
    expect(joinedFlagFailure(draft)).toBeNull();
    // 対照: 起こす面なら止める。
    draft.kind.main = "launch";
    expect(joinedFlagFailure(draft)).toMatchObject({ kind: "field", face: "main", key: "args" });
  });

  /**
   * **欄の無い鍵を名指した理由は、全体に出す**（出す先の無い欄に結びつけて、理由を消さない）。
   * 繋ぐ面には gguf・引数・実行ファイルの欄が無い。
   */
  it("その面の種類に無い欄を名指した理由は全体に、在る欄なら欄に結びつける", () => {
    const kinds = { main: "connect", sub: "launch" } as const;
    const field = (face: "main" | "sub", key: "gguf" | "args" | "command" | "endpoint" | "context_tokens") =>
      ({ kind: "field", face, key, message: "m" }) as const;
    for (const key of ["gguf", "args", "command"] as const) {
      expect(keepOnShownField(field("main", key), kinds), key).toEqual({ kind: "form", message: "m" });
      expect(keepOnShownField(field("sub", key), kinds), key).toEqual(field("sub", key));
    }
    expect(keepOnShownField(field("main", "endpoint"), kinds)).toEqual(field("main", "endpoint"));
    expect(keepOnShownField(field("main", "context_tokens"), kinds)).toEqual(field("main", "context_tokens"));
    // **欄でない失敗は、そのまま。**
    expect(keepOnShownField({ kind: "changed", message: "c" }, kinds)).toEqual({ kind: "changed", message: "c" });
  });

  /**
   * **いま画面に在る欄にだけ結びつける**（D-19 のレビュー）。理由が出た後に種類や裏方のチェックを切り替えて
   * **その欄が消えたら、理由は下書き全体へ移る**（理由を画面から消さない）。戻せば欄へ戻る。
   */
  it("いま画面に無い欄（種類に無い・裏方を別にしていない）に結びついた理由は、全体に出す", () => {
    const field = (face: "main" | "sub", key: "gguf" | "args" | "endpoint") =>
      ({ kind: "field", face, key, message: "m" }) as const;
    const both = { hasSub: true, kind: { main: "launch", sub: "launch" } } as const;
    expect(failureOnScreen(field("sub", "args"), both)).toEqual(field("sub", "args"));
    // 裏方を別にしないと決めた → 裏方の欄は画面に無い。
    expect(failureOnScreen(field("sub", "args"), { ...both, hasSub: false })).toEqual({ kind: "form", message: "m" });
    // 本文を繋ぐにした → 本文の gguf の欄は無いが、宛先の欄は在る。
    const mainConnect = { hasSub: true, kind: { main: "connect", sub: "launch" } } as const;
    expect(failureOnScreen(field("main", "gguf"), mainConnect)).toEqual({ kind: "form", message: "m" });
    expect(failureOnScreen(field("main", "endpoint"), mainConnect)).toEqual(field("main", "endpoint"));
    // **もう片方の面は動かない**（裏方は起こすのまま）。
    expect(failureOnScreen(field("sub", "gguf"), mainConnect)).toEqual(field("sub", "gguf"));
    expect(failureOnScreen({ kind: "name", message: "n" }, mainConnect)).toEqual({ kind: "name", message: "n" });
  });
});

/**
 * **起こす面で gguf を空のまま保存しない**（D-19 のレビュー / 必須）。
 *
 * サーバは gguf を書いた面を起こすので、空のまま送ると**黙って繋ぐ面として書かれ**、切り替えると誰も待ち受けていない
 * 宛先へ繋ぎに行く（次に直して保存すると引数も消える）。**送らずに、その面の gguf の欄で言う。**
 * **期待値の文面は直書き。**
 */
describe("起こす面の gguf", () => {
  const MESSAGE =
    "保存しませんでした: 起こす面には gguf のパスを書いてください。繋ぐなら「動いているサーバに繋ぐ」を選んでください。";

  it("起こす面の gguf が空なら、その面の gguf の欄に結びつけて止める", () => {
    expect(launchWithoutGgufFailure(emptyProfileDraft())).toEqual({
      kind: "field",
      face: "main",
      key: "gguf",
      message: MESSAGE,
    });
  });

  it("空白だけの gguf も空として止める", () => {
    const draft = draftOfProfile(ALPHA);
    draft.main.gguf = "  ";
    expect(launchWithoutGgufFailure(draft)).toMatchObject({ face: "main", key: "gguf" });
  });

  it("対照: gguf を書いた起こす面・繋ぐ面は止めない", () => {
    expect(launchWithoutGgufFailure(draftOfProfile(ALPHA))).toBeNull();
    const draft = emptyProfileDraft();
    draft.kind.main = "connect";
    expect(launchWithoutGgufFailure(draft)).toBeNull();
  });

  /** **裏方は、別の上流にするときだけ見る**（別にしないなら裏方の欄は送らない）。両方なら本文を先に言う。 */
  it("裏方の gguf は、裏方を別にするときだけ見る。両方なら本文を先に言う", () => {
    const draft = draftOfProfile(ALPHA);
    draft.sub.gguf = "";
    draft.hasSub = false;
    expect(launchWithoutGgufFailure(draft)).toBeNull();
    draft.hasSub = true;
    expect(launchWithoutGgufFailure(draft)).toEqual({ kind: "field", face: "sub", key: "gguf", message: MESSAGE });
    draft.main.gguf = "";
    expect(launchWithoutGgufFailure(draft)).toMatchObject({ face: "main" });
  });
});

/**
 * **末尾が `/v1` の宛先は、保存の前に止める**（作業単位 B-68 / Issue origin#97）。
 *
 * personacore は宛先の後ろに `/v1/chat/completions` などを足すので、`…/v1` まで書くと `…/v1/v1/…` に送る。
 * **判定はサーバ（`src/llm/lmstudio.rs` の `endpoint_ends_with_v1`）と同じ表を読む**（写しのずれを、両側の赤で拾う）。
 * **期待値の文面は直書き。**
 */
describe("末尾が /v1 の宛先（作業単位 B-68）", () => {
  const V1_CORPUS = readFileSync(
    fileURLToPath(new URL("../../tests/fixtures/endpoint_v1_suffix_corpus.tsv", import.meta.url)),
    "utf8",
  );
  /** 表の行（`#` の行と空の行を除く）。**`\r` を落としてから割る**（作業ツリーは CRLF になりうる）。 */
  const rows: [string, string][] = V1_CORPUS.split(/\r?\n/u)
    .filter((line) => !line.startsWith("#") && line.trim() !== "")
    .map((line) => {
      const parts = line.split("\t");
      if (parts.length !== 2) {
        throw new Error(`宛先<TAB>分類 の形ではありません: ${JSON.stringify(line)}`);
      }
      return [parts[0] as string, parts[1] as string];
    });

  it.each(rows)("%j は %s", (endpoint, expected) => {
    expect(endpointEndsWithV1(endpoint) ? "v1" : "ok").toBe(expected);
  });

  /** **読めていないから緑、を通さない**（表が痩せていない・2つの分類が両方出る）。 */
  it("表は痩せておらず、2つの分類が両方出る", () => {
    expect(rows.length).toBeGreaterThanOrEqual(30);
    expect([...new Set(rows.map(([, expected]) => expected))].sort()).toEqual(["ok", "v1"]);
  });

  const MESSAGE =
    "保存しませんでした: 末尾の /v1 は書かないでください（personacore が /v1 を足すので、/v1/v1/… に送ることになります）。ホストとポートまで書いてください（例 http://127.0.0.1:8080）。";

  /** **起こす面にも効く**（起こした llama へも、宛先に `/v1/…` を足して送る）。 */
  it("起こす面の宛先が /v1 で終わるなら、その面の endpoint の欄に結びつけて止める", () => {
    const draft = draftOfProfile(ALPHA);
    draft.main.endpoint = "http://127.0.0.1:41001/v1/";
    expect(endpointV1Failure(draft)).toEqual({ kind: "field", face: "main", key: "endpoint", message: MESSAGE });
  });

  /** **裏方は、別の上流にするときだけ見る**。止めるのは悪い面だけ（本文が正しく裏方が悪い → 裏方）。両方なら本文を先に言う。 */
  it("繋ぐ面でも止める。裏方は別にするときだけ見て、両方なら本文を先に言う", () => {
    const draft = draftOfProfile(ALPHA);
    draft.kind.sub = "connect";
    draft.sub.endpoint = "https://example.com/api/V1";
    draft.hasSub = false;
    expect(endpointV1Failure(draft)).toBeNull();
    draft.hasSub = true;
    expect(endpointV1Failure(draft)).toEqual({ kind: "field", face: "sub", key: "endpoint", message: MESSAGE });
    draft.main.endpoint = "http://127.0.0.1:41001/v1";
    expect(endpointV1Failure(draft)).toMatchObject({ face: "main", key: "endpoint" });
  });

  it("対照: /v1beta・パスの前置き・末尾の / だけ・ホストとポートだけは止めない", () => {
    for (const endpoint of [
      "https://example.com/v1beta",
      "https://example.com/api",
      "http://127.0.0.1:41001/",
      "http://127.0.0.1:41001",
    ]) {
      const draft = draftOfProfile(ALPHA);
      draft.main.endpoint = endpoint;
      draft.sub.endpoint = endpoint;
      expect(endpointV1Failure(draft), endpoint).toBeNull();
    }
  });
});

/**
 * **一覧の面を、依頼の面の形にする**（作業単位 A-13b-1）。
 *
 * 応答の `api_key_set` / `api_key_hint` は依頼に無い（送ると `deny_unknown_fields` で 400）。
 * **`api_key` も `allow_key_over_plain_http` も足さない**（触らない鍵と承認は送らない＝残す・今の規則 / 作業単位 A-13b-3）。
 */
function requested(face: WrittenFace): WrittenFaceRequest {
  const { api_key_set: _set, api_key_hint: _hint, allow_key_over_plain_http: _approved, ...rest } = face;
  return rest;
}

describe("下書きと、送る本文", () => {
  /**
   * **一覧 → 下書き → 本文 が、書いたままの値に戻る。**
   * 保存した後に読み直した一覧が、送った内容と一致するための前提。
   */
  it.each([ALPHA, BETA, GAMMA])("`$name` は、下書きを経て同じ値に戻る", (profile) => {
    const request = buildSaveRequest(draftOfProfile(profile), "rev-1");
    expect(request).toEqual({
      revision: "rev-1",
      main: requested(profile.main),
      sub: profile.sub === null ? null : requested(profile.sub),
    });
  });

  /**
   * **鍵を書いた面を直しても、依頼に `api_key` を載せない**（ADR-0056 §決定3 / 作業単位 A-13b-1）。
   *
   * サーバは「書いていない＝残す / `null`＝消す」で読むので、**`api_key: null` を1つ載せるだけで鍵が消える。**
   * 画面は鍵の値を持たないので、**鍵の欄に触らなければ鍵ごと送らない**（承認も送らない）。
   *
   * > 作業単位 A-13b-3 でフィクスチャを繋ぐ面に替えた（一時、起こす面の鍵を `null` で送っていたため）。
   * > レビュー是正で鍵は両方の種類の欄に戻り、**起こす面の鍵も残す**（その腕は `tests/upstream-profiles-key-draft.test.ts` の
   * > 「起こす面の鍵は、ほかの欄を直しても・種類を切り替えても残り…」）。
   */
  it("鍵を書いた面を直しても、依頼に `api_key` を載せない（＝鍵を残す）", () => {
    const keyed: WrittenProfile = {
      ...BETA,
      main: { ...(BETA.sub as WrittenFace), api_key_set: true, api_key_hint: "9z9z", allow_key_over_plain_http: true },
      sub: { ...GAMMA.main, api_key_set: true, api_key_hint: null },
    };
    const draft = draftOfProfile(keyed);
    expect([draft.kind.main, draft.kind.sub]).toEqual(["connect", "connect"]);
    const request = buildSaveRequest(draft, "rev-1");
    for (const face of [request.main, request.sub]) {
      expect(face).not.toBeNull();
      expect(Object.keys(face ?? {})).not.toContain("api_key");
      expect(Object.keys(face ?? {})).not.toContain("api_key_set");
      expect(Object.keys(face ?? {})).not.toContain("api_key_hint");
      expect(Object.keys(face ?? {})).not.toContain("allow_key_over_plain_http");
    }
  });

  it("印が無い（ファイルが無い）なら、`null` のまま送る", () => {
    expect(buildSaveRequest(draftOfProfile(GAMMA), null).revision).toBeNull();
  });

  it("空の欄は「書かない」として送る（`endpoint` / `model` は空でもそのまま）", () => {
    const draft = emptyProfileDraft();
    expect(buildSaveRequest(draft, null)).toEqual({
      revision: null,
      main: { endpoint: "", model: "", gguf: null, args: null, command: null, context_tokens: null },
      sub: null,
    });
  });

  /** **裏方を書かないと決めたら、下書きに残っている値は送らない。** */
  it("裏方を別にしないなら、裏方の欄の値が残っていても送らない", () => {
    const draft = draftOfProfile(ALPHA);
    draft.hasSub = false;
    expect(buildSaveRequest(draft, "r").sub).toBeNull();
    // 対照: 戻せば送る。
    draft.hasSub = true;
    expect(buildSaveRequest(draft, "r").sub).toEqual(requested(ALPHA.sub as WrittenFace));
  });

  it("引数は1行に1つ。空行は落とし、行の中の空白はそのまま", () => {
    expect(argsOfText("--ctx-size\n4096")).toEqual(["--ctx-size", "4096"]);
    expect(argsOfText("--ctx-size\r\n4096\r\n")).toEqual(["--ctx-size", "4096"]);
    expect(argsOfText("--alias\ntwo words\n\n  \n")).toEqual(["--alias", "two words"]);
    expect(argsOfText("")).toBeNull();
    expect(argsOfText("\n \n")).toBeNull();
  });

  it("下書きは、面ごとの値をその面の欄に持つ", () => {
    const draft = draftOfProfile(ALPHA);
    expect(draft.name).toBe("alpha");
    expect(draft.hasSub).toBe(true);
    expect(draft.main).toEqual({
      endpoint: "http://127.0.0.1:41001",
      model: "alpha-main",
      gguf: "~/models/alpha-main.gguf",
      args: "--ctx-size\n4096",
      command: "C:/tools/llama-a.exe",
      context_tokens: "28672",
      // **新しい鍵の欄は、開いたときは必ず空**（画面は鍵の値を持たない / 作業単位 A-13b-3）。
      api_key: "",
    });
    expect(draft.sub).toEqual({
      endpoint: "http://127.0.0.1:41002",
      model: "alpha-sub",
      gguf: "~/models/alpha-sub.gguf",
      args: "-ngl\n10",
      command: "C:/tools/llama-b.exe",
      context_tokens: "",
      api_key: "",
    });
    expect(draftOfProfile(GAMMA).hasSub).toBe(false);
  });

  /**
   * **コンテキスト長の欄は、数字の文字列を整数で送り、空なら書かない**（作業単位 A-13a-2）。
   *
   * **前後の空白は落とす**（数として読むので、空白に意味は無い）。
   * **値の範囲（応答の余白より大きいか）はここでは見ない** —— 決めるのはサーバの読み込みの検証で、
   * 理由は 422 でその欄に返る（規則の写しを画面に作らない）。
   */
  it("コンテキスト長の欄は、整数で送り、空なら書かない（範囲はサーバが決める）", () => {
    const draft = draftOfProfile(BETA);
    draft.main.context_tokens = " 20480 ";
    draft.sub.context_tokens = "";
    const request = buildSaveRequest(draft, "r");
    expect([request.main.context_tokens, request.sub?.context_tokens]).toEqual([20480, null]);

    // **空白だけも「書かない」。** 範囲外（余白ちょうど）でも、整数なら送る。
    draft.main.context_tokens = "   ";
    draft.sub.context_tokens = "1024";
    const again = buildSaveRequest(draft, "r");
    expect([again.main.context_tokens, again.sub?.context_tokens]).toEqual([null, 1024]);
    expect(contextTokensFailure(draft)).toBeNull();
  });

  it("足すときに、在る名前は取らせない（黙って上書きしない）", () => {
    const listed = [ALPHA, BETA, GAMMA];
    expect(nameTaken("beta", listed)).toBe(true);
    expect(nameTaken("delta", listed)).toBe(false);
    // **綴りが違えば別の名前**（`--upstream` は綴りで選ぶ）。
    expect(nameTaken("Beta", listed)).toBe(false);
  });
});

/**
 * **旗と値を1行に書いた引数を、送る前に止める**（作業単位 D-13 / Issue origin#29）。
 *
 * 1行は引数1つとして渡るので、`--ctx-size 16384` は**空白を含んだ1つの引数**になり、
 * `llama` が `error: invalid argument: --ctx-size 16384` で起動直後に終了する。
 *
 * **判定は「`--` で始まり、空白を含む行」だけ。** 空白を含む値そのもの（パス）は正当。
 *
 * **期待値の文面は直書き**（`upstreamProfiles.ts` の定数から作らない）。
 */
describe("旗と値を1行に書いた引数", () => {
  function draftWith(mainArgs: string, subArgs: string, hasSub: boolean) {
    const draft = draftOfProfile(ALPHA);
    draft.main.args = mainArgs;
    draft.sub.args = subArgs;
    draft.hasSub = hasSub;
    return draft;
  }

  it("本文の面の `--ctx-size 16384` は、本文の引数の欄に結びつけて止める", () => {
    expect(joinedFlagFailure(draftWith("--ctx-size 16384\n--jinja", "-ngl\n10", true))).toEqual({
      kind: "field",
      face: "main",
      key: "args",
      message:
        "保存しませんでした: 「--ctx-size 16384」は、空白ごと1つの引数として渡るので、上流が起動しません。--ctx-size と 16384 は別の行に書いてください。",
    });
  });

  /** **3行の真ん中に置く**（先頭・末尾だけを見る変異を通さない）。 */
  it("裏方の面の真ん中の行でも、裏方の引数の欄に結びつけて止める", () => {
    expect(
      joinedFlagFailure(draftWith("--ctx-size\n4096", "-ngl\n--batch-size 512\n--jinja", true)),
    ).toEqual({
      kind: "field",
      face: "sub",
      key: "args",
      message:
        "保存しませんでした: 「--batch-size 512」は、空白ごと1つの引数として渡るので、上流が起動しません。--batch-size と 512 は別の行に書いてください。",
    });
  });

  it("両方の面にあれば、本文の面を先に言う", () => {
    const failure = joinedFlagFailure(draftWith("--threads 6", "--batch-size 512", true));
    expect(failure?.kind === "field" ? failure.face : null).toBe("main");
  });

  /** **裏方を別にしないなら、裏方の欄は送らない**ので、そこに何が残っていても止めない。 */
  it("裏方を別にしないなら、裏方の欄に残った行では止めない", () => {
    expect(joinedFlagFailure(draftWith("--ctx-size\n4096", "--batch-size 512", false))).toBeNull();
    // 対照: 裏方を別にすれば止める。
    expect(joinedFlagFailure(draftWith("--ctx-size\n4096", "--batch-size 512", true))).not.toBeNull();
  });

  it("全角の空白も空白として数える", () => {
    const failure = joinedFlagFailure(draftWith("--ctx-size\u{3000}16384", "", false));
    expect(failure?.message).toBe(
      "保存しませんでした: 「--ctx-size\u{3000}16384」は、空白ごと1つの引数として渡るので、上流が起動しません。--ctx-size と 16384 は別の行に書いてください。",
    );
  });

  it("旗の後ろに空白だけが残っている行は、空白を消すように言う", () => {
    expect(joinedFlagFailure(draftWith("--jinja \n--ctx-size\n4096", "", false))?.message).toBe(
      "保存しませんでした: 「--jinja」の後ろに空白があります。空白ごと1つの引数として渡るので、上流が起動しません。空白を消してください。",
    );
  });

  /**
   * **止めないもの。** どれも1行が正当な1つの引数でありうる。
   *
   * * 空白を含む値（パス）—— `-` で始まらない
   * * `--alias=two words` —— 旗と値を `=` でつないだ1つの引数（空白は値の中）
   * * `-1` —— `-` で始まるが空白を含まない（負の値）
   * * 空の行・空白だけの行 —— `argsOfText` が落とす
   */
  it("空白を含む値・`=` でつないだ旗・空白の無い行は止めない", () => {
    expect(
      joinedFlagFailure(
        draftWith(
          "--chat-template-file\nC:\\Program Files\\llama\\t.jinja\n--alias=two words\n--seed\n-1\n\n  \n--jinja",
          "--model\nD:\\my models\\a b.gguf",
          true,
        ),
      ),
    ).toBeNull();
  });

  it("タブも空白として数える", () => {
    expect(joinedFlagFailure(draftWith("--ctx-size\t16384", "", false))?.message).toBe(
      "保存しませんでした: 「--ctx-size\t16384」は、空白ごと1つの引数として渡るので、上流が起動しません。--ctx-size と 16384 は別の行に書いてください。",
    );
  });

  /** **`-` 1つの短いオプションも同じ理由で起きない**（`-ngl 99` / `-c 16384`）。 */
  it("`-` 1つの短いオプションと値を1行に書いた行も止める", () => {
    expect(joinedFlagFailure(draftWith("--jinja\n-ngl 99", "", false))?.message).toBe(
      "保存しませんでした: 「-ngl 99」は、空白ごと1つの引数として渡るので、上流が起動しません。-ngl と 99 は別の行に書いてください。",
    );
    expect(joinedFlagFailure(draftWith("-c 16384", "", false))?.message).toBe(
      "保存しませんでした: 「-c 16384」は、空白ごと1つの引数として渡るので、上流が起動しません。-c と 16384 は別の行に書いてください。",
    );
  });

  /**
   * **行頭の空白も、空白ごと1つの引数として渡る**（`argsOfText` もサーバも行を刈らない）。
   * 読み飛ばしてから判定し、行頭の空白があることも言う。
   */
  it.each([
    [" --ctx-size 16384", "ASCII の空白"],
    ["\u{3000}--ctx-size 16384", "全角の空白"],
    ["\t--ctx-size 16384", "タブ"],
  ])("行頭に空白がある `%s`（%s）も止め、行頭の空白も消すように言う", (line) => {
    expect(joinedFlagFailure(draftWith(line, "", false))?.message).toBe(
      "保存しませんでした: 「--ctx-size 16384」は、空白ごと1つの引数として渡るので、上流が起動しません。--ctx-size と 16384 は別の行に書いてください。行の先頭の空白も消してください。",
    );
  });

  it("行頭の空白だけ・前後の空白だけの行は、空白を消すように言う", () => {
    expect(joinedFlagFailure(draftWith(" --jinja", "", false))?.message).toBe(
      "保存しませんでした: 「--jinja」の前に空白があります。空白ごと1つの引数として渡るので、上流が起動しません。空白を消してください。",
    );
    expect(joinedFlagFailure(draftWith(" -ngl ", "", false))?.message).toBe(
      "保存しませんでした: 「-ngl」の前後に空白があります。空白ごと1つの引数として渡るので、上流が起動しません。空白を消してください。",
    );
    // **`=` でつないだ行でも、行頭の空白は同じ理由で起きない。**
    expect(joinedFlagFailure(draftWith(" --alias=two words", "", false))?.message).toBe(
      "保存しませんでした: 「--alias=two words」の前に空白があります。空白ごと1つの引数として渡るので、上流が起動しません。空白を消してください。",
    );
  });

  /** **対照: `-` で始まらない行は、行頭に空白があってもこの判定の外**（値の行。報告に書いてある）。 */
  it("対照: `-` で始まらない行は、行頭に空白があっても止めない", () => {
    expect(joinedFlagFailure(draftWith("--model\n C:\\my models\\a.gguf", "", false))).toBeNull();
  });

  it("引数が1つも無ければ止めない", () => {
    expect(joinedFlagFailure(emptyProfileDraft())).toBeNull();
  });
});

/**
 * **数として読めないコンテキスト長は、送らずにその欄で言う**（作業単位 A-13a-2）。
 *
 * サーバの依頼の型は整数なので、数字でない値を送ると本文として読めない 400 になり、
 * **どの欄が悪いかが画面に届かない。** 送る前に止める（旗と値を1行に書いた引数と同じ流れ）。
 * **期待値の文面は直書き。**
 */
describe("コンテキスト長の欄", () => {
  const NOT_DIGITS =
    "保存しませんでした: コンテキスト長は、半角の数字だけで書いてください（空にすると書きません）。";
  const TOO_LARGE = "保存しませんでした: コンテキスト長が大きすぎます。";

  function draftWith(main: string, sub: string, hasSub: boolean) {
    const draft = draftOfProfile(ALPHA);
    draft.main.context_tokens = main;
    draft.sub.context_tokens = sub;
    draft.hasSub = hasSub;
    return draft;
  }

  it.each(["32k", "1.5", "-1", "1e5", "２００００", "0x100", "20 480"])(
    "`%s` は本文の欄に結びつけて止める",
    (raw) => {
      expect(contextTokensFailure(draftWith(raw, "", false))).toEqual({
        kind: "field",
        face: "main",
        key: "context_tokens",
        message: NOT_DIGITS,
      });
    },
  );

  it("裏方の欄なら、裏方の欄に結びつける。両方なら本文を先に言う", () => {
    expect(contextTokensFailure(draftWith("28672", "abc", true))).toEqual({
      kind: "field",
      face: "sub",
      key: "context_tokens",
      message: NOT_DIGITS,
    });
    const both = contextTokensFailure(draftWith("x", "y", true));
    expect(both?.kind === "field" ? both.face : null).toBe("main");
  });

  /** **裏方を別にしないなら、裏方の欄は送らない**ので見ない。 */
  it("裏方を別にしないなら、裏方の欄に残った値では止めない", () => {
    expect(contextTokensFailure(draftWith("28672", "abc", false))).toBeNull();
  });

  /** **数字だけでも、正確に送れない桁数なら止める**（丸めた値を送らない）。 */
  it("数字だけでも、正確に送れないほど大きければ止める", () => {
    expect(contextTokensFailure(draftWith("99999999999999999999", "", false))).toEqual({
      kind: "field",
      face: "main",
      key: "context_tokens",
      message: TOO_LARGE,
    });
  });

  /**
   * **境界の対**（A-13a-2 のレビュー是正）: 2^53−1 は正確に送れるので通し、2^53 は止める。
   * 境界を1つ外側へずらす実装（`value <= 2 ** 53`）を赤にする。
   */
  it("2^53−1 は送り、2^53 は大きすぎると止める", () => {
    const fits = draftWith("9007199254740991", "", false);
    expect(contextTokensFailure(fits)).toBeNull();
    expect(buildSaveRequest(fits, "r").main.context_tokens).toBe(9007199254740991);
    expect(contextTokensFailure(draftWith("9007199254740992", "", false))).toEqual({
      kind: "field",
      face: "main",
      key: "context_tokens",
      message: TOO_LARGE,
    });
  });

  it("対照: 数字・前後の空白・空は止めない", () => {
    expect(contextTokensFailure(draftWith(" 28672 ", "10240", true))).toBeNull();
    expect(contextTokensFailure(draftWith("", "  ", true))).toBeNull();
  });
});

/** サーバの 422 `upstream_profile_invalid`（`profile_editor.rs` の `SaveError::Invalid`）。 */
function invalid(face: string | null, key: string | null): ApiRequestError {
  return new ApiRequestError(
    "上流プロファイルの形が不正なので保存しませんでした",
    422,
    "invalid_request_error",
    "upstream_profile_invalid",
    {
      reason: "endpoint_not_loopback",
      reason_message: "gguf を書いた面の endpoint はループバック（127.0.0.1 / localhost / ::1）だけです",
      face,
      key,
    },
  );
}

describe("保存の失敗を読み分ける", () => {
  it("422 は、止まった面と鍵に結びつける（理由の文面ごと）", () => {
    expect(describeSaveFailure(invalid("sub", "endpoint"), true)).toEqual({
      kind: "field",
      face: "sub",
      key: "endpoint",
      message:
        "保存しませんでした: gguf を書いた面の endpoint はループバック（127.0.0.1 / localhost / ::1）だけです",
    });
    expect(describeSaveFailure(invalid("main", "args"), true)).toMatchObject({
      kind: "field",
      face: "main",
      key: "args",
    });
    // **コンテキスト長の 422 も、その欄に結びつく**（作業単位 A-13a-2 で鍵が `context_tokens` になった）。
    expect(describeSaveFailure(invalid("sub", "context_tokens"), true)).toMatchObject({
      kind: "field",
      face: "sub",
      key: "context_tokens",
    });
  });

  it("面か鍵が分からない 422 は、欄ではなく全体に出す", () => {
    for (const [face, key] of [
      [null, null],
      ["main", null],
      [null, "gguf"],
      ["third", "gguf"],
      ["main", "ggml"],
    ] as const) {
      expect(describeSaveFailure(invalid(face, key), true)).toEqual({
        kind: "form",
        message:
          "保存しませんでした: gguf を書いた面の endpoint はループバック（127.0.0.1 / localhost / ::1）だけです",
      });
    }
  });

  it("裏方を書いていない下書きで裏方の欄を言われたら、全体に出す（欄が画面に無い）", () => {
    expect(describeSaveFailure(invalid("sub", "model"), false).kind).toBe("form");
  });

  it("欄の無い鍵（`allow_key_over_plain_http` / `sampling` / `response_tokens`）を名指した 422 は、全体に出す（ADR-0058 §決定6 / ADR-0059 §決定5）", () => {
    for (const key of ["allow_key_over_plain_http", "sampling", "response_tokens"] as const) {
      // **前提: 3つとも写し元の鍵ではある**（知らない鍵だから全体に出た、ではない）。
      expect(FACE_KEYS as readonly string[]).toContain(key);
      expect(EDITABLE_FACE_KEYS as readonly string[]).not.toContain(key);
      for (const face of ["main", "sub"] as const) {
        expect(describeSaveFailure(invalid(face, key), true).kind, `${face} / ${key}`).toBe("form");
      }
    }
  });

  it("409 `upstream_profiles_changed` は、上書きしていないことと、読み直すことを言う", () => {
    const failure = describeSaveFailure(
      new ApiRequestError("x", 409, "invalid_request_error", "upstream_profiles_changed"),
      true,
    );
    expect(failure).toEqual({
      kind: "changed",
      message:
        "upstreams.json は、この画面が読んだ後に外で書き換えられています。上書きはしていません。読み直してから、もう一度直してください。",
    });
  });

  it("名前が使えない 400 は、名前の欄に結びつける（サーバの文面で）", () => {
    expect(
      describeSaveFailure(
        new ApiRequestError("その名前は使えません", 400, "invalid_request_error", "upstream_profile_name_invalid"),
        true,
      ),
    ).toEqual({ kind: "name", message: "その名前は使えません" });
  });

  it("それ以外はサーバの文面を全体に出す。投げたものが分からなければ、そう言う", () => {
    expect(
      describeSaveFailure(
        new ApiRequestError("読むだけです", 409, "invalid_request_error", "upstream_profiles_read_only"),
        true,
      ),
    ).toEqual({ kind: "form", message: "読むだけです" });
    expect(describeSaveFailure(new Error("boom"), true)).toEqual({
      kind: "form",
      message: "予期しない失敗です",
    });
  });
});

/** 状態の組（`LaunchView`）。 */
function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

/** 状態の文面の「いま」と時間帯（下の節の既定。**`since` より 12 秒後、東京**）。 */
const NOW = new Date("2026-09-25T01:00:12Z");
const ZONE = "Asia/Tokyo";

describe("載せている最中の面の時刻（Issue origin#40）", () => {
  /** サーバの実際の綴り（小数部 9 桁・時差つき）。 */
  const REAL = "2026-09-26T13:25:15.770371400+09:00";
  const loading = (since: string | null): LaunchView => launch({ state: "loading", since });

  it("サーバの実際の綴りを、読み手の時刻と経過で言う", () => {
    expect(launchLabel(loading(REAL), new Date("2026-09-26T04:25:27.770Z"), "Asia/Tokyo")).toBe(
      "載せています（13:25:15 から、12 秒経過）",
    );
  });

  it("秒に満たない端数は切り捨てる（経過を先回りしない）", () => {
    expect(launchLabel(loading(REAL), new Date("2026-09-26T04:25:28.500Z"), "Asia/Tokyo")).toBe(
      "載せています（13:25:15 から、12 秒経過）",
    );
  });

  it("`Z` 終わりも読む", () => {
    expect(launchLabel(loading("2026-09-26T04:25:15Z"), new Date("2026-09-26T04:25:20Z"), "Asia/Tokyo")).toBe(
      "載せています（13:25:15 から、5 秒経過）",
    );
  });

  it("同じ瞬間が、東京とニューヨークで違う時刻になる", () => {
    const now = new Date("2026-09-26T04:25:18Z");
    expect([
      launchLabel(loading(REAL), now, "Asia/Tokyo"),
      launchLabel(loading(REAL), now, "America/New_York"),
    ]).toEqual(["載せています（13:25:15 から、2 秒経過）", "載せています（00:25:15 から、2 秒経過）"]);
  });

  it("59 秒と 60 秒の境目で、分に繰り上がる", () => {
    const since = "2026-09-26T04:00:00Z";
    expect([
      launchLabel(loading(since), new Date("2026-09-26T04:00:59Z"), "UTC"),
      launchLabel(loading(since), new Date("2026-09-26T04:01:00Z"), "UTC"),
      launchLabel(loading(since), new Date("2026-09-26T04:02:05Z"), "UTC"),
      launchLabel(loading(since), new Date("2026-09-26T04:00:00Z"), "UTC"),
    ]).toEqual([
      "載せています（04:00:00 から、59 秒経過）",
      "載せています（04:00:00 から、1 分経過）",
      "載せています（04:00:00 から、2 分 5 秒経過）",
      "載せています（04:00:00 から、0 秒経過）",
    ]);
  });

  it("60 分で時間の桁が付き、0 の桁は省く", () => {
    const since = "2026-09-26T04:00:00Z";
    const at = (seconds: number): string =>
      launchLabel(loading(since), new Date(Date.parse(since) + seconds * 1000), "UTC");
    expect([at(3599), at(3600), at(3603), at(4503), at(7200), at(3660)]).toEqual([
      "載せています（04:00:00 から、59 分 59 秒経過）",
      "載せています（04:00:00 から、1 時間経過）",
      "載せています（04:00:00 から、1 時間 3 秒経過）",
      "載せています（04:00:00 から、1 時間 15 分 3 秒経過）",
      "載せています（04:00:00 から、2 時間経過）",
      "載せています（04:00:00 から、1 時間 1 分経過）",
    ]);
  });

  it("西側の時差（`-04:00`）も読む", () => {
    expect(
      launchLabel(
        loading("2026-09-26T00:25:15.770371400-04:00"),
        new Date("2026-09-26T04:25:27.770Z"),
        "Asia/Tokyo",
      ),
    ).toBe("載せています（13:25:15 から、12 秒経過）");
  });

  it("経過が負なら（時計のずれ）、時刻だけ言う", () => {
    expect(launchLabel(loading(REAL), new Date("2026-09-26T04:25:14Z"), "Asia/Tokyo")).toBe(
      "載せています（13:25:15 から）",
    );
  });

  it("読めない綴りは、生の綴りのまま出す", () => {
    expect(
      ["yesterday", "2026-09-26T13:25:15", "2026-13-45T99:00:00Z", "2026-09-26T13:25:15+0900"].map((since) =>
        launchLabel(loading(since), NOW, ZONE),
      ),
    ).toEqual([
      "載せています（yesterday から）",
      "載せています（2026-09-26T13:25:15 から）",
      "載せています（2026-13-45T99:00:00Z から）",
      "載せています（2026-09-26T13:25:15+0900 から）",
    ]);
  });

  it("読めない時間帯なら、生の綴りのまま出す", () => {
    expect(launchLabel(loading(REAL), NOW, "Not/AZone")).toBe(`載せています（${REAL} から）`);
  });

  it("`since` が `null` なら、状態だけ", () => {
    expect(launchLabel(loading(null), NOW, ZONE)).toBe("載せています");
  });
});

describe("起こした上流の状態", () => {
  /**
   * **`not_started` は作業単位 F-10b で文面を決めた**（ADR-0055 §決定3。F-10a は仮の文面だった）。
   * **`external` と同じ「起こしていません」で始めない** —— 外の上流は起こす物が無く、
   * こちらは起こせば起きる（起こすボタンがある）。読み手が2つを取り違えないように。
   */
  it("6つの状態を、6つとも別の文面で言う", () => {
    const labels = [
      launchLabel(launch({ state: "external" }), NOW, ZONE),
      launchLabel(launch({ state: "not_started" }), NOW, ZONE),
      launchLabel(launch({ state: "loading", since: "2026-09-25T10:00:00+09:00" }), NOW, ZONE),
      launchLabel(launch({ state: "ready", answered_name: "alpha-main.gguf" }), NOW, ZONE),
      launchLabel(launch({ state: "failed", reason: "子が自分で終了しました", log: "main-20260925-100000.err.log" }), NOW, ZONE),
      launchLabel(launch({ state: "stopped" }), NOW, ZONE),
    ];
    expect(labels).toEqual([
      "起こしていません（外の上流に依頼しています）",
      "まだ起こしていません（前回選んだプロファイルは、起動しただけでは起こしません）",
      "載せています（10:00:00 から、12 秒経過）",
      "準備完了（上流が名乗った名前: alpha-main.gguf）",
      "失敗しました: 子が自分で終了しました（ログ: upstream-logs の main-20260925-100000.err.log）",
      "止めました",
    ]);
  });

  it("欄が `null` でも、状態だけは言う", () => {
    expect(launchLabel(launch({ state: "loading" }), NOW, ZONE)).toBe("載せています");
    expect(launchLabel(launch({ state: "ready" }), NOW, ZONE)).toBe("準備完了");
    expect(launchLabel(launch({ state: "failed" }), NOW, ZONE)).toBe("失敗しました（理由は届いていません）");
  });

  it("知らない状態は、そのまま出す（欄を消さない）", () => {
    expect(launchLabel(launch({ state: "sleeping" }), NOW, ZONE)).toBe("sleeping");
  });

  it("どちらかの面が載せている最中かを見る", () => {
    const report = (main: string, sub: string): UpstreamReport =>
      ({
        main: { launch: launch({ state: main }) },
        sub: { launch: launch({ state: sub }) },
      }) as unknown as UpstreamReport;
    expect(hasLoadingFace(report("loading", "ready"))).toBe(true);
    expect(hasLoadingFace(report("ready", "loading"))).toBe(true);
    expect(hasLoadingFace(report("ready", "failed"))).toBe(false);
    expect(hasLoadingFace(report("external", "external"))).toBe(false);
    // **起こすのを待っている面は、載せている最中ではない**（読み直し続けない / 作業単位 F-10b）。
    expect(hasLoadingFace(report("not_started", "not_started"))).toBe(false);
  });
});

/**
 * **起こすボタンが送る名前**（作業単位 F-10b / ADR-0055 §決定3）。
 *
 * **本文の面が `not_started` のときだけ、いま選ばれているプロファイルの名前を返す。**
 * **裏方の面の状態では出さない**（ADR-0055 §決定3・ADR-0042 §決定3 の追記）。
 * 名前が無い（選んでいない）なら出さない —— 無い名前を頼ませない。
 */
describe("起こすボタンが送る名前", () => {
  const report = (main: string, sub: string, selected: string | null): UpstreamReport =>
    ({
      main: { launch: launch({ state: main }) },
      sub: { launch: launch({ state: sub }) },
      profile_sample: "upstreams.json",
      selected_profile: selected,
    }) as unknown as UpstreamReport;

  it.each([
    { main: "not_started", sub: "not_started", selected: "beta", expected: "beta" },
    { main: "not_started", sub: "external", selected: "beta", expected: "beta" },
    // **名前が無ければ出さない**（見本の場所を名前として送らない）。
    { main: "not_started", sub: "external", selected: null, expected: null },
    // **裏方だけ待っている → 出さない**（本文の面の状態で決める）。
    { main: "external", sub: "not_started", selected: "beta", expected: null },
    { main: "ready", sub: "not_started", selected: "beta", expected: null },
    // **対照: not_started 以外の本文の面では出さない。**
    { main: "external", sub: "external", selected: "beta", expected: null },
    { main: "loading", sub: "loading", selected: "beta", expected: null },
    { main: "ready", sub: "ready", selected: "beta", expected: null },
    { main: "failed", sub: "ready", selected: "beta", expected: null },
    { main: "stopped", sub: "stopped", selected: "beta", expected: null },
    { main: "sleeping", sub: "ready", selected: "beta", expected: null },
  ])("本文 $main / 裏方 $sub / 選んだ名前 $selected → $expected", ({ main, sub, selected, expected }) => {
    expect(profileToStart(report(main, sub, selected))).toBe(expected);
  });
});

/** 一覧の応答（`GET /admin/upstream/profiles`）。 */
function listing(overrides: Partial<ProfilesView>): ProfilesView {
  return {
    path: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
    editable: true,
    state: "ok",
    revision: "rev",
    problem: null,
    profiles: [ALPHA, BETA, GAMMA],
    ...overrides,
  };
}

describe("一覧の状態を言う", () => {
  const PATH = "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json";
  const HAND = "画面からは直せません。ファイルを手で直してから、読み直してください。";

  it("直せる置き場所では、書く先を言う", () => {
    expect(listingNotices(listing({}))).toEqual([`書く先: ${PATH}`]);
    expect(canEdit(listing({}))).toBe(true);
  });

  it("読むだけの置き場所では、読んでいるファイルと、ここからは直せないことを言う", () => {
    const view = listing({ editable: false, path: "F:/repo/scripts/upstreams.json" });
    expect(listingNotices(view)).toEqual([
      "読んでいるファイル: F:/repo/scripts/upstreams.json",
      "この起動では、ここからは直せません（読むだけです）。起動するときに PERSONACORE_UPSTREAMS が別のファイルを指しているためです。",
    ]);
    expect(canEdit(view)).toBe(false);
  });

  it("ファイルが無ければ、足すとそこに作ると言う（直せる置き場所のときだけ）", () => {
    const missing = listing({ state: "missing", revision: null, profiles: [] });
    expect(listingNotices(missing)).toEqual([
      `書く先: ${PATH}`,
      "まだファイルがありません。プロファイルを足して保存すると、この場所に作ります。",
    ]);
    expect(canEdit(missing)).toBe(true);
    const readOnly = listing({ state: "missing", editable: false, revision: null, profiles: [] });
    expect(listingNotices(readOnly)).toContain("ファイルがありません。");
    expect(canEdit(readOnly)).toBe(false);
  });

  it("検証に通らないファイルは、直せないことと理由と場所を言う", () => {
    const view = listing({
      state: "invalid",
      profiles: [],
      problem: {
        reason: "unknown_key",
        message:
          "知らない鍵があります（使えるのは endpoint / model / gguf / args / command / context_tokens だけです）",
        profile: "beta",
        face: "main",
        key: null,
      },
    });
    expect(listingNotices(view)).toEqual([
      `書く先: ${PATH}`,
      "ファイルが読み込みの検証に通りません: 知らない鍵があります（使えるのは endpoint / model / gguf / args / command / context_tokens だけです）（プロファイル「beta」の main）",
      HAND,
    ]);
    expect(canEdit(view)).toBe(false);
  });

  it("読めないファイルは、直せないことと理由を言う", () => {
    const view = listing({
      state: "unreadable",
      revision: null,
      profiles: [],
      problem: {
        reason: "unreadable",
        message: "上流プロファイルを読めませんでした（置き場所の権限か文字コードを確かめてください）",
        profile: null,
        face: null,
        key: null,
      },
    });
    expect(listingNotices(view)).toEqual([
      `書く先: ${PATH}`,
      "ファイルを読めませんでした: 上流プロファイルを読めませんでした（置き場所の権限か文字コードを確かめてください）",
      HAND,
    ]);
    expect(canEdit(view)).toBe(false);
  });

  it("置き場所が無い起動では、選ぶことも直すこともできないと言う", () => {
    const view = listing({ state: "none", path: null, editable: false, revision: null, profiles: [] });
    expect(listingNotices(view)).toEqual([
      "この起動には、上流プロファイルの置き場所がありません。画面からは選ぶことも直すこともできません。",
    ]);
    expect(canEdit(view)).toBe(false);
  });

  it("読めたが1つも無ければ、そう言う", () => {
    expect(listingNotices(listing({ profiles: [] }))).toEqual([
      `書く先: ${PATH}`,
      "プロファイルは1つも書かれていません。",
    ]);
  });

  it("形の違う応答は、一覧として扱わない", () => {
    expect(isProfilesView(listing({}))).toBe(true);
    expect(isProfilesView(listing({ profiles: [] }))).toBe(true);
    expect(isProfilesView({})).toBe(false);
    expect(isProfilesView(null)).toBe(false);
    expect(isProfilesView("not found")).toBe(false);
    expect(isProfilesView({ state: "ok" })).toBe(false);
    expect(isProfilesView({ state: 1, profiles: [] })).toBe(false);
    expect(isProfilesView({ state: "ok", profiles: {} })).toBe(false);
  });

  it("知らない状態は、そのまま言い、直させない", () => {
    const view = listing({ state: "sleeping" });
    expect(listingNotices(view)).toContain("知らない状態です: sleeping");
    expect(canEdit(view)).toBe(false);
  });
});

// ----------------------------------------------------------------------
// 写し元の綴り
// ----------------------------------------------------------------------

/**
 * **`pub const <名前>: [&str; N] = ["a", "b"];` の綴りを読む。**
 *
 * **`rustfmt` が折り返した形（1行に1要素・末尾のカンマ）も読む**（作業単位 A-13a-2 で
 * `FACE_KEYS` が6つになり、1行に収まらなくなった）。宣言の行から `];` の行までを1つにつなぐ。
 *
 * **読めなければ throw する**（探せていないことを緑にしない）。
 * **⚠ 改行は `\r` を落としてから見る**（作業ツリーは CRLF）。
 */
function stringArrayConstOf(source: string, name: string): string[] {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const starts = lines.flatMap((line, index) =>
    line.startsWith(`pub const ${name}: [&str; `) ? [index] : [],
  );
  if (starts.length !== 1 || starts[0] === undefined) {
    throw new Error(`const ${name} の宣言が ${starts.length} 個あります`);
  }
  const end = lines.findIndex((line, index) => index >= (starts[0] as number) && line.endsWith("];"));
  if (end < 0) {
    throw new Error(`const ${name} の終わり（];）が見つかりません`);
  }
  const joined = lines.slice(starts[0], end + 1).join(" ");
  const found = /= \[(.*)\];$/u.exec(joined);
  if (found?.[1] === undefined) {
    throw new Error(`const ${name} の値を読めません`);
  }
  // **末尾のカンマ**（折り返した形）の後ろの空の要素だけを落とす。
  const items = found[1].split(",");
  if (items.length > 1 && (items[items.length - 1] ?? "").trim() === "") {
    items.pop();
  }
  return items.map((item) => {
    const spelled = /^\s*"([^"]*)"\s*$/u.exec(item);
    if (spelled?.[1] === undefined) {
      throw new Error(`const ${name} の読めない要素: ${item}`);
    }
    return spelled[1];
  });
}

/** **`pub const <名前>: &str = "…";` の綴りを読む。** */
function stringConstOf(source: string, name: string): string {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const hits = lines.flatMap((line) => {
    const found = new RegExp(`^pub const ${name}: &str = "([^"]*)";$`, "u").exec(line);
    return found?.[1] === undefined ? [] : [found[1]];
  });
  if (hits.length !== 1 || hits[0] === undefined) {
    throw new Error(`const ${name} の宣言が ${hits.length} 個あります`);
  }
  return hits[0];
}

/**
 * **`fn label` の中の `Self::X … => "綴り",` を全部**（`{ .. }` 付きの腕も読む）。
 */
function faceStateLabelsOf(source: string): string[] {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const at = lines.findIndex((line) => line.trim().startsWith("pub const fn label(&self) -> &'static str {"));
  if (at < 0) {
    throw new Error("fn label が見つかりません");
  }
  const labels: string[] = [];
  for (let index = at + 1; index < lines.length; index += 1) {
    const line = (lines[index] ?? "").trim();
    if (line === "}") {
      break;
    }
    const found = /^Self::[A-Za-z]+(?: \{ \.\. \})? => "([^"]*)",$/u.exec(line);
    if (found?.[1] !== undefined) {
      labels.push(found[1]);
    }
  }
  if (labels.length === 0) {
    throw new Error("fn label に綴りを返す腕が1つもありません");
  }
  return labels;
}

/**
 * **`fn label` の腕を、変種の名前つきで**（`Self::Ready { .. } => "ready",` → `Ready: "ready"`）。
 *
 * **綴りの集合だけでは、どの綴りが「準備完了」かを言えない**（作業単位 D-16）。
 * 会話の画面は「`ready` なら送れる」「`loading` なら止める」を綴りで決めるので、
 * **2つの綴りを入れ替える変異は、集合の見張り番（上の [`faceStateLabelsOf`]）では緑になる。**
 */
function faceStateVariantsOf(source: string): Record<string, string> {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const at = lines.findIndex((line) => line.trim().startsWith("pub const fn label(&self) -> &'static str {"));
  if (at < 0) {
    throw new Error("fn label が見つかりません");
  }
  const variants: Record<string, string> = {};
  for (let index = at + 1; index < lines.length; index += 1) {
    const line = (lines[index] ?? "").trim();
    if (line === "}") {
      break;
    }
    const found = /^Self::([A-Za-z]+)(?: \{ \.\. \})? => "([^"]*)",$/u.exec(line);
    if (found?.[1] !== undefined && found[2] !== undefined) {
      variants[found[1]] = found[2];
    }
  }
  if (Object.keys(variants).length === 0) {
    throw new Error("fn label に綴りを返す腕が1つもありません");
  }
  return variants;
}

const UPSTREAMS_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/upstreams.rs", import.meta.url)),
  "utf8",
);
const LAUNCH_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/launch.rs", import.meta.url)),
  "utf8",
);

describe("写し元の綴り", () => {
  /** **ずれると、422 が欄に結びつかず、全体の一言に落ちる**（止まった場所が画面から消える）。 */
  it("面の綴りは `FACE_NAMES` と一致する", () => {
    expect([...FACES]).toEqual(stringArrayConstOf(UPSTREAMS_RS, "FACE_NAMES"));
  });

  it("鍵の綴りは `FACE_KEYS` と一致する（並びも）", () => {
    expect([...FACE_KEYS]).toEqual(stringArrayConstOf(UPSTREAMS_RS, "FACE_KEYS"));
  });

  /** **画面の注記が「書かなければ何を起こすか」を言うので、その綴り。** */
  it("既定の実行ファイル名は `DEFAULT_COMMAND` と一致する", () => {
    expect(DEFAULT_COMMAND).toBe(stringConstOf(UPSTREAMS_RS, "DEFAULT_COMMAND"));
  });

  /**
   * **サーバが返す状態の綴りを、画面は1つ残らず知っている。**
   * 知らない綴りはそのまま出す枝に落ちる——**「載せています」が `loading` として出る。**
   */
  it("`FaceState::label` の綴りは、どれも生のまま画面に出ない", () => {
    const spellings = faceStateLabelsOf(LAUNCH_RS);
    expect(spellings).toEqual(["external", "not_started", "loading", "ready", "failed", "stopped"]);
    for (const spelling of spellings) {
      expect(launchLabel(launch({ state: spelling }), NOW, ZONE), spelling).not.toBe(spelling);
    }
  });

  /**
   * **どの綴りがどの状態か**まで一致する（作業単位 D-16）。
   * **会話の画面が送信を止めるかをこの綴りで決める**ので、`ready` と `loading` の取り違えは
   * 「載っているのに送れない／載る前に送れる」になる。
   */
  it("状態の綴りは、変種ごとに `FaceState::label` と一致する", () => {
    expect({ ...LAUNCH_STATE }).toEqual(faceStateVariantsOf(LAUNCH_RS));
  });

  describe("対照: 植えたソースからは、その場の綴りを読む", () => {
    it("配列の定数", () => {
      expect(stringArrayConstOf('pub const X: [&str; 2] = ["p", "q"];\r\n', "X")).toEqual(["p", "q"]);
      expect(() => stringArrayConstOf("pub const Y: [&str; 1] = [\"p\"];", "X")).toThrow();
    });

    /** **`rustfmt` が折り返した形**（1行に1要素・末尾のカンマ / CRLF）。後ろの別の配列は読まない。 */
    it("折り返した配列の定数", () => {
      const planted = [
        "pub const X: [&str; 3] = [",
        '    "p",',
        '    "q",',
        '    "r",',
        "];",
        'pub const Z: [&str; 1] = ["z"];',
      ].join("\r\n");
      expect(stringArrayConstOf(planted, "X")).toEqual(["p", "q", "r"]);
      // **綴りでない要素（定数の名前）は読めないと言う**（他の定数から組んだ形を通さない）。
      expect(() =>
        stringArrayConstOf(['pub const X: [&str; 2] = [', '    "p",', "    OTHER,", "];"].join("\n"), "X"),
      ).toThrow();
      // **終わりが無ければ読めないと言う。**
      expect(() => stringArrayConstOf('pub const X: [&str; 1] = [\n    "p",\n', "X")).toThrow();
    });

    it("文字列の定数", () => {
      expect(stringConstOf('pub const X: &str = "zz";\r\n', "X")).toBe("zz");
      expect(() => stringConstOf('pub const X: &str = "a";\npub const X: &str = "b";', "X")).toThrow();
    });

    it("状態の綴り", () => {
      const planted = [
        "    pub const fn label(&self) -> &'static str {",
        "        match self {",
        "            // コメントの \"x\" は数えない",
        '            Self::One => "one",',
        '            Self::Two { .. } => "two",',
        "        }",
        "    }",
      ].join("\r\n");
      expect(faceStateLabelsOf(planted)).toEqual(["one", "two"]);
      expect(() => faceStateLabelsOf("fn other() {}")).toThrow();
      expect(faceStateVariantsOf(planted)).toEqual({ One: "one", Two: "two" });
      expect(() => faceStateVariantsOf("fn other() {}")).toThrow();
    });
  });
});
