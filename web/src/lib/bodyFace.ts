/**
 * **会話の画面が言う、本文の面の状態と、送信を止めるか**
 * （作業単位 D-16 / Issue origin#37 / ADR-0042 §決定3 の 2026-09-26 の追記）。**I/O を持たない。**
 *
 * # 何を決めるか
 *
 * | 本文の面（`report.main.launch.state`） | 送信 | 理由 |
 * |---|---|---|
 * | `ready` | **止めない** | 載っている |
 * | `external` | **止めない** | personacore が起こしていない。**状態を知らないので、止める根拠が無い**（ADR-0035） |
 * | `not_started` | 止める | **まだ起こしていない**（ADR-0055 §決定3 / 作業単位 F-10b）。**起こすボタンを出す**（下の節） |
 * | `loading` | 止める | 載る前に送ると、上流エラーで返るだけ（差し替えの直後もこの形で見える） |
 * | `failed` / `stopped` | 止める | 載っていない。**上流モデルの画面への導線を出す** |
 * | 知らない綴り | **止めない** | 下の ⚠ |
 * | まだ読めていない・読めなかった | **止めない** | 状態を知らない（`external` と同じ理由） |
 *
 * **裏方の面の状態では止めない**（本文は返る。裏方が載るまでの抽出・変容は今までどおり飛ぶ）。
 * **裏方が `not_started` でも止めないし、起こすボタンも出さない**（ADR-0055 §決定3）。
 * **裏方が載せている間は、その旨だけ言う**（本文で止めていないときだけ。止めているなら理由は1つで足りる）。
 *
 * # 起こすボタン（ADR-0055 §決定3 / 作業単位 F-10b）
 *
 * 本文の面が `not_started` なら、**いま選ばれているプロファイルの名前**（`selected_profile`）を
 * [`BodyFaceStatus.startProfile`] に載せる。画面はその名前で `POST /admin/upstream/profile` を送り、
 * 応答（起こした面は `loading`）をそのまま状態として採る —— 止める理由は「載せている最中」へ移り、
 * 載ったら戻る（今までの差し替えと同じ流れ）。
 * **名前が無いとき（選んでいない）はボタンを出さず、上流モデルの画面への導線に倒す**（無い名前を頼ませない）。
 *
 * # ⚠ 知らない綴りで止めない
 *
 * **判定の問い（ADR-0036）: 「外したとき、遅くなるだけか、壊れるか。」**
 * サーバが次の状態を足した日に、それが「載っている」側の状態だったら、**止める側に倒すと送れなくなる（壊れる）。**
 * 止めない側に倒すと、**今までどおり上流エラーの形で返る**（サーバの振る舞いは変えていない）。
 * だから**止めるのは、知っている「載っていない」綴りだけ。**
 *
 * **⚠ この注記はかつて「6つ目の状態を足した日に」と書いていた。** 6つ目（`not_started`）は
 * 作業単位 F-10a で実際に足され、そのあいだ**この表に無いので止めなかった**（`holdOf` の `default`）
 * —— 上の規則どおりの「止めない」側の壊れ方で、上流エラーの形で返っていた。F-10b で止める側に入れた。
 *
 * # ⚠ 「差し替え中」はサーバが別の綴りで言わない
 *
 * 差し替え（`POST /admin/upstream/profile`）の応答は、起こし直した面を **`loading`** で返す。
 * **その前の「古い子を止めて終了を待つ」間、サーバは面の状態を書かない**（`launch.rs` の `stop_except`）ので、
 * `GET /admin/upstream` は**古い `ready` のまま**に見える。**この画面が言えるのは `loading` になってから。**
 * 画面を増やす（差し替えを別の綴りで言わせる）のはサーバの変更なので、この単位ではしていない。
 *
 * # 綴りと文面の写しを増やさない
 *
 * 綴りは `upstreamProfiles.ts` の [`LAUNCH_STATE`]（見張り番が `launch.rs` と変種ごとに突き合わせる）、
 * 状態の文面は同じファイルの [`launchLabel`]（`#/upstream` のカードの「起こした上流の状態」と同じ文面）、
 * 起こすボタンが送る名前は同じファイルの [`profileToStart`]（`#/upstream` の画面と同じ判定）を使う。
 */

import type { UpstreamReport } from "./api";
import { LAUNCH_STATE, launchLabel, profileToStart } from "./upstreamProfiles";

