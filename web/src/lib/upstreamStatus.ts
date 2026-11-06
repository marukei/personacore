/**
 * **「いまの上流」の面ごとのカード・状態の印・異常の帯・照らし合わせの見出し**
 * （作業単位 D-12b-2 / Issue origin#93）。**I/O を持たない。**
 *
 * # 何を決めるか
 *
 * `UpstreamPanel.svelte` の観測の節は、面ごとのカード（本文生成・裏方）に、
 * **状態の印**・上流が名乗ったモデル名・起こし方・宛先・コンテキスト長を出す。
 * 異常は**目立つ帯**で知らせ、突き合わせの表と注記は `<details>`（照らし合わせ）に畳む（常に閉じて出す）。
 * **どの値から何を決めるかを、ここ1箇所に置く**（`.svelte` に散らすと、取り違えの変異を1本で押さえられない）。
 *
 * # 状態の印（[`faceStatus`]）
 *
 * **2つの事実を別の欄から読む** —— `launch.state` は「この実行ファイルが何をしたか」、
 * `answered_by` / `unauthorized` は「上流が最後の照会に何と答えたか」（`api.ts` の `LaunchView` の ⚠）。
 *
 * | `launch.state` | 上流の答え | 印 | 帯 |
 * |---|---|---|---|
 * | `not_started` | どれでも | 起こしていません | なし（起こすボタンが聞く） |
 * | `loading` | どれでも | 載せています | なし（読み直し続ける） |
 * | `failed` | どれでも | 失敗しました | **あり** |
 * | `stopped` | どれでも | 止めました | **あり** |
 * | `ready` / `external` | `key_over_plain_http` が `not_approved` | 鍵を送っていません | **あり** |
 * | `ready` / `external` | `unauthorized` | 鍵を断られました | **あり** |
 * | `external` | `answered_by` が `null` で `check_result` が `no_answer` | **繋がりません** | **あり**（作業単位 B-66） |
 * | `external` | `answered_by` が `null` で `check_result` が `no_model_list` | 一覧に答えません（**中立の色** / レビュー是正） | なし |
 * | `external` | `answered_by` が `null` で `check_result` が `not_checked` | まだ確かめていません（**中立の色**） | なし |
 * | `ready` / `external` | `answered_by` が `null`（上の2つ以外） | 名乗りがありません（**中立の色**） | なし（下の ⚠） |
 * | `ready` | 答えた | 動いています | なし |
 * | `external` | 答えた | 答えています | なし |
 * | 知らない綴り | どれでも | **その綴りのまま** | なし |
 *
 * ## ⚠ 起こした子が動いていない間は、上流の答えを読まない
 *
 * 起こしていない・載せている・失敗・止めたの4つでは、上流の答えは**動いていない子についての空か古い値**である
 * （差し替えの直後は「一度も聞いていない」観測が入る / `upstream.rs` の `unqueried`）。
 * そこから「鍵を断られた」「名乗りが無い」を言うと、**起こした状態と食い違う2つ目の主張**になる。
 *
 * ## ⚠ ありえない組み合わせでは、異常の側の事実を落とさない
 *
 * `unauthorized` は `answered_by: null` と組で読む欄（ADR-0056 §決定4）。**断られたのに答えた読み手がある**
 * 組み合わせが届いたら、**断られたことを言う**（答えたほうを採って「答えています」と言うと、断られた事実が画面から消える）。
 *
 * **送らなかった（`not_approved`）のに断られた**（`unauthorized`）組み合わせは、**送らなかったほうを言う**
 * （作業単位 A-13b-3）。どちらも異常だが帯は1面に1本なので、片方を選ぶ。送らなかったのなら、断られたのは
 * それより前の観測で、**いま直すのは承認か宛先**である（「鍵を確かめて」と言うと、直しても通らない場所へ向ける）。
 *
 * # 鍵の送り方（`key_over_plain_http` / 作業単位 A-13b-3 / ADR-0056 §決定5）
 *
 * | 値 | 出し方 |
 * |---|---|
 * | `not_approved` | **印と異常の帯**（送信を止めている） |
 * | `local_network` / `approved` | **カードの印**（[`keyMark`]。暗号化せずに送っているが、止まってはいない） |
 * | 欄が無い | 何も出さない |
 *
 * ## ⚠ 知らない綴りでは、異常とも正常とも言わない
 *
 * サーバが7つ目の状態を足した日に、それが「動いている」側か「動いていない」側かを画面は知らない。
 * **綴りのまま出し、帯を出さない**（`bodyFace.ts` の「知らない綴りで止めない」と同じ立場）。
 * 詳しい文面は、カードの「起こした上流の状態」の行（`launchLabel` —— 知らない綴りはそのまま）が言う。
 *
 * # 照会したが答えない外の上流だけを、異常と言う（作業単位 B-66 / Issue origin#94）
 *
 * **D-12b-2 の2周目から B-66 までは、名乗りが無いことを異常と言わなかった**（帯にせず、色も中立）。
 * 当時サーバはコンテキスト長を設定した面に照会せず、そのときも `answered_by` は `null`、`unauthorized` は `false` で、
 * **止まっている上流とまったく同じ形で届いた**から —— 利用者の普段の構成（コンテキスト長を書いた繋ぐ面）で帯が出続ける。
 *
 * **B-66 でサーバが書いた面にも照会し、`check_result` で「照会していない」と「照会したが答えない」を言い分けるようになった。**
 * いまは**外の上流（`external`）で `no_answer` のときだけ帯**（「繋がりません」）にし、**`not_checked` は中立**
 * （「まだ確かめていません」）にする。**起こした面（`ready`）の振る舞いは変えない**（名乗りが無ければ中立のまま —— 決定4）。
 * **知らない結果の綴りでは、異常とも言わない**（「名乗りがありません」の中立のまま）。
 * **書いたプロファイルの値で判定しない** —— 書いた値と観測した値を混ぜない（Issue origin#24）。
 *
 * # 照らし合わせの見出し（[`comparisonSummary`]）
 *
 * **照らし合わせは常に閉じて出し、見出しが一致しない行（`drifted_other` + `drifted_absent`）の数を面ごとに言う。**
 * **聞けなかった行（`unanswered`）は数えない**（上流が黙っている・コンテキスト長を設定した面の事実で、ずれではない）。
 *
 * **⚠ 一致しないことで自動に開かない・帯にもしない。** `llama-server` を上流にすると、一覧が gguf の名前1件なので
 * **定数の行が全部「一覧に無い」になる**（`src/measured.rs` の `absent_groups` の doc / 作業単位 G-5）。
 * 開く・帯にすると、普段の使い方でずっとそうなる（毎回出る報せは読まれなくなる）。**開閉は利用者の操作だけで変わる。**
 */

