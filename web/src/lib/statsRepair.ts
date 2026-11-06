/**
 * **読み出せなくなった性格ステータスを、画面から直す**（作業単位 D-8 /
 * [ADR-0015](../../../docs/adr/0015-stat-allocation.md) /
 * [ADR-0022](../../../docs/adr/0022-repairing-memories-without-deleting.md) と同じ立場）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `replaceStats`（**既存の1本だけ**）、
 * 画面は `StatsRepairForm.svelte`、入口は `StatsPanel.svelte` の
 * `statsState.kind === "failed"` の枝。
 *
 * # なぜ要るのか
 *
 * **`GET .../stats` が 500 になる状態がある** ——
 * 尺度を超える値・孤児・範囲外、そして**正規化すると同じになる軸の重複**（作業単位 B-19）。
 * **そのとき、画面には「再読み込み」と「キャラクターの一覧へ」しか無かった。**
 * 「軸を追加」も「この内容で置き換える」も `{:else}` 側のフォームの中にしかなく、
 * **直すには `curl` か `sqlite3` が要った。**
 *
 * **⚠ API は開いている。** `Store::replace_stat_allocs` は既存の軸を読まないので、
 * **壊れた行があっても `PUT .../stats` は通る**
 * （`tests/server_admin.rs::a_row_beyond_the_scale_can_be_repaired_through_the_stats_endpoint`）。
 * **閉じていたのは画面だけ。**
 *
 * # ⚠ **これは「置き換え」の第2のモードであって、第5のモードではない**
 *
 * 送る本文も組み立ても、通常の置き換え（`statsEdit.ts` の `buildReplace`）と**同一**である。
 * **違うのは「元の値が読めない」ことだけ。** したがって:
 *
 * | | 通常（`StatsPanel` の `{:else}`） | **修復（ここ）** |
 * |---|---|---|
 * | 下書きの初期値 | `draftOfStats`（サーバの値） | **空**（推測で埋めない） |
 * | 軸名 | 保存済みの行は変えられない | **全部が入力欄** |
 * | 何が消えるか | `replaceSummary` が押す前に言う | **言えない**（下の `UNKNOWN_LOSS_NOTICE`） |
 * | 送る本文 | `buildReplace` | **同じ `buildReplace`** |
 *
 * **組み立てを写さないこと。** 写した瞬間、
 * 「全軸を送る」という意味論の見張り番（`tests/stats-edit.test.ts` の
 * `1軸だけ下げても、触っていない軸まで送る`）が片方にしか効かなくなる。
 *
 * # ⚠ **「壊れている」と「読めない」を混ぜない**（実測してある）
 *
 * **`ApiError::internal()` の `code` は `None`**（`src/server/error.rs`）。
 * つまり **FE には「格納値が壊れた 500」と「DB 障害の 500」を分ける手掛かりが無い。**
 * 分けられるのは**サーバへ届いたかどうか**まで（`api.ts` の `unreachableOrAborted` が
 * 状態番号 `0` を付ける）。
 *
 * **だから、この画面は「壊れています」と言わない。**
 * 言うのは「**いまの値を読み出せない**」までで、**そこから先は推測しない。**
 * 写し元を読む見張り番は `tests/stats-repair.test.ts`
 * （`ApiError::internal()` に `code` が無いことを、`error.rs` を実際に読んで固定する）。
 *
 * # ⚠ **形の食い違いを、握り潰さない**（レビューの指摘 / 2026-08-28）
 *
 * `api.ts` は**返り値の形を検証しない**と自ら明記している。
 * したがって**一覧の応答が想定と違う形で来ることは起こりうる**（版の食い違い・別のサーバ）。
 *
 * **`scaleFromListing` を全域関数にした**（例外を投げない）。**理由は3つ:**
 *
 * 1. **画面を壊さない。** 例外を再送出すると、描画の途中で落ちうる。
 * 2. **`catch` に飲ませない。** 飲ませると**道具の壊れが永久に見えなくなる。**
 * 3. **「尺度が読めない」と同じ結末にしない。** 形の食い違いは
 *    **`{ kind: "malformed" }`** という別の値になり、**画面には別の文が出る**
 *    （`MALFORMED_LISTING_NOTICE`）。**同じ文にすると、区別が付かないまま埋もれる。**
 *
 * **例外で表さない代わりに、値で表す。** 見張り番は `tests/stats-repair.test.ts` と
 * `tests/stats-repair-panel.test.ts` の
 * `一覧の形が違えば、尺度が引けなかった回とは別の文を出す`。
 */

import { ApiRequestError } from "./api";
import { NOT_FOUND_CODE, emptyRow, type StatsDraft } from "./statsEdit";

