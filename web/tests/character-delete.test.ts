/**
 * **削除が拒否されたときの読み分け**（作業単位 D-7 / `src/lib/characterDelete.ts`）。
 *
 * # なぜ純粋関数で見るのか
 *
 * **この作業単位の本体は「消せなかったときの案内」である。**
 * 画面は**押してみるまで消せるかを知れない**（判定は `store` が削除と同じ文で
 * 不可分に行う / `src/server/admin.rs` の `delete_character` の doc）。
 * したがって「返ってきた失敗を、利用者が次に取れる行動へどう写すか」だけが
 * こちら側の設計であり、そこを単体で見張れるようにしてある。
 *
 * # 画面側との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **失敗の種別ごとの文言と、次の一手** | **ここ** |
 * | どの行を押したらどの識別子へ `DELETE` が飛ぶか・確認の段・成功後の一覧 | `character-list-deleting.test.ts` |
 * | パスの符号化 | `api-paths.test.ts` |
 *
 * **I/O を持たないので jsdom は要らない**（既定の `node` で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError } from "../src/lib/api";
import {
  describeDeleteFailure,
  HAS_SESSIONS_CODE,
  NOT_FOUND_CODE,
} from "../src/lib/characterDelete";

/**
 * サーバが実際に返すもの（`src/server/error.rs` の
 * `ApiError::character_has_sessions` を読んで写した）。
 *
 * **⚠ これは「サーバはこう返す」という前提を書いたフィクスチャであって、
 * 写しの正しさを見張るものではない。** 綴りの突き合わせは、このファイル末尾の
 * **「`src/server/error.rs` の `code` と一致する」**が
 * **写し元を実際に読んで**行う。
 *
 * **⚠ そして `message` をここから期待値に使わないこと。**
 * 使うと「サーバの文言を素通ししている」と「FE が同じ文字列を決め打ちしている」が
 * **区別できない**（レビューで実測: `message` を定数へ置換する変異が全緑だった）。
 * **見張り番は `文言が改訂されても、そのまま出す`** ——
 * **同じ `code` で別の文言を返す腕**を通す。
 */
const HAS_SESSIONS = new ApiRequestError(
  "会話のあるキャラクターは削除できません",
  409,
  "invalid_request_error",
  "character_has_sessions",
);

/** 同上（`ApiError::character_not_found`）。 */
const NOT_FOUND = new ApiRequestError(
  "指定されたキャラクターが存在しません",
  404,
  "invalid_request_error",
  "character_not_found",
);

describe("会話があるので消せない（ADR-0016 §2）", () => {
  it("サーバの文言をそのまま出す（言っていないことを足さない）", () => {
    const failure = describeDeleteFailure(HAS_SESSIONS);
    expect(failure.reason).toBe("has-sessions");
    expect(failure.message).toBe(HAS_SESSIONS.message);
  });

  /**
   * **⚠ 「会話を消せば削除できます」と書かないこと。**
   *
   * **会話を消す入口はどこにも存在しない**（`src/server/mod.rs` のルータに
   * セッションを消す経路は無い）。無い操作を案内すると、利用者は
   * **永久に見つからないボタンを探すことになる。**
   */
  it("案内が、実際に取れる行動だけを指している", () => {
    const guidance = describeDeleteFailure(HAS_SESSIONS).guidance;
    expect(guidance).not.toBeNull();
    const text = guidance as string;
    // **この画面に実在する導線**（一覧の行の「編集」）。
    expect(text).toContain("編集");
    // **しまってから完全に消す導線**（作業単位 A-10）。どちらの釦も一覧に実在する
    // （`character-list-archiving.test.ts` が押している）。
    expect(text).toContain("「しまう」");
    expect(text).toContain("「完全に消す」");
    // **存在しない操作を案内していない。**
    expect(text).not.toContain("会話を削除");
    expect(text).not.toContain("会話を消せ");
    expect(text).not.toContain("セッションを削除");
  });

  it("綴りを直せば通る、と読めない（404 の案内と混ざっていない）", () => {
    const guidance = describeDeleteFailure(HAS_SESSIONS).guidance as string;
    // 404 側の一手（「再読み込み」）を混ぜない。**対処が正反対**
    // （`classify_delete` の doc）。
    expect(guidance).not.toContain("再読み込み");
  });
});

