/**
 * **「いまの上流」の面ごとのカード・状態の印・異常の帯・照らし合わせの開き方**を決める純粋関数
 * （作業単位 D-12b-2 / Issue origin#93 / `src/lib/upstreamStatus.ts`）。
 *
 * # ここで見るもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **状態の印を、起こした状態と上流の答えのどれから決めるか**（組み合わせの表まるごと） | **ここ** |
 * | **異常の帯がどの面の・どの状態で出て、どの状態で出ないか** | **ここ** ＋ `upstream-panel-cards.test.ts`（画面） |
 * | **照らし合わせの見出しが、一致しない行の数を面ごとに言うこと** | **ここ** ＋ `upstream-panel-cards.test.ts`（画面） |
 *
 * **⚠ 司令塔の判断で仕様を変えた（D-12b-2 の2周目）。** 「答えがない」は帯にしない（照会していない面も同じ形で届くので、
 * 普段の構成で帯が出続ける）。照らし合わせは自動で開かない（llama-server ではずれが常にある / G-5）。
 *
 * **⚠ 作業単位 B-66 でもう一度変えた**（Issue origin#94）。サーバが `check_result` で「照会していない」と
 * 「照会したが答えない」を言い分けるので、**外の上流で照会したが答えない面は帯にする**（「繋がりません」）。
 * まだ照会していない面と、起こした面の名乗りが無いことは、今までどおり中立。
 *
 * # ⚠ 期待値は直書きする
 *
 * **実装の定数から期待値を作らない**（文面を緩める変異が、期待値も一緒に動かして緑になる / CLAUDE.md）。
 *
 * # ⚠ 2つの面で同じ値にしない
 *
 * 帯と照らし合わせは**面ごとの事実**なので、同値だと面を取り違える変異が観測できない。
 *
 * **jsdom は要らない**（DOM を触らない）。
 */

import { describe, expect, it } from "vitest";

import type { LaunchView, MeasuredView, UpstreamReport, UpstreamView } from "../src/lib/api";
import {
  UNAUTHORIZED_BAND_MESSAGE,
  cardContext,
  cardName,
  checkLabel,
  comparisonSummary,
  faceStatus,
  keyMark,
  launchKindLabel,
  launchKindOf,
  mismatchedRows,
  upstreamBands,
} from "../src/lib/upstreamStatus";

/**
 * **読み手の時間帯**（作業単位 B-66 のレビュー是正 —— 帯と「最後の確認」は確かめた時刻をこの時間帯の `HH:MM:SS` で言う）。
 *
 * > 1周目は「N 分前」だった。画面は載せている面があるときしか読み直さないので、普段の構成では
 * > 「1 分以内に確かめました」が2時間後も出続けた。経過は出さない。
 */
const ZONE = "Asia/Tokyo";

/** 帯を [`ZONE`] で組む。 */
function bandsAt(source: UpstreamReport) {
  return upstreamBands(source, ZONE);
}

function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

const NO_DRIFT: MeasuredView = { checked: 4, matched: 4, drifted_other: 0, drifted_absent: 0, unanswered: 0 };

/** 1面。**既定は「外の上流が答えている」。** */
function face(name: "main" | "sub", overrides: Partial<UpstreamView>): UpstreamView {
  return {
    face: name,
    endpoint: name === "main" ? "http://127.0.0.1:41001/v1/chat/completions" : "http://127.0.0.1:41002/v1/chat/completions",
    model: name === "main" ? "main-model" : "sub-model",
    answered_name: name === "main" ? "main-answer.gguf" : "sub-answer.gguf",
    queried_context_tokens: name === "main" ? 16384 : 8192,
    answered_by: "openai-exact-id",
    unauthorized: false,
    // **確かめて、答えた**（作業単位 B-66。[`ZONE`] では 10:00:00）。
    checked_at: "2026-11-03T10:00:00+09:00",
    check_result: "answered",
    measured: NO_DRIFT,
    launch: launch({}),
    ...overrides,
  };
}

