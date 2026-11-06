/**
 * **性格ステータスの置き換えを組み立てる**（作業単位 D-4 / ADR-0015 / ADR-0024）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `replaceStats`、
 * 画面は `StatsPanel.svelte`、入口は `CharacterList.svelte` の行。
 *
 * # ⚠ **これは3つ目のモードである**
 *
 * | モード | 送るもの | どこ |
 * |---|---|---|
 * | 作成 | 空の欄はキーごと落とす | `characterEdit.ts` の `buildCreate` |
 * | 編集 | **差分だけ**（キーの有無が「変えない」） | 同 `buildUpdate` |
 * | 修復 | **全部の欄**（元の値が読めないので差分を取れない） | `characterRepair.ts` |
 * | **置き換え** | **全軸**（**送らなかった軸は消える**） | **ここ** |
 *
 * **`PUT .../stats` は丸ごと置き換え**（`admin.rs` の `replace_stats`）。
 * **「変えない」を表す手段が無い**ので、**差分モードの作法を持ち込むと
 * 送らなかった軸が黙って消える。** 作法は `characterRepair.ts` 側に揃えてある
 * ——**「全部を必ず送る」。**
 *
 * # **軸を消す方法は「送らないこと」しかない**
 *
 * 削除の API は無い（`PUT` ただ1本）。したがって
 * **「外す」＝「本文に載せない」**であり、**それが破壊的な操作そのもの**である。
 * **画面はそれを押す前に言う**（`replaceSummary`）——
 * 言わなければ、**外したつもりのない軸が消えたことに、押した後まで気付けない。**
 *
 * # 写していないもの（**増やさないこと**）
 *
 * | 写していない | 代わりに |
 * |---|---|
 * | `StatScale::MIN` / `MAX`（尺度の範囲） | 尺度は応答の `stat_scale` を出すだけ |
 * | 値の上端（そのキャラクターの尺度） | 打たせて、サーバの `stat_value_above_scale` を見せる |
 * | `StatAxis::MAX_CHARS`（軸名の長さ） | 同上。**文字数計を持たない**ので写す理由も無い |
 * | `RECENT_DRIFT_VOTES`（直近の上限） | **応答の `recent_limit`** を出す |
 *
 * **⚠ 写しているのは `code` と向きの綴りだけ**（下の定数）。
 * どちらも**値の写し**なので、ずれると案内や見出しが黙って消える
 * ——**写し元を読む見張り番を `tests/stats-edit.test.ts` に置いた。**
 */

import { ApiRequestError } from "./api";
import type { DriftView, ReplaceStatsRequest, StatsView } from "./api";
// **時刻の綴りは1箇所**（作業単位 F-7）。記憶の一覧と同じ関数を通す。
import { timeLabel } from "./timestamp";

/**
 * 置き換えの下書きの1行。
 *
 * **`value` は文字列**（`<input>` の値そのもの）。数値へ直すのは送る直前で、
 * **直せなければ送らない**（`characterEdit.ts` と同じ立場）。
 */
export interface StatRow {
  /** 軸名。 */
  axis: string;
  /** 値。**空文字は「未入力」で、送れない**（0 とは別物）。 */
  value: string;
  /** **本文に載せない＝この軸は消える。** */
  removed: boolean;
  /** **サーバから来た行か。** この画面で足した行は `false`。 */
  stored: boolean;
}

/** 置き換えの下書き。 */
export interface StatsDraft {
  rows: StatRow[];
}

/** いまの値を下書きに写す。**値 0 の軸も落とさない。** */
export function draftOfStats(stats: StatsView): StatsDraft {
  return {
    rows: stats.stats.map((alloc) => ({
      axis: alloc.axis,
      value: String(alloc.value),
      removed: false,
      stored: true,
    })),
  };
}

/**
 * 足す行の初期値。**何も埋めない。**
 *
 * **`stored` が `false`** —— この行を外しても「軸が消える」ではない
 * （元から保存されていない）。`removedAxes` がその区別を使う。
 */
export function emptyRow(): StatRow {
  return { axis: "", value: "", removed: false, stored: false };
}

/** 送る行（外していない行）。 */
export function keptRows(draft: StatsDraft): StatRow[] {
  return draft.rows.filter((row) => !row.removed);
}