import type { LaunchView, UpstreamReport, UpstreamView } from "./api";
import { instantOf, wallTimeOf } from "./timestamp";
import {
  CHECK_RESULT,
  NOT_ASKED_LABEL,
  UNDECIDED_ANSWER_LABEL,
  UNNAMED_ANSWER_LABEL,
  contextTokensLabel,
  notAskedYet,
} from "./upstream";
import { KEY_OVER_PLAIN_HTTP } from "./upstreamKey";
import { FACES, LAUNCH_STATE, faceKindLabel, faceLabel, type Face, type FaceKind } from "./upstreamProfiles";

/**
 * **印の色の種類。** 画面はこれで見た目を選ぶ（色の値は持たない）。
 *
 * | 種類 | 見た目（`UpstreamPanel.svelte`） |
 * |---|---|
 * | `running` | 読む側の地に塗った印 |
 * | `loading` | 読む側の枠だけの印 |
 * | `waiting` | 罫の枠の、色を持たない印 |
 * | `alarm` | 壊れている側（暖色）の地に塗った印 |
 * | `quiet` | 罫の枠の、色を持たない印（名乗りが無い。異常とも正常とも言わない） |
 * | `unknown` | 罫の枠の、色を持たない印（`waiting` と同じ見た目。異常とも正常とも言わない） |
 */
export type FaceTone = "running" | "loading" | "waiting" | "alarm" | "quiet" | "unknown";

/**
 * **帯を出す理由**（失敗・止めた・鍵を断られた・鍵を送っていない・繋がらない の5つだけ）。`null` は帯を出さない。
 *
 * **鍵を送っていない（`key_not_approved`）は作業単位 A-13b-3 で足した**（`key_over_plain_http` の `not_approved`）。
 * **繋がらない（`no_answer`）は作業単位 B-66 で足した**（外の上流に照会したが答えない / `check_result` の `no_answer`）。
 */
export type FaceAlarm = "failed" | "stopped" | "unauthorized" | "key_not_approved" | "no_answer";