function report(main: Partial<UpstreamView>, sub: Partial<UpstreamView>): UpstreamReport {
  return { main: face("main", main), sub: face("sub", sub), profile_sample: null, selected_profile: null };
}

/** **黙っている**（最後の照会に答えが無い —— 照会したが答えない / 作業単位 B-66）。 */
const SILENT: Partial<UpstreamView> = {
  answered_by: null,
  answered_name: null,
  queried_context_tokens: null,
  check_result: "no_answer",
};
/**
 * **まだ照会していない**（作業単位 B-66 / Issue origin#94）。**`answered_by: null` は [`SILENT`] と同じ**で、
 * 着手前の画面はこの2つを区別できなかった。
 */
const NOT_CHECKED: Partial<UpstreamView> = { ...SILENT, checked_at: null, check_result: "not_checked" };
/** **鍵を断られた**（`answered_by: null` と組 / ADR-0056 §決定4）。 */
const REFUSED: Partial<UpstreamView> = { ...SILENT, unauthorized: true, check_result: "refused" };
/**
 * **承認が無いので鍵を送らなかった**（ADR-0056 §決定5 / 作業単位 A-13b-3）。照会は接続の前に止まるので、答えも無い。
 */
const WITHHELD: Partial<UpstreamView> = {
  ...SILENT,
  key_over_plain_http: "not_approved",
  check_result: "key_not_sent",
};

const READY = launch({ state: "ready", answered_name: "main-answer.gguf" });
const LOADING = launch({ state: "loading", since: "2026-11-02T10:00:00+09:00" });
const NOT_STARTED = launch({ state: "not_started" });
const FAILED = launch({ state: "failed", reason: "子が自分で終了しました", log: "main.err.log" });
const STOPPED = launch({ state: "stopped" });
const UNKNOWN = launch({ state: "sleeping" });