describe("そんなキャラクターは居ない（404）", () => {
  it("会話がある側と別の種別になる", () => {
    const failure = describeDeleteFailure(NOT_FOUND);
    expect(failure.reason).toBe("not-found");
    expect(failure.reason).not.toBe(describeDeleteFailure(HAS_SESSIONS).reason);
    expect(failure.message).toBe(NOT_FOUND.message);
  });

  it("案内は「取り直す」（この画面に実在する再読み込み）", () => {
    const guidance = describeDeleteFailure(NOT_FOUND).guidance;
    expect(guidance).not.toBeNull();
    expect(guidance as string).toContain("再読み込み");
    // **会話がある側の一手と混ざっていない。**
    expect(guidance as string).not.toContain("編集");
  });

  it("2種の案内は、文言そのものが違う", () => {
    expect(describeDeleteFailure(NOT_FOUND).guidance).not.toBe(
      describeDeleteFailure(HAS_SESSIONS).guidance,
    );
  });
});

describe("それ以外は推測しない", () => {
  /**
   * **`code` が区別の担い手**（`ApiError::character_already_exists` の doc が
   * 「区別の担い手は `code`」と書いている）。
   *
   * **状態番号だけで決めない。** UI のフォールバックに落ちた 404 は
   * 素の本文で返る（`src/server/web.rs`）ので、**`code` の無い 404 は
   * 「キャラクターが居ない」ではなく「その経路がそもそも無い」**でもありうる。
   */
  it("`code` の無い 404 を、居ない扱いにしない", () => {
    const failure = describeDeleteFailure(
      new ApiRequestError("サーバが 404 を返しました", 404, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("サーバが 404 を返しました");
  });

  it("`code` の無い 409 を、会話がある扱いにしない", () => {
    const failure = describeDeleteFailure(
      new ApiRequestError("サーバが 409 を返しました", 409, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
  });

  it("知らない `code` は、そのまま「分からない」", () => {
    const failure = describeDeleteFailure(
      new ApiRequestError("何か", 500, "api_error", "character_is_haunted"),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("何か");
  });

  it("繋がらなかった回も、次の一手を捏造しない", () => {
    const failure = describeDeleteFailure(
      new ApiRequestError(
        "サーバへ接続できませんでした（personacore は動いていますか）",
        0,
        null,
        null,
      ),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toContain("接続できませんでした");
  });

  it("`ApiRequestError` ですらない失敗は、中身を画面へ流さない", () => {
    const failure = describeDeleteFailure(
      new Error("C:\\Users\\someone\\Rust\\personacore\\web で落ちた"),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("予期しない失敗です");
    expect(failure.message).not.toContain("Rust");
  });
});

/**
 * **サーバの文言を素通ししているか**（レビューの是正 3）。
 *
 * # なぜフィクスチャの `message` では見られないのか
 *
 * `HAS_SESSIONS.message` は**サーバの文言の写し**なので、それを期待値にすると
 * **「素通ししている」と「FE が同じ文字列を決め打ちしている」が同値**になる
 * （**フィクスチャ同値** —— CLAUDE.md の最頻の穴）。
 * **実測: `message: failure.message` を定数へ置換する変異が、両枝とも全緑だった。**
 *
 * # **仮定の話ではない**
 *
 * `src/server/error.rs` は `character_already_exists` の文言を
 * **「名前」→「識別子」へ実際に改訂した記録**を持っている
 * （`code` は変えず、人間が読む側だけ直した）。
 * **`code` を保ったまま `message` だけが変わる**のは、このリポジトリで**起きたこと**である。
 *
 * **したがって、同じ `code` で別の文言を返す腕を通す。**
 */
describe("文言は素通しする（決め打ちしない）", () => {
  /** **改訂された想定の文言。** 写し元のどの綴りとも一致しない。 */
  const REWORDED_HAS_SESSIONS = new ApiRequestError(
    "会話のある個体は消せません（文言が改訂された想定）",
    409,
    "invalid_request_error",
    HAS_SESSIONS_CODE,
  );

  /** 同上（404 の側）。 */
  const REWORDED_NOT_FOUND = new ApiRequestError(
    "その識別子は見つかりませんでした（文言が改訂された想定）",
    404,
    "invalid_request_error",
    NOT_FOUND_CODE,
  );

  it("会話がある側: 文言が改訂されても、そのまま出す", () => {
    const failure = describeDeleteFailure(REWORDED_HAS_SESSIONS);
    // **種別は `code` で決まるので変わらない。**
    expect(failure.reason).toBe("has-sessions");
    expect(failure.message).toBe(REWORDED_HAS_SESSIONS.message);
    // **古い文言を出していない**（決め打ちの変異はここで落ちる）。
    expect(failure.message).not.toBe(HAS_SESSIONS.message);
    expect(failure.message).not.toContain("キャラクター");
    // 案内は `code` から決まるので、こちらは変わらない。
    expect(failure.guidance).not.toBeNull();
    expect(failure.guidance as string).toContain("編集");
  });

  it("居ない側: 文言が改訂されても、そのまま出す", () => {
    const failure = describeDeleteFailure(REWORDED_NOT_FOUND);
    expect(failure.reason).toBe("not-found");
    expect(failure.message).toBe(REWORDED_NOT_FOUND.message);
    expect(failure.message).not.toBe(NOT_FOUND.message);
    expect(failure.message).not.toContain("存在しません");
    expect(failure.guidance as string).toContain("再読み込み");
  });

  it("2つの枝が、互いの文言を出していない", () => {
    // **枝どうしの取り違えも殺す**（`message` を他方の定数にする変異）。
    expect(describeDeleteFailure(REWORDED_HAS_SESSIONS).message).not.toBe(
      describeDeleteFailure(REWORDED_NOT_FOUND).message,
    );
  });
});

/**
 * **`code` の綴りは `src/server/error.rs` の写しである**（レビューの是正 4）。
 *
 * # なぜ見張るのか
 *
 * **型ではなく値の写し。** 綴りが1文字ずれると**どちらの枝にも入らず
 * `unknown` に落ち、案内が黙って消える** ——
 * **画面は何も言わなくなるのに、型検査もテストも緑のまま。**
 *
 * **先例は `tests/character-edit.test.ts`**（`src/domain/character.rs` の
 * `MAX_CHARS` を読んで突き合わせる）。**同じ形で塞ぐ。**
 *
 * **⚠ ADR-0031 §6 が「気付く仕掛けは痛みが出た時点で」と書いているのは
 * 型の写しの話**で、これは値の写しなので別件。
 */
describe("`code` は写し元と一致する", () => {
  it("`src/server/error.rs` の `code` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
      "utf8",
    );

    expect(codeOf(source, "character_has_sessions")).toBe(HAS_SESSIONS_CODE);
    expect(codeOf(source, "character_not_found")).toBe(NOT_FOUND_CODE);
    // **対照: 2つは別物**（同じ値を2回読んで緑、という形にしない）。
    expect(HAS_SESSIONS_CODE).not.toBe(NOT_FOUND_CODE);
  });

  /**
   * **走査する側が壊れていないこと**（`15b` の「そもそも探せていないから緑」）。
   *
   * **⚠ 実ファイルだけを見る対照は自己成就する** ——
   * 走査の綴りを壊す変異が、**別の綴りを走査対象そのものに書き込む**と
   * 1件見つけて緑になる。**だから、植えたソースを同じ関数へ通す。**
   */
  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "    pub const fn character_has_sessions() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      '            message: "植えた文言",',
      '            kind: "invalid_request_error",',
      '            code: Some("planted_code_for_the_control"),',
      "        }",
      "    }",
    ].join("\n");

    // **実ファイルの値ではなく、植えた値が返る**（＝本当に読んでいる）。
    expect(codeOf(planted, "character_has_sessions")).toBe(
      "planted_code_for_the_control",
    );
    expect(codeOf(planted, "character_has_sessions")).not.toBe(
      HAS_SESSIONS_CODE,
    );
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => codeOf("", "character_has_sessions")).toThrow();

    // **次の構築子の値を拾ってこない**（`pub const fn` の手前で切る）。
    const planted = [
      "    pub const fn character_has_sessions() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      "        }",
      "    }",
      "",
      "    pub const fn character_not_found() -> Self {",
      "        Self {",
      '            code: Some("character_not_found"),',
      "        }",
      "    }",
    ].join("\n");
    expect(() => codeOf(planted, "character_has_sessions")).toThrow();
  });
});

/**
 * `error.rs` のその構築子から `code` を1つ取り出す。
 *
 * **次の `pub const fn` の手前で切る。** 切らないと、`code` を持たない構築子を
 * 尋ねたときに**次の構築子の値を返す**（探せていないのに緑になる）。
 */
function codeOf(source: string, constructor: string): string {
  const start = source.indexOf(`pub const fn ${constructor}() -> Self {`);
  if (start < 0) {
    throw new Error(`${constructor} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("pub const fn ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = /code: Some\("([^"]+)"\)/.exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`${constructor} に code がありません`);
  }
  return found[1];
}