/** 1面の状態の印。 */
export interface FaceStatus {
  /** 印の文面。 */
  readonly badge: string;
  readonly tone: FaceTone;
  /** 帯を出す理由（異常でなければ `null`）。 */
  readonly alarm: FaceAlarm | null;
}

/**
 * **鍵を断られたときの帯の文面**（作業単位 A-13b-1 で足した `unauthorized` を読む）。
 *
 * **作業単位 A-13b-3 で差し替えた**（D-12b-2 の間は「推論サーバが鍵を受け付けませんでした。」の仮の文言）。
 * 鍵の欄ができたので、**直す場所**（下の節のプロファイルの「直す」）を名乗る。節の見出しの綴りは
 * `UpstreamProfiles.svelte` の `h2` と同じ（見張り番は `tests/upstream-panel-cards.test.ts` の、帯が名指す節が同じ画面に在ること）。
 * **画面のほかの場所にこの綴りを写さないこと**（直す場所を1つに保つ）。
 *
 * **作業単位 B-69（Issue origin#98）で、鍵のほかの候補を足した** —— 上流の側で鍵を有効にしていない・宛先が違う
 * （末尾の `/v1` の二重。宛先違いで上流が 401 を返すかは利用者の操作順からの推論で、単独では測っていない ——
 * origin#98 のコメント。だから文面では言い切らない）。直す場所は鍵の欄と宛先で、
 * **保存したら「切り替え直す」まで言う**（保存だけでは、動いている上流は変わらない）。
 */
export const UNAUTHORIZED_BAND_MESSAGE =
  "推論サーバが鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。下の「upstreams.json に書いたプロファイル」の「直す」でこの面の鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。";

/**
 * **承認が無いので鍵を送らなかったときの帯の文面**（作業単位 A-13b-3 / `key_over_plain_http` の `not_approved`）。
 *
 * **断られたとは言わない**（上流は鍵を見ていない）。直すのは承認か宛先（`https://`）で、鍵ではない。
 */
export const KEY_NOT_APPROVED_BAND_MESSAGE =
  "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、鍵を送っていません（推論サーバは鍵を見ていません）。下の「upstreams.json に書いたプロファイル」の「直す」で承認するか、宛先を https:// にしてください。";

/**
 * 載せられなかったときの帯の文面。**理由は繰り返さない**（カードの「起こした上流の状態」が理由とログの名前まで言う）。
 */
const FAILED_BAND_MESSAGE = "載せられませんでした。理由は、この面のカードの「起こした上流の状態」にあります。";

/** 止めたときの帯の文面（`launch.rs` の `FaceState::Stopped` —— この実行ファイルが終わるときに止めた）。 */
const STOPPED_BAND_MESSAGE = "この実行ファイルが終わるときに止めました。";

/**
 * **1面の状態の印を決める**（冒頭の表）。
 */
export function faceStatus(view: UpstreamView): FaceStatus {
  switch (view.launch.state) {
    case LAUNCH_STATE.NotStarted:
      return { badge: "起こしていません", tone: "waiting", alarm: null };
    case LAUNCH_STATE.Loading:
      return { badge: "載せています", tone: "loading", alarm: null };
    case LAUNCH_STATE.Failed:
      return { badge: "失敗しました", tone: "alarm", alarm: "failed" };
    case LAUNCH_STATE.Stopped:
      return { badge: "止めました", tone: "alarm", alarm: "stopped" };
    case LAUNCH_STATE.Ready:
      return answeredStatus(view, "動いています", false);
    case LAUNCH_STATE.External:
      return answeredStatus(view, "答えています", true);
    default:
      return { badge: view.launch.state, tone: "unknown", alarm: null };
  }
}

/**
 * **答えが期待できる面**（準備完了・外の上流）の印。**送っていない → 断られた → 答えた → 答えない**の順に見る。
 *
 * **答えないときの言い方は、外の上流だけ `check_result` で割る**（作業単位 B-66 / 冒頭の ⚠）:
 *
 * | 面 | `check_result` | 印 |
 * |---|---|---|
 * | 外の上流 | `no_answer`（照会したが繋がらない・黙っている） | **繋がりません**（異常の帯） |
 * | 外の上流 | `no_model_list`（届いたが一覧が読めない） | **一覧に答えません**（中立 / レビュー是正） |
 * | 外の上流 | `not_checked`（まだ照会していない） | **まだ確かめていません**（中立） |
 * | 外の上流 | それ以外・知らない綴り | 名乗りがありません（中立。今までのまま） |
 * | 準備完了（起こした面） | どれでも | 名乗りがありません（中立。**起こす面の振る舞いは変えない** —— 決定4） |
 *
 * **`no_model_list` を帯にしない理由**: モデル一覧は上流に「任意」の能力（ADR-0038 §決定1）。一覧の口を持たない
 * 上流にコンテキスト長を書いて繋ぐと、会話は通るのに照会は毎回失敗する —— 帯にすると出続ける（D-12b-2 の2周目が避けた形）。
 */