describe("状態の印", () => {
  /**
   * **組み合わせの表まるごと**（`[名前, 面の上書き, 印, 色の種類, 帯の理由]`）。
   *
   * # 決め方
   *
   * * **起こした子が動いていない状態**（起こしていない・載せている・失敗・止めた）は、**起こした状態だけで決める** ——
   *   そのときの上流の答え（`answered_by` / `unauthorized`）は、動いていない子についての古い・空の値だから。
   * * **答えが期待できる状態**（準備完了・外の上流）は、**上流の答えで決める** —— 鍵を断られた → 名乗りが無い → 答えた。
   * * **外の上流が照会に答えないのは異常**（帯 / 作業単位 B-66）。**まだ照会していない・起こした面の名乗りが無い・
   *   知らない結果の綴りは中立**（色なし・帯なし）。
   * * **知らない綴りは、その綴りのまま出し、異常とも正常とも言わない**（嘘をつかない）。
   */
  const CASES: [string, Partial<UpstreamView>, string, string, string | null][] = [
    ["準備完了で答えている", { launch: READY }, "動いています", "running", null],
    ["外の上流が答えている", {}, "答えています", "running", null],
    ["起こしていない", { ...SILENT, launch: NOT_STARTED }, "起こしていません", "waiting", null],
    ["載せている", { ...SILENT, launch: LOADING }, "載せています", "loading", null],
    ["失敗した", { ...SILENT, launch: FAILED }, "失敗しました", "alarm", "failed"],
    ["止めた", { ...SILENT, launch: STOPPED }, "止めました", "alarm", "stopped"],
    // **照会したが答えない外の上流は、異常の帯**（作業単位 B-66 —— 着手前は「名乗りがありません」の中立の印だった。
    // 照会していない面と区別できなかったので。いまは `check_result` で割る）。
    ["外の上流が黙っている", SILENT, "繋がりません", "alarm", "no_answer"],
    // **まだ照会していない外の上流は、中立のまま**（帯にしない）。
    ["外の上流にまだ照会していない", NOT_CHECKED, "まだ確かめていません", "quiet", null],
    // **届いたが一覧が読めない外の上流は、中立**（レビュー是正 —— 一覧は上流に「任意」の能力 / ADR-0038 §決定1。
    // 一覧の口を持たない上流にコンテキスト長を書いて繋ぐと、会話は通るのに照会は毎回失敗する）。
    ["外の上流が一覧に答えない", { ...SILENT, check_result: "no_model_list" }, "一覧に答えません", "quiet", null],
    ["準備完了で一覧に答えない", { ...SILENT, check_result: "no_model_list", launch: READY }, "名乗りがありません", "quiet", null],
    // **知らない結果の綴りでは、異常とも言わない**（今までの中立の印）。
    ["外の上流が黙っていて結果が知らない綴り", { ...SILENT, check_result: "sideways" }, "名乗りがありません", "quiet", null],
    // **起こす面の振る舞いは変えない**（作業単位 B-66 の決定4）—— 準備完了の面は、答えなくても照会していなくても中立。
    ["準備完了なのに黙っている", { ...SILENT, launch: READY }, "名乗りがありません", "quiet", null],
    ["準備完了でまだ照会していない", { ...NOT_CHECKED, launch: READY }, "名乗りがありません", "quiet", null],
    ["外の上流が鍵を断った", REFUSED, "鍵を断られました", "alarm", "unauthorized"],
    ["準備完了の上流が鍵を断った", { ...REFUSED, launch: READY }, "鍵を断られました", "alarm", "unauthorized"],
    // **ありえない組み合わせ** —— 鍵を断られたのに答えた読み手がある。**断られた事実を落とさない。**
    ["鍵を断られたのに答えている", { unauthorized: true }, "鍵を断られました", "alarm", "unauthorized"],
    // **動いていない子の古い観測で、印も帯も変えない。**
    ["起こしていないのに鍵を断られた", { ...REFUSED, launch: NOT_STARTED }, "起こしていません", "waiting", null],
    ["載せている間に答えがある", { launch: LOADING }, "載せています", "loading", null],
    ["失敗したのに答えがある", { launch: FAILED }, "失敗しました", "alarm", "failed"],
    ["止めたのに鍵を断られた", { ...REFUSED, launch: STOPPED }, "止めました", "alarm", "stopped"],
    // **知らない綴り**は、そのまま出し、帯を出さない（答えていても黙っていても）。
    ["知らない綴りで答えている", { launch: UNKNOWN }, "sleeping", "unknown", null],
    ["知らない綴りで黙っている", { ...SILENT, launch: UNKNOWN }, "sleeping", "unknown", null],
    ["知らない綴りで鍵を断られた", { ...REFUSED, launch: UNKNOWN }, "sleeping", "unknown", null],
    // **承認が無いので鍵を送らなかった**（作業単位 A-13b-3）。送信を止めているので異常。
    ["外の上流へ鍵を送らなかった", WITHHELD, "鍵を送っていません", "alarm", "key_not_approved"],
    ["準備完了の上流へ鍵を送らなかった", { ...WITHHELD, launch: READY }, "鍵を送っていません", "alarm", "key_not_approved"],
    // **ありえない組み合わせ** —— 送らなかったのに断られた。**送らなかったほうを言う**（鍵を直せとは言わない）。
    [
      "鍵を送らなかったのに断られた",
      { ...WITHHELD, unauthorized: true },
      "鍵を送っていません",
      "alarm",
      "key_not_approved",
    ],
    // **暗号化せずに送っている面は、異常ではない**（カードの印で言う / [`keyMark`]）。
    ["ローカルネットワークへ鍵を送って答えている", { key_over_plain_http: "local_network" }, "答えています", "running", null],
    ["承認して外へ鍵を送って答えている", { key_over_plain_http: "approved" }, "答えています", "running", null],
    // **動いていない子・知らない綴りでは、送らなかったことで印も帯も変えない。**
    ["起こしていないのに送らなかった", { ...WITHHELD, launch: NOT_STARTED }, "起こしていません", "waiting", null],
    ["知らない綴りで送らなかった", { ...WITHHELD, launch: UNKNOWN }, "sleeping", "unknown", null],
  ];

  it.each(CASES)("%s", (_name, overrides, badge, tone, alarm) => {
    expect(faceStatus(face("main", overrides))).toEqual({ badge, tone, alarm });
  });

  /** **6つの起こした状態と、答えの3つの形が、別の印になる**（同じ印に潰す変異を、表とは別に1本で止める）。 */
  it("知っている印は、全部別の文面", () => {
    const badges = CASES.filter(([, , , tone]) => tone !== "unknown").map(([, , badge]) => badge);
    expect(new Set(badges)).toEqual(
      new Set([
        "動いています",
        "答えています",
        "起こしていません",
        "載せています",
        "失敗しました",
        "止めました",
        "名乗りがありません",
        "鍵を断られました",
        "鍵を送っていません",
        "繋がりません",
        "まだ確かめていません",
        "一覧に答えません",
      ]),
    );
  });
});