/** 会話の画面に出すもの。 */
export interface BodyFaceStatus {
  /** **常に出す一言**（本文の面がいまどうなっているか）。 */
  readonly line: string;
  /** **送信を止める理由**（1行）。`null` は止めない。 */
  readonly hold: string | null;
  /** 止める理由の後ろに、上流モデルの画面への導線を置くか。 */
  readonly linkToUpstream: boolean;
  /** **裏方の一言**（裏方が載せている間だけ。本文で止めているときは出さない）。 */
  readonly subNote: string | null;
  /**
   * **起こすボタンが送るプロファイルの名前**（ADR-0055 §決定3 / 作業単位 F-10b）。
   * `null` はボタンを出さない（本文の面が `not_started` でない・選んだ名前が無い・状態を読めていない）。
   */
  readonly startProfile: string | null;
}

/** 一言の頭。**どの面の話か**を必ず名乗る（裏方の状態と読み違えさせない）。 */
const LINE_HEAD = "本文のモデル: ";

/** 載せている間に止める理由。**戻ることまで言う**（利用者が何かをする必要は無い）。 */
const LOADING_HOLD =
  "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。";
const FAILED_HOLD = "本文のモデルを載せられなかったので、送れません。";
const STOPPED_HOLD = "本文のモデルが止まっているので、送れません。";
/**
 * 起こしていない間に止める理由（作業単位 F-10b）。**何をすればよいかは、隣の起こすボタンが言う**
 * （ボタンを出せないときは、上流モデルの画面への導線が後ろに付く）。
 */
const NOT_STARTED_HOLD = "本文のモデルをまだ起こしていないので、送れません。";
const SUB_LOADING_NOTE =
  "裏方のモデルを載せています。載るまでの間、記憶の抽出と変容は行われません。";

/**
 * **本文の面の状態を、会話の画面の一言と送信の可否に組む。**
 *
 * @param report いちばん新しく読めた `GET /admin/upstream`（まだ読めていなければ `null`）。
 * @param readError 直近の読みが失敗した理由（成功していれば `null`）。**`report` より優先する** ——
 *   読めなくなった後に古い「載せています」で止め続けない（古い「起こしていない」でボタンを出し続けない）。
 * @param now `report` を読んだ時刻（[`launchLabel`] の経過の「いま」）。
 * @param zone 読み手の時間帯（`browserZone()` を画面が解決して渡す）。
 */
export function bodyFaceStatus(
  report: UpstreamReport | null,
  readError: string | null,
  now: Date,
  zone: string,
): BodyFaceStatus {
  if (readError !== null) {
    return {
      line: `${LINE_HEAD}状態を読めませんでした（${readError}）。送信は止めていません。`,
      hold: null,
      linkToUpstream: false,
      subNote: null,
      startProfile: null,
    };
  }
  if (report === null) {
    return {
      line: `${LINE_HEAD}状態を読み込み中…`,
      hold: null,
      linkToUpstream: false,
      subNote: null,
      startProfile: null,
    };
  }

  const main = report.main.launch;
  const line = `${LINE_HEAD}${launchLabel(main, now, zone)}`;
  const startProfile = profileToStart(report);
  const held = holdOf(main.state, startProfile);
  const subNote =
    held === null && report.sub.launch.state === LAUNCH_STATE.Loading ? SUB_LOADING_NOTE : null;
  return {
    line,
    hold: held?.reason ?? null,
    linkToUpstream: held?.link ?? false,
    subNote,
    startProfile,
  };
}

/**
 * **止める綴りだけを挙げる**（冒頭の「知らない綴りで止めない」）。
 *
 * @param startProfile 起こすボタンが送る名前。`not_started` で名前が無ければ、導線に倒す。
 */
function holdOf(
  state: string,
  startProfile: string | null,
): { reason: string; link: boolean } | null {
  switch (state) {
    case LAUNCH_STATE.NotStarted:
      return { reason: NOT_STARTED_HOLD, link: startProfile === null };
    case LAUNCH_STATE.Loading:
      return { reason: LOADING_HOLD, link: false };
    case LAUNCH_STATE.Failed:
      return { reason: FAILED_HOLD, link: true };
    case LAUNCH_STATE.Stopped:
      return { reason: STOPPED_HOLD, link: true };
    default:
      return null;
  }
}