function answeredStatus(view: UpstreamView, answering: string, external: boolean): FaceStatus {
  if (view.key_over_plain_http === KEY_OVER_PLAIN_HTTP.NotApproved) {
    return { badge: "鍵を送っていません", tone: "alarm", alarm: "key_not_approved" };
  }
  if (view.unauthorized) {
    return { badge: "鍵を断られました", tone: "alarm", alarm: "unauthorized" };
  }
  if (view.answered_by !== null) {
    return { badge: answering, tone: "running", alarm: null };
  }
  if (external && view.check_result === CHECK_RESULT.NoAnswer) {
    return { badge: "繋がりません", tone: "alarm", alarm: "no_answer" };
  }
  if (external && view.check_result === CHECK_RESULT.NoModelList) {
    return { badge: "一覧に答えません", tone: "quiet", alarm: null };
  }
  if (external && notAskedYet(view)) {
    return { badge: NOT_ASKED_LABEL, tone: "quiet", alarm: null };
  }
  return { badge: "名乗りがありません", tone: "quiet", alarm: null };
}

/**
 * **確かめた時刻**（作業単位 B-66 のレビュー是正）。読み手の時間帯（`zone`）の `HH:MM:SS` で言う。
 * **`null` は「まだ照会していない」**（呼び出し側が言う）。
 *
 * **経過は出さない。** 1周目は「N 分前」だったが、画面は載せている面があるときしか読み直さないので、
 * 普段の構成では「1 分以内に確かめました」が2時間後も出続けた。**時刻なら、読み直さなくても嘘にならない。**
 * 時刻の読み方は `launchLabel` の `since` と同じ（`timestamp.ts` の `instantOf` / `wallTimeOf`）。
 * **読めない綴り・読めない時間帯では、届いた綴りをそのまま出す。**
 */
export function checkedAtLabel(checkedAt: string | null, zone: string): string | null {
  if (checkedAt === null) {
    return null;
  }
  const at = instantOf(checkedAt);
  const wall = at === null ? null : wallTimeOf(at, zone);
  return `${wall ?? checkedAt} に確かめました`;
}

/** **最後の照会の結果の文面**（作業単位 B-66）。知らない綴りはそのまま出す。 */
function checkResultLabel(result: string): string {
  switch (result) {
    case CHECK_RESULT.Answered:
      return "答えました";
    case CHECK_RESULT.NoAnswer:
      return "繋がりません";
    case CHECK_RESULT.NoModelList:
      return "一覧に答えません";
    case CHECK_RESULT.Refused:
      return "鍵を断られました";
    case CHECK_RESULT.KeyNotSent:
      return "鍵を送っていません";
    default:
      return result;
  }
}

/**
 * **カードの「最後の確認」の行**（作業単位 B-66 / Issue origin#94 の決定4）。
 *
 * **まだ照会していない面は時刻を言わない**（「まだ確かめていません」）。照会した面は、時刻と結果を並べる
 * （`10:03:30 に確かめました（答えました）` / 時刻は読み手の時間帯）。
 */
export function checkLabel(view: UpstreamView, zone: string): string {
  if (view.check_result === CHECK_RESULT.NotChecked) {
    return NOT_ASKED_LABEL;
  }
  const at = checkedAtLabel(view.checked_at, zone);
  const result = checkResultLabel(view.check_result);
  return at === null ? result : `${at}（${result}）`;
}

/**
 * **起こし方**: `external` なら繋ぐ、ほかの知っている状態なら起こす。**知らない綴りは `null`**（どちらとも言わない）。
 *
 * **観測から決める**（書いたプロファイルの `gguf` から決めない —— 書いた値と観測した値を混ぜない）。
 * `--upstream` を渡していない起動は、どちらの面も `external`（`api.ts` の `UpstreamView.launch`）。
 */