describe("最後の確認（作業単位 B-66）", () => {
  /**
   * **カードの「最後の確認」の行**: 確かめた時刻（読み手の時間帯 [`ZONE`] の `HH:MM:SS`）と、結果。
   * **経過は出さない**（レビュー是正 —— 画面は普段は読み直さないので、経過は読んだときのまま止まる）。
   * **まだ照会していない面は時刻を言わない。** 知らない結果の綴りはそのまま、読めない時刻もそのまま出す。
   * **`Z` 終わり・小数部 9 桁**（サーバの `to_rfc3339` の形）も、読み手の時間帯へ直して言う。
   */
  it.each([
    ["答えた", {}, "10:00:00 に確かめました（答えました）"],
    ["答えなかった", { ...SILENT, checked_at: "2026-11-03T10:03:00+09:00" }, "10:03:00 に確かめました（繋がりません）"],
    ["一覧に答えない", { ...SILENT, check_result: "no_model_list" }, "10:00:00 に確かめました（一覧に答えません）"],
    ["断られた", { ...REFUSED, checked_at: "2026-11-03T08:00:00+09:00" }, "08:00:00 に確かめました（鍵を断られました）"],
    ["送らなかった", WITHHELD, "10:00:00 に確かめました（鍵を送っていません）"],
    ["まだ照会していない", NOT_CHECKED, "まだ確かめていません"],
    ["知らない結果の綴り", { check_result: "sideways" }, "10:00:00 に確かめました（sideways）"],
    ["読めない時刻", { checked_at: "yesterday" }, "yesterday に確かめました（答えました）"],
    ["サーバの綴り（UTC・小数部 9 桁）", { checked_at: "2026-11-03T03:04:05.123456789+00:00" }, "12:04:05 に確かめました（答えました）"],
  ] as const)("%s", (_name, overrides, label) => {
    expect(checkLabel(face("sub", overrides), ZONE)).toBe(label);
  });

  /** **時間帯は引数で受ける**（同じ時刻を別の時間帯で読むと、別の `HH:MM:SS` になる）。 */
  it("読み手の時間帯で言う", () => {
    const at = { checked_at: "2026-11-03T01:00:00Z" };
    expect([checkLabel(face("sub", at), "Asia/Tokyo"), checkLabel(face("sub", at), "America/New_York")]).toEqual([
      "10:00:00 に確かめました（答えました）",
      "20:00:00 に確かめました（答えました）",
    ]);
  });
});