/**
 * **いまの値を読めなかった理由**（分けられるところまで）。
 *
 * | 種別 | 何が分かっているか |
 * |---|---|
 * | `unreachable` | **サーバへ届いていない**（状態番号 `0`） |
 * | `missing` | **そのキャラクターが居ない**（404 ＋ `character_not_found`） |
 * | `server` | **サーバは答えたが、応答を使えなかった** |
 * | `unknown` | **`ApiRequestError` ですらない**（画面側の想定外） |
 *
 * **⚠ `server` をこれ以上割らない。** 割る材料が無い（上の ⚠）。
 * 「格納値が壊れています」と書いたら、それは推測である。
 *
 * # ⚠ `unknown` は絵空事ではない（レビューの指摘 / 2026-08-28）
 *
 * `StatsPanel` の `loadStats` は **`draftOfStats(value)` を `getStats` と同じ `try` の中**で
 * 呼んでいる。`api.ts` は**返り値の形を検証しない**ので、
 * **`stats` の欠けた 200 が `TypeError` になってここへ落ちる。**
 * **その回に修復の入口が出ないと、D-8 が直そうとした状態そのものになる**
 * （エラーの `<p>` だけが出て、フォームも説明も無い）。
 * **だから `unknown` でも入口を出す**（`repairEntryFor`）。
 * 見張り番は `tests/stats-repair-panel.test.ts` の
 * `いまの値が 200 でも形が違えば、想定外として扱い、それでも直せる`。
 */
export type StatsReadFailureKind =
  | "unreachable"
  | "missing"
  | "server"
  | "unknown";

/** 読めなかったこと。**文言はサーバが言ったもの。** */
export interface StatsReadFailure {
  kind: StatsReadFailureKind;
  /** **サーバが言ったこと。** 読めなかったときだけこちらの定型文になる。 */
  message: string;
}

/**
 * 失敗を、画面が使える形へ。
 *
 * **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約:
 * エラーに内部の詳細を載せない）。
 */
export function describeStatsFailure(failure: unknown): StatsReadFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { kind: "unknown", message: "予期しない失敗です" };
  }
  // **状態番号 `0` は「届いていない」**（`api.ts` の `unreachableOrAborted`）。
  // **HTTP の失敗ではない**ので、同じ 500 の仲間に入れない。
  if (failure.status === 0) {
    return { kind: "unreachable", message: failure.message };
  }
  if (failure.code === NOT_FOUND_CODE) {
    return { kind: "missing", message: failure.message };
  }
  return { kind: "server", message: failure.message };
}

/**
 * **尺度の引き当て**（値の上端 / ADR-0015）。
 *
 * # ⚠ なぜ一覧から引くのか
 *
 * **`GET /admin/characters/{handle}` は存在しない。**
 * 1件を指すパスが受けるのは `PATCH` と `DELETE` だけ（ADR-0016 §1 / `src/server/mod.rs`）。
 * **`GET .../stats` は落ちている**（尺度はその応答にしか無い）。
 * **残るのは一覧だけ**で、**一覧は `stat_alloc` を1行も読まない**
 * （`Store::list_characters`）。
 *
 * **推測ではなく実測してある** ——
 * `tests/server_admin.rs::a_broken_stat_row_stops_stats_and_drift_but_the_listing_still_reports_the_scale`
 * が、壊れた行を作って3つの入口を実際に叩いている。
 *
 * **⚠ 変容の応答（`GET .../drift`）から取らないこと。** そちらも
 * `Store::stat_allocs` を読むので、**本物の壊れ方では一緒に 500 になる**（同テストで実測）。
 */
export type ScaleLookup =
  /** 一覧に読める行があり、その行の尺度。 */
  | { kind: "known"; scale: number }
  /** **キャラクターの行そのものが読めない**（`unreadable` に居る）。 */
  | { kind: "character-unreadable" }
  /** 一覧のどちらにも居ない・一覧そのものが読めなかった。 */
  | { kind: "unknown" }
  /** **一覧の応答が、想定した形をしていない**（このファイル冒頭の ⚠）。 */
  | { kind: "malformed" };

/** 形だけを見た一覧（**中身は `unknown` のまま**）。 */
interface LooseListing {
  characters: unknown[];
  unreadable: unknown[];
}

/**
 * 一覧が、**少なくとも2つの配列を持つ形**をしているか。
 *
 * **`as` で押し込まない**（`api.ts` の `isErrorBody` と同じ立場）。
 */
function isListing(value: unknown): value is LooseListing {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const listing = value as { characters?: unknown; unreadable?: unknown };
  return Array.isArray(listing.characters) && Array.isArray(listing.unreadable);
}

/** 要素の欄を、**形を仮定せずに**読む。 */
function fieldOf(entry: unknown, key: string): unknown {
  if (typeof entry !== "object" || entry === null) {
    return undefined;
  }
  return (entry as Record<string, unknown>)[key];
}

