/**
 * **削除が失敗したときに、利用者へ何と言うか**（作業単位 D-7 / ADR-0016 §2）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `deleteCharacter`、
 * 出すのは `CharacterList.svelte`。
 *
 * # なぜここだけ切り出すのか
 *
 * **この作業単位の本体は「消せなかったときの案内」である。**
 * 削除そのものは1本の `DELETE` でしかなく、設計が要るのは失敗側だけ。
 * **押してみるまで消せるか分からない**（判定は `store` が削除と同じ文で
 * 不可分に行う / `src/server/admin.rs` の `delete_character` の doc）ので、
 * **「押した後に何と言うか」が唯一の導線**になる。
 *
 * # 見分けるのは `code`。状態番号では決めない
 *
 * `src/server/error.rs` の `character_already_exists` の doc が
 * **「区別の担い手は `code`」**と書いている。状態番号で決めると、
 * **UI のフォールバックに落ちた 404**（素の本文 / `src/server/web.rs`）を
 * 「そのキャラクターは居ません」と読んでしまう——**居ないのは経路の方**である。
 *
 * | `code` | 状態 | 何が起きたか |
 * |---|---|---|
 * | `character_has_sessions` | 409 | 居るが、**しまっておらず会話があるので消せない**（しまえば消せる / 作業単位 A-10） |
 * | `character_not_found` | 404 | **そもそも居ない** |
 * | それ以外・無し | — | **分からない。推測しない** |
 *
 * # ⚠ **無い操作を案内しないこと**
 *
 * **会話を1つだけ消す入口はどこにも存在しない**（`src/server/mod.rs` のルータに
 * セッションを消す経路は無い）。「会話を消せば削除できます」と書くと、
 * **利用者は永久に見つからないボタンを探すことになる。**
 * ここが案内してよいのは、**この画面に実在する導線だけ**である。
 */

import { ApiRequestError } from "./api";

/**
 * サーバが返す `code`（**`src/server/error.rs` の綴りの写し**）。
 *
 * # ⚠ **値の写しなので、ずれると案内が黙って消える**
 *
 * 綴りが1文字でも食い違えば、**どちらの枝にも入らず `unknown` に落ちる。**
 * **画面は理由も次の一手も言わなくなるが、型検査もテストも緑のまま**である
 * （型の写しは「形が違う」と気付ける余地があるが、**値の写しは静かに外れる**）。
 *
 * **したがって、写し元を実際に読む見張り番を置いた** ——
 * `tests/character-delete.test.ts` の
 * **「`src/server/error.rs` の `code` と一致する」**。
 * **先例は `characterEdit.ts` の `LIMITS`**（`src/domain/character.rs` を読む）で、
 * **理由も同じ**（型ではなく値で、ずれると画面が黙って嘘をつく）。
 *
 * > **ADR-0031 §6 が「気付く仕掛けは痛みが出た時点で」と言っているのは型の写し**の話。
 * > **これは値の写しなので別件**である。
 */
export const HAS_SESSIONS_CODE = "character_has_sessions";
/** 同上（`ApiError::character_not_found`）。 */
export const NOT_FOUND_CODE = "character_not_found";

/**
 * 失敗の種別。**`unknown` は「分類できなかった」であって「サーバの障害」ではない。**
 */
export type DeleteFailureReason = "has-sessions" | "not-found" | "unknown";

/** 画面に出すもの。 */
export interface DeleteFailure {
  reason: DeleteFailureReason;
  /**
   * **サーバが言ったこと。** 読めなかったときだけこちらの定型文になる
   * （`chat.ts` の `outcomeNotice` と同じ立場 —— **足さない**）。
   */
  message: string;
  /**
   * **次に取れる行動。** 分からないなら `null`。
   *
   * **`null` を埋めないこと。** 分類できていない失敗に一手を書くと、
   * それは推測であり、外れたときに利用者を遠回りさせる。
   */
  guidance: string | null;
}

/**
 * **会話があるので消せない**ときの一手。
 *
 * **会話を1つだけ消す案内はしない**（上の ⚠。その入口はいまも無い）。実在するのは
 * **一覧の行の「しまう」と、しまった一覧の「完全に消す」**（作業単位 A-10 / ADR-0049 §決定1）、
 * そして**行の「編集」**（`PATCH` / ADR-0016 §1）なので、その3つだけを指す。
 *
 * **完全に消すと会話・記憶ごと消えることも言う**（消したいのが一覧の片付けだけなら、
 * しまうだけで足りる）。
 */
const HAS_SESSIONS_GUIDANCE =
  "会話のあるキャラクターは、行の「しまう」でしまってから、" +
  "「しまったキャラクター」の一覧の「完全に消す」で消せます（会話・記憶も一緒に消えます）。" +
  "一覧から外したいだけなら、しまうだけで足ります。" +
  "表示名・ペルソナ・出力形式・尺度を変えたいだけなら、行の「編集」から直せます。";

/**
 * **居ないと言われた**ときの一手。
 *
 * 対処が正反対（`classify_delete` の doc）——こちらは**綴り直し・取り直し**で通る。
 * 実在するのは一覧の**「再読み込み」**。
 */
const NOT_FOUND_GUIDANCE =
  "すでに消えているのかもしれません。「再読み込み」で一覧を取り直してください。";

/**
 * 失敗を、画面に出せる形へ。
 *
 * **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約:
 * エラーに内部の詳細を載せない）。`CharacterList` の `messageOf` と同じ文言。
 */
export function describeDeleteFailure(failure: unknown): DeleteFailure {
  if (!(failure instanceof ApiRequestError)) {
    return {
      reason: "unknown",
      message: "予期しない失敗です",
      guidance: null,
    };
  }
  switch (failure.code) {
    case HAS_SESSIONS_CODE:
      return {
        reason: "has-sessions",
        message: failure.message,
        guidance: HAS_SESSIONS_GUIDANCE,
      };
    case NOT_FOUND_CODE:
      return {
        reason: "not-found",
        message: failure.message,
        guidance: NOT_FOUND_GUIDANCE,
      };
    default:
      return {
        reason: "unknown",
        message: failure.message,
        guidance: null,
      };
  }
}