describe("鍵の送り方の印（作業単位 A-13b-3）", () => {
  /**
   * **暗号化せずに送っている2つは、カードの印で言う**（異常ではない）。**送っていない（`not_approved`）は帯が言う**ので、
   * ここでは出さない。欄が無い面（警告の無い面）にも出さない。**知らない綴りは、そのまま出す。**
   */
  it.each([
    ["local_network", "鍵を暗号化せずに送っています（宛先はローカルネットワーク）"],
    ["approved", "鍵を暗号化せずに送っています（承認済み。宛先はローカルネットワークの外）"],
    ["not_approved", null],
    ["sideways", "鍵の送り方: sideways"],
  ] as const)("%s → %s", (value, mark) => {
    expect(keyMark(face("main", { key_over_plain_http: value }))).toBe(mark);
  });

  it("欄の無い面には印を出さない", () => {
    expect(keyMark(face("main", {}))).toBeNull();
  });
});

describe("起こし方", () => {
  it.each([
    ["external", "connect", "動いているサーバに繋ぐ"],
    ["not_started", "launch", "この機で llama を起こす"],
    ["loading", "launch", "この機で llama を起こす"],
    ["ready", "launch", "この機で llama を起こす"],
    ["failed", "launch", "この機で llama を起こす"],
    ["stopped", "launch", "この機で llama を起こす"],
    // **知らない綴りは、どちらとも言わない。**
    ["sleeping", null, "分かりません"],
  ] as const)("%s → %s", (state, kind, label) => {
    expect(launchKindOf(launch({ state }))).toBe(kind);
    expect(launchKindLabel(kind)).toBe(label);
  });
});

describe("カードのモデル名とコンテキスト長", () => {
  it("名乗った名前はそのまま、名乗らなければ理由ごとに別の文面", () => {
    expect(cardName(face("main", {}))).toEqual({ text: "main-answer.gguf", named: true });
    expect(cardName(face("main", SILENT))).toEqual({ text: "（上流は名乗っていません）", named: false });
    expect(cardName(face("main", { answered_name: null }))).toEqual({
      text: "上流は一覧を出しましたが、どれが答えたかは決められません",
      named: false,
    });
    // **まだ照会していない面は「名乗っていない」と言わない**（作業単位 B-66）。
    expect(cardName(face("main", NOT_CHECKED))).toEqual({ text: "まだ確かめていません", named: false });
  });

  /** **頼んだ名前と違っていても、カードは名乗った名前だけを出す**（食い違いの一言は照らし合わせの表のセルにだけ / B-45）。 */
  it("頼んだ名前と違っても、食い違いの一言を足さない", () => {
    expect(cardName(face("main", { answered_name: "other.gguf" })).text).toBe("other.gguf");
  });

  it("申告した長さは出どころを添え、読めなければ数を出さない", () => {
    expect(cardContext(face("main", {}))).toEqual({ value: "16384 トークン", source: "上流が申告した値" });
    expect(cardContext(face("sub", {}))).toEqual({ value: "8192 トークン", source: "上流が申告した値" });
    expect(cardContext(face("main", SILENT))).toEqual({ value: "読めませんでした", source: null });
    // **まだ照会していない面は「読めませんでした」と言わない**（作業単位 B-66 / Issue origin#94 のコメント1）。
    expect(cardContext(face("main", NOT_CHECKED))).toEqual({ value: "まだ確かめていません", source: null });
  });
});