/**
 * 一覧から、そのキャラクターの尺度を引く。**例外を投げない**（冒頭の ⚠）。
 *
 * **識別子で引く。** 位置で取ると（先頭・末尾）**別のキャラクターの尺度**を
 * 値の上端として画面に出すことになる（CLAUDE.md「抜けるのはいつも帰属」）。
 *
 * **引数が `unknown` なのは、`api.ts` が応答の形を検証しないから**であって、
 * 型を緩めたのではない。**ここが唯一の関所**である。
 */
export function scaleFromListing(
  listing: unknown,
  handle: string,
): ScaleLookup {
  if (!isListing(listing)) {
    return { kind: "malformed" };
  }
  const readable = listing.characters.find(
    (character) => fieldOf(character, "handle") === handle,
  );
  if (readable !== undefined) {
    const scale = fieldOf(readable, "stat_scale");
    // **数でなければ「読めた」と言わない。** 言うと `null / 7` の親戚が出る。
    return typeof scale === "number"
      ? { kind: "known", scale }
      : { kind: "malformed" };
  }
  // **`handle` が `null` の行は決して一致しない**（D-6 と同じ ——
  // 識別子が読めない行は API から指せない）。
  const broken = listing.unreadable.find(
    (row) => fieldOf(row, "handle") === handle,
  );
  if (broken !== undefined) {
    return { kind: "character-unreadable" };
  }
  return { kind: "unknown" };
}

/** 修復の入口を出せない理由。 */
export type RepairBlockedReason =
  | "unreachable"
  | "missing"
  | "character-unreadable";

/**
 * **修復のフォームに渡す上端。**
 *
 * # ⚠ `number | null` にしない
 *
 * `null` にすると**「なぜ分からないのか」を別の欄で運ぶ**ことになり、
 * **「上端は分かっているのに理由の文も出ている」という状態が型として作れてしまう。**
 * ここで畳んでおけば、**画面はどちらか一方しか描けない。**
 */
export type RepairScale =
  | { kind: "known"; scale: number }
  /** 上端が分からない。**`notice` がその理由を言う**（推測で埋めない）。 */
  | { kind: "unreadable"; notice: string };

/**
 * **失敗の枝に、何を出すか。**
 *
 * | | 出すもの |
 * |---|---|
 * | `pending` | まだ尺度を引いている（**フォームも案内も出さない**） |
 * | `form` | **修復のフォーム** |
 * | `blocked` | **フォームを出さず、状態だけ説明する** |
 *
 * # ⚠ `pending` を「フォーム」に寄せないこと
 *
 * **`loadStats` は `statsState` を立ててから `await loadScale()` する**ので、
 * **その間に描画の境界がある。**
 * `pending` を潰すと、**`character-unreadable` の回でも一瞬フォームが描かれ**、
 * 「押せば必ず失敗するボタンは出さない」がその瞬間だけ破れる。
 * **見張り番は `tests/stats-repair-panel.test.ts` の
 * `尺度を引いている間は、修復の入口も説明も出さない`**
 * ——**`vi.waitFor` では書けない**（「A が B になる」ではなく「その間 B であってはならない」）。
 */
export type RepairEntry =
  | { kind: "pending" }
  | { kind: "form"; scale: RepairScale }
  | { kind: "blocked"; reason: RepairBlockedReason; message: string };

/**
 * **サーバへ届いていない**ときの説明。
 *
 * **名指しする操作は、この画面に実在するものだけ**（D-7 / B-15 / B-16 と同じ立場）。
 * 実在するのは `StatsPanel` の見出しにある **「再読み込み」**。
 */
const UNREACHABLE_MESSAGE =
  "サーバへ接続できていないので、いまの値が読めません。" +
  "personacore が動いていることを確かめてから、「再読み込み」でやり直してください。" +
  "（繋がるまでは、この画面から性格ステータスを直すことはできません。）";

/** **そのキャラクターが居ない**ときの説明。実在するのは **「キャラクターの一覧へ」**。 */
const MISSING_MESSAGE =
  "このキャラクターが見つかりません（すでに消えているのかもしれません）。" +
  "「キャラクターの一覧へ」で取り直してください。";

/**
 * **キャラクターの行そのものが読めない**ときの説明。
 *
 * # ⚠ ここでフォームを出さない理由（**無い操作を案内しない**）
 *
 * `PUT .../stats` は**先にキャラクターを引く**（`admin.rs` の `replace_stats` ——
 * 値の検証にそのキャラクターの尺度が要る）。その引き当ては全列を `parse` するので、
 * **行が壊れていれば置き換えも必ず 500 になる。**
 * **押せば必ず失敗するボタンは、操作ではない**（D-6 の「`handle` が `null` の行には
 * 入口を出さない」と同じ判断）。
 *
 * **直す入口は別にある** —— 一覧の修復（作業単位 D-6）。そこへ送る。
 */