/**
 * **この置き換えで消える軸。**
 *
 * **保存済みの行を外したときだけ。** この画面で足した行を外しても、
 * 元から無いものは消えない ——**「消える」と言うと嘘になる。**
 *
 * # ⚠ **保存済みの行の軸名が変わらないことに寄りかかっている**
 *
 * 返しているのは `row.axis`（＝**いまの名前**）である。
 * **もし保存済みの行を改名できたら、改名した行は「消える軸」に一度も現れない** ——
 * 古い名前の軸は実際に送られなくなるのに、画面はそれを言わないことになる。
 *
 * **寄りかかっている先は `StatsPanel.svelte` の `{#if row.stored}` の1行**
 * （保存済みの行には軸名の欄を出さない）。**その欄を出すなら、
 * ここも「読み込んだときの名前」を別に持つ形へ直すこと。**
 * **見張り番は `tests/stats-panel.test.ts::`
 * `保存済みの軸名は書き換えられない（改名は「消して足す」でしかない）`**
 * ——改名で何が起きるかの実測（sqlite3）も、そちらの doc にある。
 */
export function removedAxes(draft: StatsDraft): string[] {
  return draft.rows
    .filter((row) => row.removed && row.stored)
    .map((row) => row.axis);
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltReplace =
  | { ok: true; body: ReplaceStatsRequest }
  | { ok: false; message: string };

/**
 * 値を 0 以上の整数として読む。
 *
 * # `characterEdit.ts` の `readScale` を使い回さない理由
 *
 * あちらは**空を「省略」として返す**（作成ならサーバの既定、編集なら「変えない」）。
 * **こちらに「省略」は無い** —— 置き換えに「変えない」は無く、
 * **空欄のまま送れば、その軸は 0 になるのか消えるのかが決まらない。**
 * **意味が違うものを同じ関数で読むと、片方の意味が黙って混ざる。**
 *
 * **上端は見ない**（そのキャラクターの尺度を写さない）。超えていれば
 * サーバが `stat_value_above_scale` を返す。
 */
function readValue(raw: string): { ok: true; value: number } | { ok: false } {
  const trimmed = raw.trim();
  // **`\d` は ASCII の数字だけ。** 全角の「１」は通さない
  // （`Number.parseInt` は全角を読まないので、通すと `NaN` が送られる）。
  if (!/^\d+$/.test(trimmed)) {
    return { ok: false };
  }
  const value = Number.parseInt(trimmed, 10);
  return Number.isSafeInteger(value) ? { ok: true, value } : { ok: false };
}

/** 値が読めなかったときの文言。**どの軸の話かを名乗る。** */
function valueMessage(axis: string): string {
  return `${axis} の値は 0 以上の整数で入力してください。`;
}

/** 軸名が空のときの文言。**「外す」はこの画面に実在する操作。** */
const AXIS_REQUIRED_MESSAGE =
  "軸名の空いている行があります。軸名を入れるか、その行を「外す」でください。";

/**
 * `PUT /admin/characters/{handle}/stats` の本文を組み立てる。**全軸を送る。**
 *
 * # ⚠ **ここで「変わった行だけ」に絞らないこと**
 *
 * 絞った瞬間、**絞られた軸は消える。**
 * `characterEdit.ts` の `buildUpdate`（差分だけ送る）とは**正反対の意味論**で、
 * 見た目が似ているぶん取り違えやすい。
 * **見張り番は `tests/stats-edit.test.ts::1軸だけ下げても、触っていない軸まで送る`。**
 *
 * # 外した行だけは載せない（**それが「消す」の唯一の表現**）
 *
 * 外した行の値が壊れていても、組み立ては止めない ——**送らない行だから。**
 *
 * # 値は生のまま送る
 *
 * 軸名は**正規化しない**（`StatAxis::parse` がサーバ側で行う /
 * `buildCreate` が表示名を生で送るのと同じ立場）。空かどうかの判定にだけ `trim()`。
 */
export function buildReplace(draft: StatsDraft): BuiltReplace {
  const stats: ReplaceStatsRequest["stats"] = [];
  for (const row of keptRows(draft)) {
    if (row.axis.trim() === "") {
      return { ok: false, message: AXIS_REQUIRED_MESSAGE };
    }
    const value = readValue(row.value);
    if (!value.ok) {
      return { ok: false, message: valueMessage(row.axis) };
    }
    stats.push({ axis: row.axis, value: value.value });
  }
  return { ok: true, body: { stats } };
}

/**
 * **保存すると何が起きるかを、押す前に言う文。**
 *
 * **丸ごと置き換えであることを、画面の側で表す唯一の場所。**
 * 残る軸と消える軸を**両方名乗る** ——
 * 片方だけだと、**「送らなかった軸が消える」という意味論が読み取れない。**
 */
export function replaceSummary(draft: StatsDraft): string {
  const kept = keptRows(draft).map((row) => row.axis.trim() || "（軸名未入力）");
  const removed = removedAxes(draft);
  const head =
    kept.length === 0
      ? "保存すると、このキャラクターの性格値は1軸も残りません。"
      : `保存すると、このキャラクターの性格値は ${kept.join("・")} の ${kept.length} 軸だけになります。`;
  if (removed.length === 0) {
    return head;
  }
  return `${head}外した ${removed.join("・")} は消えます（置き換えなので、送らない軸は消えます）。`;
}

/**
 * サーバが返す `code`（**`src/server/error.rs` の綴りの写し**）。
 *
 * # ⚠ **値の写しなので、ずれると案内が黙って消える**
 *
 * 綴りが1文字でも食い違えば、**どの枝にも入らず `unknown` に落ちる。**
 * **画面は理由も次の一手も言わなくなるのに、型検査もテストも緑のまま**
 * （`characterDelete.ts` の `HAS_SESSIONS_CODE` と同じ性質）。
 *
 * **写し元を読む見張り番は `tests/stats-edit.test.ts`**（`error.rs` を実際に読む）。
 */
export const VALUE_ABOVE_SCALE_CODE = "stat_value_above_scale";
/** 同上（`ApiError::stat_scale_changed`）。 */
export const SCALE_CHANGED_CODE = "stat_scale_changed";
/** 同上（`ApiError::duplicate_stat_axis`）。 */
export const DUPLICATE_AXIS_CODE = "duplicate_stat_axis";
/** 同上（`ApiError::character_not_found`）。 */
export const NOT_FOUND_CODE = "character_not_found";

/**
 * 変容の向きの綴り（**`src/server/admin.rs` の `delta_to_wire` の写し**）。
 *
 * **`delta_to_wire` は「API の語彙はログと DB の綴りとは別物で、揃えてあるだけ」**
 * と明言している。**こちらが写しているのは API の語彙の方。**
 * ずれると**向きの見出しが付かなくなる**（生の綴りは出るので、画面から消えはしない）。
 */
export const UP_DELTA = "up";
/** 同上。 */
export const DOWN_DELTA = "down";

/**
 * 向きの見出し。
 *
 * **知らない綴りは、そのまま返す。** サーバが向きを増やした日に
 * **画面から向きが消える**のがいちばん質の悪い壊れ方なので、訳せなくても落とさない
 * （`characterRepair.ts` の `columnLabel` / `MemoryList` の生の主語と同じ立場）。
 */
export function deltaLabel(delta: string): string {
  if (delta === UP_DELTA) {
    return "上げ";
  }
  if (delta === DOWN_DELTA) {
    return "下げ";
  }
  return delta;
}

/**
 * 失敗の種別。**`unknown` は「分類できなかった」であって「サーバの障害」ではない。**
 *
 * # なぜこの4つなのか
 *
 * **`admin.rs` の `replace_stats` が返しうる拒否のうち、
 * 「送り方を直せば通る」と読み違えると遠回りになるものだけ**を分ける。
 *
 * | `code` | 状態 | 直す相手 |
 * |---|---|---|
 * | `stat_value_above_scale` | 400 | **値か、尺度**（尺度はこの画面では変えられない） |
 * | `stat_scale_changed` | 409 | **取り直して送り直す**（サーバの障害ではない） |
 * | `duplicate_stat_axis` | 400 | **同じ軸名の行が2つある** |
 * | `character_not_found` | 404 | **そもそも居ない** |
 *
 * **軸名の拒否（`stat_axis_empty` など）は `unknown` に落とす。**
 * サーバの文言が**壊れている欄を自分で名乗る**ので、
 * こちらから言い換えると情報が減るだけになる（`characterRepair.ts` と同じ判断）。
 */
export type ReplaceFailureReason =
  | "value-above-scale"
  | "scale-changed"
  | "duplicate-axis"
  | "not-found"
  | "unknown";

/** 画面に出すもの。 */
export interface ReplaceFailure {
  reason: ReplaceFailureReason;
  /** **サーバが言ったこと。** 読めなかったときだけこちらの定型文になる。 */
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
 * **値が尺度を超えた**ときの一手。
 *
 * **⚠ 実在する導線だけを指す**（`characterDelete.ts` の ⚠ / B-15）。
 * **尺度を変える口はこの画面に無い** —— あるのは一覧の**「編集」**（`PATCH`）。
 * **見張り番は `tests/stat-scale-path.test.ts`**
 * （案内が「…」で名指しした操作が、実際に描かれていることを見る）。
 */
const VALUE_ABOVE_SCALE_GUIDANCE =
  "値をこのキャラクターの尺度以下にするか、先に尺度を上げてください。" +
  "尺度は一覧の「編集」から変えられます（この画面では変えられません）。";

/** **尺度が変わった**ときの一手。**諦めさせない**（送り直せば通る）。 */
const SCALE_CHANGED_GUIDANCE =
  "別の窓で尺度が変わったようです。「再読み込み」で取り直してから、もう一度保存してください。";

/** **同じ軸を2回指定した**ときの一手。 */
const DUPLICATE_AXIS_GUIDANCE =
  "同じ軸名の行が2つあります。片方を「外す」か、軸名を直してください。" +
  "（置き換え全体が取り消されているので、保存済みの値は変わっていません。）";

/** **居ない**と言われたときの一手。**取り直し**で通る。 */
const NOT_FOUND_GUIDANCE =
  "すでに消えているのかもしれません。「キャラクターの一覧へ」で取り直してください。";

/**
 * 失敗を、画面に出せる形へ。
 *
 * **`ApiRequestError` 以外は中身を画面へ流さない**（絶対制約:
 * エラーに内部の詳細を載せない）。
 */
export function describeReplaceFailure(failure: unknown): ReplaceFailure {
  if (!(failure instanceof ApiRequestError)) {
    return { reason: "unknown", message: "予期しない失敗です", guidance: null };
  }
  switch (failure.code) {
    case VALUE_ABOVE_SCALE_CODE:
      return {
        reason: "value-above-scale",
        message: failure.message,
        guidance: VALUE_ABOVE_SCALE_GUIDANCE,
      };
    case SCALE_CHANGED_CODE:
      return {
        reason: "scale-changed",
        message: failure.message,
        guidance: SCALE_CHANGED_GUIDANCE,
      };
    case DUPLICATE_AXIS_CODE:
      return {
        reason: "duplicate-axis",
        message: failure.message,
        guidance: DUPLICATE_AXIS_GUIDANCE,
      };
    case NOT_FOUND_CODE:
      return {
        reason: "not-found",
        message: failure.message,
        guidance: NOT_FOUND_GUIDANCE,
      };
    default:
      return { reason: "unknown", message: failure.message, guidance: null };
  }
}

/**
 * **割り当ての無い軸の見出し**（作業単位 B-16）。
 *
 * サーバは `value: null` を返す（`api.ts` の `DriftAxisView.value` の doc）。
 * **`0` にも、生の `null` にもしない** ——
 * `0` は値 0 の軸と混ざり、`null` はそのまま埋め込むと **`null / 7`** と出る。
 */
export const UNALLOCATED_LABEL = "割り当てなし";

/**
 * 変容の表の「値」の欄。**綴りを決めるのはここだけ。**
 *
 * # 数を出さない理由（割り当てが無いとき）
 *
 * **どんな数も嘘になる。** `0` は「この軸は語らない」という**人の宣言**
 * （ADR-0024 §4 / 変容の対象外）と同じ見え方になり、
 * 尺度だけを出す（`なし / 7`）と**尺度が意味を持つように読める。**
 *
 * 見張り番は `tests/stats-edit.test.ts` の
 * `値 0 と割り当てなしが、同じ見出しにならない`（**2つを並べて比べる**）と、
 * `tests/stats-panel.test.ts` のセルの配列（**読み手が実際に見る値**）。
 */
export function axisValueLabel(value: number | null, scale: number): string {
  if (value === null) {
    return UNALLOCATED_LABEL;
  }
  return `${value} / ${scale}`;
}

/**
 * **累積の行が無い軸の見出し**（作業単位 D-5a）。
 *
 * **[`UNALLOCATED_LABEL`] と同じ文字列にしないこと。**
 * 2つは別のことを言っている——
 * あちらは **`stat_alloc` に行が無い**（軸が改名された）、
 * こちらは **累積の行が無い**（一度も適用まで届いていない）。
 * 同じ綴りにすると、表の2つの列が同じことを言っているように読める。
 */
export const NO_CHARGE_TIME_LABEL = "記録なし";

/**
 * 変容の表の「累積が動いた日時」の欄。**綴りを決めるのはここだけ。**
 *
 * # `null` を数で埋めない
 *
 * サーバは**累積の行が無い軸に `null`** を返す（`api.ts` の `charge_updated_at`）。
 * **`0` でも現在時刻でも埋めない** ——
 * 現在時刻で埋めると **「たったいま動いた」** と読め、
 * **この欄の存在理由（いつ動いたのかを読む）が丸ごと嘘になる。**
 *
 * # 時刻の綴り（**作業単位 F-7 → F-7b**）
 *
 * ~~サーバが返す RFC 3339 をそのまま出す~~
 * → **`T` / `Z` / ミリ秒を畳んで、時間帯を名乗る**（`timestamp.ts` の `timeLabel`）。
 * ~~**地域化はしない。**~~
 * → **F-7b で覆った** —— **読み手の時間帯で出す。**
 * **ログや `sqlite3` の UTC へは、名乗っている時差から引き算で戻せる。**
 * 理由は `timestamp.ts` の doc が持つ ——**ここに写さない。**
 *
 * # ⚠ 時間帯は引数で受け取る
 *
 * **ここで [`browserZone`] を呼ばないこと。** 呼ぶと、
 * **この関数を見る検査が、走らせた機械の時間帯でしか成立しなくなる**
 * （F-7 が地域化を避けた2つ目の理由そのもの）。
 * **解決するのは画面**（`StatsPanel.svelte`）。
 */
export function chargeTimeLabel(at: string | null, zone: string): string {
  if (at === null) {
    return NO_CHARGE_TIME_LABEL;
  }
  return timeLabel(at, zone);
}

/**
 * 変容の走らせ方の綴り（**`src/domain/drift.rs` の `StatDriftMode::label` の写し**）。
 *
 * **ログの欄・環境変数の綴り・この応答が同じ語**なので、
 * **画面に出た語をそのまま `PERSONACORE_STAT_DRIFT` へ書き戻せる。**
 * ずれると見出しも案内も消えるので、
 * **写し元を読む見張り番を `tests/stats-edit.test.ts` に置いてある。**
 */
export const RUN_MODE_OFF = "off";
/** 同上。**判定して票は残すが、値は動かさない。** */
export const RUN_MODE_SHADOW = "shadow";
/** 同上。**判定し、票を残し、値も動かす。** */
export const RUN_MODE_ON = "on";

/**
 * 走らせ方の見出し。
 *
 * **知らない綴りは、そのまま返す**（`deltaLabel` と同じ立場）。
 * サーバが4つ目の走らせ方を足した日に、
 * **画面から走らせ方の欄が消えるのがいちばん質の悪い壊れ方**である。
 *
 * # 3つを別の文字列にする
 *
 * **`shadow` と `on` の違いは「値が動くかどうか」だけ**で、どちらも票は残る。
 * 同じ見出しにすると、**画面から「値が動いているのか」が読めなくなる**
 * ——そして**変容は本文にもユーザーにも出ない**（ADR-0024 §9）ので、
 * 読める場所が他に無い。
 */
export function runModeLabel(mode: string): string {
  if (mode === RUN_MODE_OFF) {
    return "停止中（判定も記録もしていません）";
  }
  if (mode === RUN_MODE_SHADOW) {
    return "記録のみ（票は残りますが、値は動きません）";
  }
  if (mode === RUN_MODE_ON) {
    return "実行中（票を残し、値も動かします）";
  }
  return mode;
}

/**
 * **走らせ方の出どころ**（`src/domain/drift.rs` の `DriftModeSource::label` の写し /
 * 作業単位 D-15 / ADR-0045 §決定4）。**キャラクター自身の設定**。
 *
 * ずれると出どころの一言と案内の言い分けが黙って外れるので、
 * **写し元を読む見張り番を `tests/stats-edit.test.ts` に置いてある。**
 */
export const RUN_MODE_SOURCE_CHARACTER = "character";
/** 同上。**起動時の既定**（キャラクターは未設定）。 */
export const RUN_MODE_SOURCE_DEFAULT = "default";

/**
 * **この画面に実在する、編集への導線の見出し**（`StatsPanel.svelte` のヘッダのリンク）。
 *
 * 案内が `「」` で名指しするのはこれだけ。**走らせ方の編集そのものはこの画面に無い**ので、
 * 編集はここから辿れると言う（名指しした導線が実在することは
 * `tests/stats-panel.test.ts` の `assertNoticeNamesOnlyRealOperations` が見る）。
 */
export const CHARACTER_LIST_LINK_LABEL = "キャラクターの一覧へ";

/**
 * **走らせ方の出どころを言う一言**（作業単位 D-15）。**走らせ方によらず出す。**
 *
 * `shadow` / `on` でも出す —— 出どころを黙ると、**どこで切り替えるかが
 * 止まっているときにしか読めない。**
 *
 * **知らない綴りは、そのまま出す**（推測しない / `runModeLabel` と同じ立場）。
 */
export function runModeSourceNote(source: string): string {
  if (source === RUN_MODE_SOURCE_CHARACTER) {
    return "この走らせ方は、このキャラクター自身の設定です（キャラクターの編集で変えられます）。";
  }
  if (source === RUN_MODE_SOURCE_DEFAULT) {
    return (
      "このキャラクターは走らせ方を選んでいないので、起動時の既定に従っています" +
      "（キャラクターの編集で、このキャラクターだけの走らせ方を選べます）。"
    );
  }
  return `走らせ方の出どころ: ${source}`;
}

/**
 * **停止中であることと、それが「0 件」の理由であることを言う。** それ以外は `null`。
 *
 * # なぜ言うのか
 *
 * **サーバ側の既定は `off`**（`admin.rs` の `DriftView::run_mode`）で、
 * `off` は上流を呼ばず、判定もせず、**票も累積も1行も作らない。**
 * **つまり既定の配置でこの画面を開くと、
 * 「総数 0 件・全軸の累積 0・票の一覧が空」が出る**——
 * **変容が壊れている状態と、画面上で1ドットも違わない。**
 *
 * # どこで切り替えられるかを、出どころで言い分ける（作業単位 D-15 / ADR-0045）
 *
 * **以前は、止まっていれば必ず環境変数（`PERSONACORE_STAT_DRIFT`）を案内していた**
 * ——走らせ方は起動設定で、切り替える口が画面にも管理 API にも無かったから。
 * **いまはキャラクターの編集で切り替えられ、キャラクターの値が環境変数に勝つ。**
 * **キャラクター自身が「止める」を選んでいる回に環境変数を案内すると、
 * 効かない操作を名指しすることになる**（origin#31 のレビュー）。
 *
 * | 出どころ | 言うこと |
 * |---|---|
 * | `default` | 起動時の既定に従っていること・編集で動かせること・**環境変数は補足**（測定する人向け） |
 * | `character` | このキャラクター自身の設定であること・編集で変えられること（**環境変数は言わない**） |
 * | 知らない綴り | 出どころについて推測せず、編集で変えられることだけを言う |
 *
 * # ⚠ 無い操作を名指ししない（`B-15` / `B-16` / `D-6` / `D-7` / `D-8` で5回出た形）
 *
 * **走らせ方の編集は、この画面には無い。** `「」` で名指しするのは
 * **この画面に実在する導線 [`CHARACTER_LIST_LINK_LABEL`] だけ**で、
 * 編集はそこから辿れると言う（選択肢の名前も `「」` で括らない —— 選択肢は編集の画面にある）。
 * 見張り番は `tests/stats-panel.test.ts` の `assertNoticeNamesOnlyRealOperations`。
 *
 * # `shadow` と `on` には出さない
 *
 * どちらも**判定は走っている**ので、「動いていません」と書くと嘘になる。
 * **知らない綴りにも出さない**——何が起きているか分からないので、推測しない。
 */
export function runModeNotice(mode: string, source: string): string | null {
  if (mode !== RUN_MODE_OFF) {
    return null;
  }
  const reason =
    "この設定のあいだは判定も記録も行われないので、" +
    "下の票と累積はこれ以上増えません（0 件なのは、壊れているからではありません）。";
  const where = `「${CHARACTER_LIST_LINK_LABEL}」から、このキャラクターの編集を開いて`;
  if (source === RUN_MODE_SOURCE_DEFAULT) {
    return (
      "変容はいま停止しています。このキャラクターは走らせ方を選んでいないので、" +
      "起動時の既定（停止）に従っています。" +
      reason +
      // **「動かす」と「記録だけ」の結果を別々に言う**（D-15 のレビュー）——
      // 記録だけは値を動かさない。選択肢の名前は `「」` で括らない
      // （選択肢は編集の画面にあり、この画面には無い / 見張り番の主張）。
      `${where}、変容の走らせ方で動かすを選ぶと、次のやり取りから値が動き、` +
      "記録だけを選ぶと、判定と記録だけが始まります（値は動きません）。" +
      "（起動時の既定は、サーバの環境変数 PERSONACORE_STAT_DRIFT で決まります。）"
    );
  }
  if (source === RUN_MODE_SOURCE_CHARACTER) {
    return (
      "変容はいま停止しています。このキャラクターは止める設定になっています" +
      "（起動時の既定ではなく、このキャラクター自身の設定です）。" +
      reason +
      `${where}、変容の走らせ方を変えると、次のやり取りから動きます。`
    );
  }
  return (
    "変容はいま停止しています。" +
    reason +
    `走らせ方は、${where}変えられます。`
  );
}

/**
 * **軸が1本も無いことと、それが「0 件」の理由であることを言う。** それ以外は `null`。
 *
 * # なぜ言うのか（作業単位 F-6）
 *
 * **サーバは、軸が1本も無い回を上流に投げない** ——
 * `build_drift_prompt` が `None` を返し、`drift_skipped=no_axes` として数える
 * （`src/server/drift.rs` の `DriftSkipReason::NoAxes`）。
 * **答えの決まっている問いに GPU を使わない**ので、これは正しい振る舞いである。
 *
 * **しかし画面はそれを言っていなかった** ——
 * **走らせ方「実行中（票を残し、値も動かします）」の真下に「まだ1件も票がありません」**
 * が並ぶ。**2つが理由で繋がっていないので、壊れているようにしか読めない。**
 * **新しいキャラクターは必ずこの状態から始まる。**
 *
 * **[`runModeNotice`] が理由を言うのは `off` のときだけ**だった。
 * これは `D-5a` が「止めてある」と「壊れている」を分けた、**その次の穴**である。
 *
 * # ⚠ 走らせ方を見ない
 *
 * **軸が無ければ、どの走らせ方でも判定は走らない。**
 * 走らせ方で文を変えると、**「実行中だから動いている」と読める文が生まれ、
 * F-6 の壊れ方がそのまま残る**（見張り番は
 * `走らせ方が実行中でも、記録のみでも、停止中でも同じことを言う`）。
 *
 * # ⚠ 拾うのは「行が1つも無い」ときだけ
 *
 * **割り当てを失った軸（`value === null`）は「軸がある」側**である ——
 * 行は変容の記録に居るし、`Store::stat_allocs` の許可リストにも綴りが残る。
 * ここで拾うと、**軸が居るのに「1本もありません」と書く。**
 *
 * # ⚠ 名指しする操作は、この画面に実在するものだけ
 *
 * 直し方は **「軸を追加」→「この内容で置き換える」** の2手だが、
 * **その2つは置き換えのフォームの中にしか無い**（`unallocatedNotice` の doc）。
 * `GET .../stats` が失敗した回は描かれないので、`replaceAvailable` が `false` なら**名指ししない。**
 *
 * **⚠ それでも黙らない。** 理由そのものは**その状態でも真**であり、
 * **黙ると F-6 の穴が失敗経路でだけ残る。**
 * （`unallocatedNotice` が丸ごと黙るのは、**あちらの手順がその状態で成立しない**から
 * ——「いま保存されている他の軸が読めないので、そのとおりにすると残りが全部消える」。
 * **こちらは軸が1本も無いので、消える残りが存在しない。**）
 */
export function noAxesNotice(
  drift: DriftView,
  replaceAvailable: boolean,
): string | null {
  if (drift.axes.length > 0) {
    return null;
  }
  const reason =
    "このキャラクターには軸が1本もありません。" +
    "軸が無いあいだは、走らせ方に関わらず変容の判定そのものが行われないので、" +
    "票も累積も1件も増えません（0 件なのは、壊れているからではありません）。";
  if (!replaceAvailable) {
    return reason;
  }
  return (
    reason +
    "上の「軸を追加」で軸を足し、「この内容で置き換える」で保存すると、" +
    "次の会話から判定の対象になります。"
  );
}

/**
 * **割り当ての無い軸の綴り**を、応答の順のまま返す。
 *
 * 並べ替えない —— **順を決めているのはサーバ**（`admin.rs` の `drift_view`:
 * 割り当てのある軸が先、無い軸は軸名の昇順）。
 * ここで並べ替えると、表の並びと案内の並びが食い違う。
 *
 * **値 0 の軸を拾わないこと。** 拾うと、
 * 人が「この軸は語らない」と宣言しただけの軸に「消えている」と言うことになる。
 */
export function unallocatedAxes(drift: DriftView): string[] {
  return drift.axes
    .filter((axis) => axis.value === null)
    .map((axis) => axis.axis);
}

/**
 * **消えた軸があることと、直し方を言う。** 1本も無ければ `null`。
 *
 * # なぜ言うのか
 *
 * `PUT .../stats` は**丸ごと置き換え**なので、**軸を改名すると綴りが変わる。**
 * 累積と票は残るが、いまの値は無い。**読み手には「知らない軸が出てきた」としか見えず、
 * 何が起きたのかも、直せるのかも分からない。**
 *
 * # ⚠ 名指しする操作は、この画面に実在するものだけ
 *
 * 直す手段は「**同じ綴りで置き直す**」しかない ——
 * **票にも累積にも、消す口がサーバに無い**（追記のみ / ADR-0026 §決定3）。
 * その手順はこの画面の **「軸を追加」** と **「この内容で置き換える」** で辿れる。
 *
 * **見張り番は `tests/stat-scale-path.test.ts`**
 * （`「」` で名指しした操作が、実際に描かれた操作の中にあることを機械的に見る）。
 *
 * # ⚠ **`replaceAvailable` が要る理由**（レビューの実測 / 2026-08-28）
 *
 * **2つの読みは独立している。** `GET .../stats` が 500 でも
 * `GET .../drift` が 200 なら、**変容の表は出るが置き換えのフォームは描かれない**
 * （`StatsPanel` の `statsState.kind === "failed"` の枝）。
 *
 * **その状態で案内を出すと、名指しした2つの操作がどちらも画面に無い。**
 * 実測された画面の操作は **`["再読み込み", "キャラクターの一覧へ"]` の2つだけ**だった。
 * **`D-7` / `B-15` で潰した「無い操作を案内する」が、失敗経路で復活していた形。**
 *
 * **だから出す条件を、名指しする操作が描かれる条件に揃える。**
 * 置き換えのフォームが無いときは黙る。
 *
 * **⚠ 黙る理由が変わった**（2026-08-28 / 作業単位 D-8）。
 * **その状態には修復のフォームができた**（`StatsRepairForm.svelte`）ので、
 * 「読み手にできることが1つも無い」はもう事実ではない。
 * **それでも黙る** —— **この案内が言う手順が、その状態では成立しないから。**
 * 手順は「同じ綴りで足して置き換える」だが、**修復のフォームは
 * いま保存されている他の軸を1本も知らない。**
 * **そのとおりにすると、書かなかった軸が全部消える。**
 * **「辿れる操作がある」ことと「その手順が正しい」ことは別**である。
 *
 * **⚠ したがって、修復のフォームの操作名を
 * 「軸を追加」「この内容で置き換える」と揃えないこと**
 * （揃えると、`replaceAvailable` を `true` へ変える変異が
 * `assertNoticeNamesOnlyRealOperations` を素通りする）。
 *
 * **⚠ 「割り当てなし」の行そのものは、この状態でも出ている。**
 * 消えているのは**手順の案内だけ**で、事実は表に残る。
 *
 * # ⚠ 軸名を `「」` で囲まないこと
 *
 * その見張り番は **`「」` の中身を操作として取り出す。**
 * 軸名を入れると**軸名が実在する操作かどうかを問われて赤になる。**
 * 軸名は `『』` で囲む（`tests/stats-edit.test.ts` が綴りごと固定している）。
 */
export function unallocatedNotice(
  drift: DriftView,
  replaceAvailable: boolean,
): string | null {
  const lost = unallocatedAxes(drift);
  if (lost.length === 0 || !replaceAvailable) {
    return null;
  }
  const named = lost.map((axis) => `『${axis}』`).join("");
  return (
    `${named}は、いまの値がありません（置き換えで軸名が変わったあとも、` +
    "累積と票だけが残っています）。" +
    "同じ綴りで「軸を追加」して「この内容で置き換える」と、この記録に戻ります。" +
    "票と累積を消す手段はありません（記録は追記だけです）。"
  );
}