export function launchKindOf(launch: LaunchView): FaceKind | null {
  switch (launch.state) {
    case LAUNCH_STATE.External:
      return "connect";
    case LAUNCH_STATE.NotStarted:
    case LAUNCH_STATE.Loading:
    case LAUNCH_STATE.Ready:
    case LAUNCH_STATE.Failed:
    case LAUNCH_STATE.Stopped:
      return "launch";
    default:
      return null;
  }
}

/**
 * **カードの鍵の送り方の印**（作業単位 A-13b-3 / 冒頭の「鍵の送り方」）。出さないなら `null`。
 *
 * **暗号化せずに送っている2つ（`local_network` / `approved`）だけを言う。** `not_approved` は印と帯が言う
 * （[`faceStatus`]）ので、ここでは出さない。**知らない綴りは、そのまま出す**（欄を消さない）。
 */
export function keyMark(view: UpstreamView): string | null {
  switch (view.key_over_plain_http) {
    case undefined:
    case KEY_OVER_PLAIN_HTTP.NotApproved:
      return null;
    case KEY_OVER_PLAIN_HTTP.LocalNetwork:
      return "鍵を暗号化せずに送っています（宛先はローカルネットワーク）";
    case KEY_OVER_PLAIN_HTTP.Approved:
      return "鍵を暗号化せずに送っています（承認済み。宛先はローカルネットワークの外）";
    default:
      return `鍵の送り方: ${view.key_over_plain_http}`;
  }
}

/** 起こし方の文面（下書きの種類のボタンと同じ綴り）。**`null` は「分かりません」。** */
export function launchKindLabel(kind: FaceKind | null): string {
  return kind === null ? "分かりません" : faceKindLabel(kind);
}

/**
 * **カードの大きな名前**: 上流が名乗った名前。**名乗らなければ、理由ごとに別の文面**（`upstream.ts` の2つの定数）。
 *
 * **頼んだ名前と食い違っても、一言を足さない** —— その一言は照らし合わせの表の、その面のセルにだけ出す
 * （作業単位 B-45。画面に2回出すと、どちらの面の話かを読み違える余地が増える）。
 *
 * `named` は、等幅の綴りとして出すか（名乗った名前）、補足の字で出すか（名乗らなかった理由）。
 */
export function cardName(view: UpstreamView): { readonly text: string; readonly named: boolean } {
  if (view.answered_name !== null) {
    return { text: view.answered_name, named: true };
  }
  // **まだ照会していない面は「名乗っていない」と言わない**（作業単位 B-66）。
  if (notAskedYet(view)) {
    return { text: NOT_ASKED_LABEL, named: false };
  }
  return { text: view.answered_by === null ? UNNAMED_ANSWER_LABEL : UNDECIDED_ANSWER_LABEL, named: false };
}

/**
 * **カードのコンテキスト長と、その出どころ。**
 *
 * **この節が出せる出どころは「上流が申告した値」だけ** —— 観測の応答に載る長さはそれしか無い。
 * プロファイルに書いた長さは書いた節（`UpstreamProfiles.svelte`）が出す（書いた値と観測した値を混ぜない）。
 * **読めなければ数を出さず、出どころも言わない。**
 */
export function cardContext(view: UpstreamView): { readonly value: string; readonly source: string | null } {
  // **まだ照会していない面は「読めませんでした」と言わない**（作業単位 B-66 / Issue origin#94 のコメント1）。
  if (notAskedYet(view)) {
    return { value: NOT_ASKED_LABEL, source: null };
  }
  const tokens = view.queried_context_tokens;
  return { value: contextTokensLabel(tokens), source: tokens === null ? null : "上流が申告した値" };
}

/** 1本の帯。 */
export interface UpstreamBand {
  /** **この帯が言う面**（本文 → 裏方の順。2つの面の帯をまとめたら両方）。 */
  readonly faces: readonly Face[];
  /** 面の見出し（カードの見出しと同じ綴り。まとめたら「・」でつなぐ）。 */
  readonly label: string;
  readonly message: string;
  /** **調べる先の宛先**（鍵を断られた・送っていないときだけ）。起こした子の異常では `null`。 */
  readonly endpoint: string | null;
}