describe("異常の帯", () => {
  /**
   * **帯は面ごと**（本文 → 裏方の順）。**どの面の話かを必ず名乗る。**
   * 2つの面に別の異常を置くので、面を取り違える変異も、理由を取り違える変異も赤になる。
   */
  it("面ごとに、その面の異常を、本文 → 裏方の順で出す", () => {
    expect(bandsAt(report({ ...SILENT, launch: FAILED }, REFUSED))).toEqual([
      {
        faces: ["main"],
        label: "本文生成",
        message: "載せられませんでした。理由は、この面のカードの「起こした上流の状態」にあります。",
        endpoint: null,
      },
      {
        faces: ["sub"],
        label: "裏方（抽出・変容）",
        message:
          "推論サーバが鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。下の「upstreams.json に書いたプロファイル」の「直す」でこの面の鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。",
        endpoint: "http://127.0.0.1:41002/v1/chat/completions",
      },
    ]);
  });

  /**
   * **承認が無いので送らなかった面の帯**（作業単位 A-13b-3）。**断られた帯とは別の文面**で、上流は鍵を見ていない・
   * 直すのは承認か宛先だと言う。宛先も添える。**断られた面と並べて、面ごとに別の帯になることを見る。**
   */
  it("承認が無くて送らなかった面と、断られた面は、別の帯で言う", () => {
    expect(bandsAt(report(WITHHELD, REFUSED))).toEqual([
      {
        faces: ["main"],
        label: "本文生成",
        message:
          "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、鍵を送っていません（推論サーバは鍵を見ていません）。下の「upstreams.json に書いたプロファイル」の「直す」で承認するか、宛先を https:// にしてください。",
        endpoint: "http://127.0.0.1:41001/v1/chat/completions",
      },
      {
        faces: ["sub"],
        label: "裏方（抽出・変容）",
        message:
          "推論サーバが鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。下の「upstreams.json に書いたプロファイル」の「直す」でこの面の鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。",
        endpoint: "http://127.0.0.1:41002/v1/chat/completions",
      },
    ]);
  });

  /** **暗号化せずに送っているだけの面には、帯を出さない**（印だけ）。 */
  it("ローカルネットワーク・承認済みで送っている面には帯を出さない", () => {
    expect(
      bandsAt(report({ key_over_plain_http: "local_network" }, { key_over_plain_http: "approved" })),
    ).toEqual([]);
  });

  /**
   * **まだ照会していない面には帯を出さない**（同じ回に止めた面があれば、そちらの帯だけ）。
   *
   * > **作業単位 B-66 で裏方の面を `SILENT` から `NOT_CHECKED` へ替えた。** 着手前は「名乗りの無い面には出ない」を
   * > 見ていたが、いまは照会したが答えない外の上流には帯が出る（下の「答えない外の上流の帯」）。
   */
  it("止めた面には帯が出て、まだ照会していない面には出ない", () => {
    expect(bandsAt(report({ ...SILENT, launch: STOPPED }, NOT_CHECKED))).toEqual([
      {
        faces: ["main"],
        label: "本文生成",
        message: "この実行ファイルが終わるときに止めました。",
        endpoint: null,
      },
    ]);
  });

  /**
   * **異常でない状態では、1本も出さない**（常に出す変異を止める）。
   * **名乗りが無い面（外の上流・準備完了）も、ここに入る**（照会していない面と区別できないので異常と言わない）。
   */
  it.each([
    ["両面とも答えている", {}, {}],
    ["準備完了と外の上流", { launch: READY }, {}],
    ["載せている・起こしていない", { ...SILENT, launch: LOADING }, { ...SILENT, launch: NOT_STARTED }],
    ["知らない綴りで黙っている", { ...SILENT, launch: UNKNOWN }, { ...REFUSED, launch: UNKNOWN }],
    // **作業単位 B-66 で `SILENT` から `NOT_CHECKED` へ替えた**（照会したが答えない外の上流には、いまは帯が出る）。
    ["両面とも外の上流でまだ照会していない", NOT_CHECKED, NOT_CHECKED],
    ["準備完了で名乗りが無い", { ...SILENT, launch: READY }, { ...SILENT, launch: READY }],
    ["外の上流で、結果が知らない綴り", { ...SILENT, check_result: "sideways" }, NOT_CHECKED],
    // **届いたが一覧が読めない外の上流は帯にしない**（レビュー是正 —— 会話は通るのに帯が出続ける形を作らない）。
    ["両面とも外の上流が一覧に答えない", { ...SILENT, check_result: "no_model_list" }, { ...SILENT, check_result: "no_model_list" }],
  ] as const)("%s → 帯なし", (_name, main, sub) => {
    expect(bandsAt(report(main, sub))).toEqual([]);
  });

  /**
   * **照会したが答えない外の上流は、帯で知らせる。確かめた時刻を言い、宛先を添える**
   * （作業単位 B-66 / Issue origin#94。時刻はレビュー是正で経過から替えた）。
   *
   * **面を両方の向きで置く**（本文だけ・裏方だけ）——片方の面だけを見る変異と、面を取り違える変異を赤にする。
   * **答えている面には帯を出さない。**
   */
  it("答えない外の上流の帯は、確かめた時刻と宛先を言う", () => {
    const message =
      "推論サーバへ繋がりません（10:00:00 に確かめました）。推論サーバを起動したら「いま確かめる」で確かめ直せます。";
    expect(bandsAt(report(SILENT, {}))).toEqual([
      { faces: ["main"], label: "本文生成", message, endpoint: "http://127.0.0.1:41001/v1/chat/completions" },
    ]);
    expect(bandsAt(report({}, { ...SILENT, checked_at: "2026-11-03T09:00:00+09:00" }))).toEqual([
      {
        faces: ["sub"],
        label: "裏方（抽出・変容）",
        message:
          "推論サーバへ繋がりません（09:00:00 に確かめました）。推論サーバを起動したら「いま確かめる」で確かめ直せます。",
        endpoint: "http://127.0.0.1:41002/v1/chat/completions",
      },
    ]);
  });

  /** **裏方だけが異常なら、裏方の帯だけ**（本文の面だけを見る変異を止める）。 */
  it("裏方だけが鍵を断られれば、裏方の帯だけ", () => {
    expect(bandsAt(report({}, REFUSED)).map((band) => band.faces)).toEqual([["sub"]]);
    expect(bandsAt(report(REFUSED, {})).map((band) => band.faces)).toEqual([["main"]]);
  });

  /**
   * **理由も宛先も同じ2つの面の帯は、1本にまとめ、面の名前を両方言う**（作業単位 A-13b-3 のレビュー是正 /
   * 裏方が「本文と同じ」のプロファイル —— 同じ宛先に同じ理由で2本並んでいた）。
   * **宛先だけ違えば2本、理由だけ違えば2本。** 起こした子の異常（宛先を持たない）も同じ規則。
   */
  it("理由も宛先も同じなら1本にまとめ、面の名前を両方言う", () => {
    const shared = "http://203.0.113.5:46001/v1/chat/completions";
    const WITHHELD_BAND =
      "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、鍵を送っていません（推論サーバは鍵を見ていません）。下の「upstreams.json に書いたプロファイル」の「直す」で承認するか、宛先を https:// にしてください。";
    expect(bandsAt(report({ ...WITHHELD, endpoint: shared }, { ...WITHHELD, endpoint: shared }))).toEqual([
      {
        faces: ["main", "sub"],
        label: "本文生成・裏方（抽出・変容）",
        message: WITHHELD_BAND,
        endpoint: shared,
      },
    ]);
    // **断られた・失敗・止めたにも同じ規則。**
    for (const [name, overrides] of [
      ["断られた", { ...REFUSED, endpoint: shared }],
      ["答えない", { ...SILENT, endpoint: shared }],
      ["失敗した", { ...SILENT, launch: FAILED }],
      ["止めた", { ...SILENT, launch: STOPPED }],
    ] as const) {
      expect(
        bandsAt(report(overrides, overrides)).map((band) => [band.faces, band.label]),
        name,
      ).toEqual([[["main", "sub"], "本文生成・裏方（抽出・変容）"]]);
    }
  });

  it("宛先だけ違えば2本、理由だけ違えば2本", () => {
    // 宛先だけ違う（フィクスチャの宛先は面ごとに違う）。
    expect(bandsAt(report(WITHHELD, WITHHELD)).map((band) => band.faces)).toEqual([["main"], ["sub"]]);
    // 理由だけ違う（宛先は同じ）。
    const shared = "http://203.0.113.5:46001/v1/chat/completions";
    expect(
      bandsAt(report({ ...WITHHELD, endpoint: shared }, { ...REFUSED, endpoint: shared })).map(
        (band) => band.faces,
      ),
    ).toEqual([["main"], ["sub"]]);
    // 起こした子の異常で理由が違う（失敗と止めた）。
    expect(
      bandsAt(report({ ...SILENT, launch: FAILED }, { ...SILENT, launch: STOPPED })).map((band) => band.faces),
    ).toEqual([["main"], ["sub"]]);
  });

  /**
   * **鍵の文言は1つの定数**。帯はその定数をそのまま使う。
   *
   * > 作業単位 D-12b-2 の間は仮の文言（「推論サーバが鍵を受け付けませんでした。」）で、A-13b-3 で差し替えた。
   * > 鍵の欄ができたので、直す場所（プロファイルの「直す」）を名乗る。
   * > 作業単位 B-69（Issue origin#98）で、鍵のほかの候補（上流の側で鍵を有効にしていない・宛先が違う）と、
   * > 保存した後に「切り替え直す」を押すことを足した。
   */
  it("鍵を断られたときの文言は、定数そのもの（直す場所を名乗る）", () => {
    expect(UNAUTHORIZED_BAND_MESSAGE).toBe(
      "推論サーバが鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。下の「upstreams.json に書いたプロファイル」の「直す」でこの面の鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。",
    );
    expect(bandsAt(report(REFUSED, {}))[0]?.message).toBe(UNAUTHORIZED_BAND_MESSAGE);
  });
});