const CHARACTER_UNREADABLE_MESSAGE =
  "このキャラクターの行そのものが読み出せないため、性格ステータスの置き換えも通りません。" +
  "「キャラクターの一覧へ」で、先にその行を修復してください。";

/**
 * **尺度を引けなかった**ときの断り。**上端を知らないことを、知らないと言う。**
 *
 * **名指しする操作は実在するものだけ**（「再読み込み」は `StatsPanel` の見出しにある）。
 */
export const UNKNOWN_SCALE_NOTICE =
  "このキャラクターの尺度（値の上端）を読み取れませんでした。" +
  "「再読み込み」で取り直せますが、分からないままでも直せます" +
  "（上端を超える値は、保存したときにサーバが理由付きで返します）。";

/**
 * **一覧の応答の形が、想定と違った**ときの断り。
 *
 * **⚠ `UNKNOWN_SCALE_NOTICE` と同じ文にしないこと**（レビューの指摘 / 2026-08-28）。
 * **同じにすると「引けなかった」と「そもそも読めない形だった」が区別できなくなり、
 * 道具の壊れが埋もれる。** 前者はサーバが落ちているだけで、後者は**版の食い違い**である。
 */
export const MALFORMED_LISTING_NOTICE =
  "キャラクターの一覧の応答を、想定した形として読めませんでした" +
  "（personacore と画面の版が食い違っているのかもしれません）。" +
  "「再読み込み」で取り直してください。読めないままでも、この下の欄から直せます" +
  "（上端を超える値は、保存したときにサーバが理由付きで返します）。";

/**
 * 失敗と尺度の引き当てから、失敗の枝に出すものを決める。**純粋関数。**
 *
 * **`scale` が `null` は「まだ引いていない」。** `{ kind: "unknown" }` とは別物で、
 * **混ぜると、読み込み中に「尺度が分かりません」と言う**ことになる。
 */
export function repairEntryFor(
  failure: StatsReadFailure,
  scale: ScaleLookup | null,
): RepairEntry {
  if (failure.kind === "unreachable") {
    return {
      kind: "blocked",
      reason: "unreachable",
      message: UNREACHABLE_MESSAGE,
    };
  }
  if (failure.kind === "missing") {
    return { kind: "blocked", reason: "missing", message: MISSING_MESSAGE };
  }
  if (scale === null) {
    return { kind: "pending" };
  }
  if (scale.kind === "character-unreadable") {
    return {
      kind: "blocked",
      reason: "character-unreadable",
      message: CHARACTER_UNREADABLE_MESSAGE,
    };
  }
  if (scale.kind === "known") {
    return { kind: "form", scale: { kind: "known", scale: scale.scale } };
  }
  // **理由ごとに別の文。** 「引けなかった」と「読めない形だった」を混ぜない。
  return {
    kind: "form",
    scale: {
      kind: "unreadable",
      notice:
        scale.kind === "malformed"
          ? MALFORMED_LISTING_NOTICE
          : UNKNOWN_SCALE_NOTICE,
    },
  };
}

/**
 * 修復の下書き。**空の行が1本。**
 *
 * # 推測で埋めない
 *
 * 元の値は読み出せない（読めていれば、この画面は出ていない）。
 * **`characterRepair.ts` の `emptyRepairDraft` と同じ立場**で、
 * 推測した値が「いま正しい設定」として保存されるのを避ける。
 *
 * # ⚠ 0 行から始めない
 *
 * 0 行の下書きは**そのまま送れてしまう**（空の配列 ＝ 全軸が消える）。
 * **1本の空行から始めれば、組み立てが「軸名の空いている行があります」で止まる**ので、
 * **何も打たずに押しただけで全消しになることが無い。**
 * **全部消すこと自体は禁じていない** —— その行を「外す」と、消せる。
 */
export function emptyRepairDraft(): StatsDraft {
  return { rows: [emptyRow()] };
}

/**
 * **何が消えるかを、押す前に言えない**という宣言。
 *
 * # ⚠ できないことを、できるふりをしない（D-3c の作法）
 *
 * 通常のフォームは `replaceSummary` が「外した X は消えます」と**名指しで**言う。
 * **この画面ではそれが原理的に書けない** ——
 * **いま保存されている軸を、1本も読み出せないから。**
 *
 * **「消えるものはありません」と書かないこと。** それは嘘になる。
 * **黙るのも駄目** —— 丸ごと置き換えなので、**書かなかった軸は実際に消える。**
 * **言えるのは「ここに書いた軸だけが残る」ことと、「何が消えるかは言えない」ことの2つ。**
 */
export const UNKNOWN_LOSS_NOTICE =
  "いま保存されている軸は読み出せません。そのため、保存したときに何が消えるかを、" +
  "押す前に言うことができません。これは丸ごと置き換えなので、" +
  "ここに書いた軸だけが残ります。";