/**
 * **異常の帯**（本文 → 裏方の順）。**面を引くのは `report[face]`**（`view.face` ではない —— 鍵で引く）。
 *
 * **2つの面の帯が、理由も宛先も同じなら1本にまとめ、面の名前を両方言う**（作業単位 A-13b-3 のレビュー是正 /
 * 司令塔が画面で見た —— 裏方が「本文と同じ」のプロファイルで `not_approved` のとき、同じ文面・同じ宛先の帯が2本並んだ）。
 * **理由か宛先が違えば、2本のまま。** 起こした子の異常（失敗・止めた）は宛先を持たないので、理由が同じならまとめる。
 */
export function upstreamBands(report: UpstreamReport, zone: string): readonly UpstreamBand[] {
  const bands: { face: Face; alarm: FaceAlarm; message: string; endpoint: string | null }[] = FACES.flatMap((face) => {
    const view = report[face];
    const { alarm } = faceStatus(view);
    return alarm === null ? [] : [{ face, alarm, ...bandBody(alarm, view, zone) }];
  });
  const merged: { faces: Face[]; alarm: FaceAlarm; message: string; endpoint: string | null }[] = [];
  for (const band of bands) {
    const same = merged.find((seen) => seen.alarm === band.alarm && seen.endpoint === band.endpoint);
    if (same === undefined) {
      merged.push({ faces: [band.face], alarm: band.alarm, message: band.message, endpoint: band.endpoint });
    } else {
      same.faces.push(band.face);
    }
  }
  return merged.map(({ faces, message, endpoint }) => ({
    faces,
    label: faces.map((face) => faceLabel(face)).join("・"),
    message,
    endpoint,
  }));
}

function bandBody(alarm: FaceAlarm, view: UpstreamView, zone: string): { message: string; endpoint: string | null } {
  switch (alarm) {
    case "failed":
      return { message: FAILED_BAND_MESSAGE, endpoint: null };
    case "stopped":
      return { message: STOPPED_BAND_MESSAGE, endpoint: null };
    case "unauthorized":
      return { message: UNAUTHORIZED_BAND_MESSAGE, endpoint: view.endpoint };
    case "key_not_approved":
      return { message: KEY_NOT_APPROVED_BAND_MESSAGE, endpoint: view.endpoint };
    case "no_answer":
      return { message: noAnswerBandMessage(view, zone), endpoint: view.endpoint };
  }
}

/**
 * **照会したが繋がらない外の上流の帯の文面**（作業単位 B-66 / Issue origin#94 の決定4）。
 *
 * **いつ確かめたかを時刻で言う**（5分ごとの照会の間に上流が起きていれば、古い知らせである。経過ではなく時刻なのは
 * [`checkedAtLabel`] の doc）。直す手は「推論サーバを起動してから確かめ直す」で、ボタン（この節の「いま確かめる」）を名指す。
 *
 * **「起こす」と言わない**（推敲の是正）—— この画面の「起こす」は personacore が上流を起こす操作（起こすボタン・
 * 起こし方の行）の言葉である。この帯が出るのは外の上流（繋ぐ面）なので、利用者が自分で推論サーバを起動する話になる。
 */
function noAnswerBandMessage(view: UpstreamView, zone: string): string {
  const at = checkedAtLabel(view.checked_at, zone);
  const head = at === null ? "推論サーバへ繋がりません" : `推論サーバへ繋がりません（${at}）`;
  return `${head}。推論サーバを起動したら「いま確かめる」で確かめ直せます。`;
}

/** **その面の、一致しない行の数**（別のモデルが載っている + 依頼する綴りが一覧に無い）。聞けなかった行は数えない。 */
export function mismatchedRows(view: UpstreamView): number {
  return view.measured.drifted_other + view.measured.drifted_absent;
}

/**
 * 照らし合わせの見出し（`<summary>`）。**一致しない行があれば、面ごとの数を言う**（どちらか片方だけがずれていても両方）。
 * 無ければ、そう言う。**照らし合わせは開かない**（冒頭の ⚠ —— この見出しが知らせる）。
 *
 * **「一致しています」とは言わない** —— 聞けなかった行（上流が黙っている・照会していない面）だけでも
 * 一致しない行は 0 になるので、言えるのは「一致しない行は無い」までである。
 */
export function comparisonSummary(report: UpstreamReport): string {
  const head = "照らし合わせの詳細";
  if (!FACES.some((face) => mismatchedRows(report[face]) > 0)) {
    return `${head} —— 一致しない行はありません`;
  }
  return `${head} —— 一致しない行があります（本文 ${mismatchedRows(report.main)} 行・裏方 ${mismatchedRows(report.sub)} 行）`;
}