describe("照らし合わせの見出し", () => {
  const OTHER: MeasuredView = { checked: 9, matched: 8, drifted_other: 1, drifted_absent: 0, unanswered: 0 };
  const ABSENT: MeasuredView = { checked: 9, matched: 6, drifted_other: 0, drifted_absent: 3, unanswered: 0 };
  /** **聞けなかった行は「一致しない」に数えない**（上流が黙っている・コンテキスト長を設定した）。 */
  const UNASKED: MeasuredView = { checked: 9, matched: 0, drifted_other: 0, drifted_absent: 0, unanswered: 9 };

  it("一致しない行は、別のモデルと、一覧に無いものの和", () => {
    expect(mismatchedRows(face("main", { measured: OTHER }))).toBe(1);
    expect(mismatchedRows(face("main", { measured: ABSENT }))).toBe(3);
    expect(mismatchedRows(face("main", { measured: UNASKED }))).toBe(0);
    expect(mismatchedRows(face("main", { measured: { ...OTHER, drifted_absent: 3 } }))).toBe(4);
  });

  /**
   * **見出しは、一致しない行の数を面ごとに言う。** 2つの面で数を違えてあるので、面を取り違える変異も、
   * 片方の面からだけ数を取る変異も赤になる。**どちらの面だけがずれていても、両方の数を言う。**
   */
  it.each([
    ["本文だけ別のモデル", OTHER, NO_DRIFT, "照らし合わせの詳細 —— 一致しない行があります（本文 1 行・裏方 0 行）"],
    ["裏方だけ一覧に無い", NO_DRIFT, ABSENT, "照らし合わせの詳細 —— 一致しない行があります（本文 0 行・裏方 3 行）"],
    ["両方ずれている", OTHER, ABSENT, "照らし合わせの詳細 —— 一致しない行があります（本文 1 行・裏方 3 行）"],
    ["両方一致", NO_DRIFT, NO_DRIFT, "照らし合わせの詳細 —— 一致しない行はありません"],
    // **聞けなかった行は数えない**（一致したとも言わず、一致しない行が無いとだけ言う）。
    ["聞けなかっただけ", UNASKED, UNASKED, "照らし合わせの詳細 —— 一致しない行はありません"],
  ] as const)("%s", (_name, main, sub, summary) => {
    expect(comparisonSummary(report({ measured: main }, { measured: sub }))).toBe(summary);
  });
});
