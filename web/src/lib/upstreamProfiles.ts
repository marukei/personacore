/**
 * **書いたプロファイルを、画面の行と下書きに組む**（作業単位 F-4b-3 / Issue origin#24）。**I/O を持たない。**
 *
 * 送るのは `api.ts`、画面は `UpstreamProfiles.svelte`（下書きのフォームもそこにある）。
 *
 * # 書いた値と観測した値を混ぜない（Issue origin#24 の閉じる条件）
 *
 * **ここが扱うのは、利用者が `upstreams.json` に書いたままの値だけ**（`api.ts` の `WrittenFace`）。
 * **上流が名乗った名前・窓は `upstream.ts` が扱う**（`GET /admin/upstream` の観測）。
 * **書いていない鍵を、決まる値（既定の実行ファイル名・本文の上流）で埋めない** ——
 * 埋めると「書いた」と「決まった」が同じ見え方になる。**書いていないと言う。**
 *
 * # 帰属は1箇所で決める
 *
 * どのプロファイルの、どちらの面の、どの鍵か ——**入れ替えの変異を1本で押さえる**ため、
 * 一覧の行（[`summaryLines`]）と、種類ごとの欄（[`KIND_FIELDS`]）はここで組む（`upstream.ts` の `reportRows` と同じ立場）。
 *
 * # 写し元
 *
 * | ここ | 写し元 | 見張り番 |
 * |---|---|---|
 * | [`FACES`] | `src/server/upstreams.rs` の `FACE_NAMES` | `tests/upstream-profiles.test.ts` |
 * | [`FACE_KEYS`] | 同 `FACE_KEYS` | 同上 |
 * | [`DEFAULT_COMMAND`] | 同 `DEFAULT_COMMAND` | 同上 |
 * | [`LAUNCH_STATE`]（[`launchLabel`] と会話の画面が読む綴り） | `src/server/launch.rs` の `FaceState::label` | 同上（**変種ごとに**） |
 */

import {
  ApiRequestError,
  type LaunchView,
  type ProfilesView,
  type SaveRequest,
  type UpstreamReport,
  type WrittenFace,
  type WrittenFaceRequest,
  type WrittenProfile,
} from "./api";
import { instantOf, wallTimeOf } from "./timestamp";
import { keyDestination } from "./upstreamKey";

/** **面の綴り**（`upstreams.rs` の `FACE_NAMES` の写し）。 */
export const FACES = ["main", "sub"] as const;

/** 面。 */
export type Face = (typeof FACES)[number];

/**
 * **面の中の鍵**（`upstreams.rs` の `FACE_KEYS` の写し。**並びも**）。
 *
 * **`context_tokens` は作業単位 A-13a-2 で足した**（ADR-0054 §決定5）。
 *
 * **`api_key` は作業単位 A-13b-1 で足した**（ADR-0056 §決定1）。**鍵の欄は作業単位 A-13b-3 で足した**
 * （値は見せない。設定済みか・末尾4文字・新しい鍵・消す）。下書きの欄・422 を結びつける欄は [`EDITABLE_FACE_KEYS`] の鍵だけを扱う。
 *
 * **`allow_key_over_plain_http` は作業単位 A-13b-2 で足した**（ADR-0056 §決定5）。**欄にはしない**
 * （真偽値を打たせない）。承認は保存の前の確認で取る（作業単位 A-13b-3 / [`nextKeyQuestion`]）。
 *
 * **`sampling` は作業単位 A-16 で足した**（ADR-0058 §決定6）。**欄にはしない**（画面で値を見せる・直す作業は
 * 別の作業単位）。画面は値を受け取らず送りもしない —— 保存はサーバが、いまのファイルの同じ名前・同じ面の値を残す。
 */
export const FACE_KEYS = [
  "endpoint",
  "model",
  "gguf",
  "args",
  "command",
  "context_tokens",
  "api_key",
  "allow_key_over_plain_http",
  "sampling",
] as const;

/** 面の中の鍵。 */
export type FaceKey = (typeof FACE_KEYS)[number];

/**
 * **画面が欄を持つ鍵**（[`FACE_KEYS`] から `allow_key_over_plain_http` を除いたもの。並びは同じ）。
 *
 * **`api_key` の欄は、値を出さない**（応答は鍵の値を返さない —— `api_key_set` / `api_key_hint` だけ / ADR-0056 §決定3）。
 * 下書きの `api_key` は**新しい鍵**の入力で、空なら置き換えない（[`FaceDraft`]）。作業単位 A-13b-3 で欄にした
 * （A-13b-1・A-13b-2 の間は除いていた）。**鍵を名指した 422 は、その面の鍵の欄に出る。**
 *
 * **`allow_key_over_plain_http` を除く理由**: 真偽値を欄で打たせない。承認は保存の前の確認で取り、
 * 鍵の欄の隣で状態を言う（[`keySendingState`]）。
 *
 * **`sampling` を除く理由**（ADR-0058 §決定6）: 画面はまだこの欄を持たない。除かないと、`key: "sampling"` の 422 を
 * 存在しない欄に結びつけ、理由が画面から消える（[`isEditableFaceKey`]）。
 */
export const EDITABLE_FACE_KEYS = FACE_KEYS.filter(
  (key): key is EditableFaceKey => key !== "allow_key_over_plain_http" && key !== "sampling",
);

/** 画面が欄を持つ鍵。 */
export type EditableFaceKey = Exclude<FaceKey, "allow_key_over_plain_http" | "sampling">;

/**
 * **`command` を書かなかった面で起こす実行ファイルの名前**（`upstreams.rs` の `DEFAULT_COMMAND` の写し）。
 *
 * **表の値を埋めるのには使わない**（書いた値ではない）。注記が言うだけ。
 */
export const DEFAULT_COMMAND = "llama";

/** 面の見出し（`UpstreamPanel.svelte` の列の見出しと同じ綴り）。 */
export function faceLabel(face: Face): string {
  return face === "main" ? "本文生成" : "裏方（抽出・変容）";
}

/** 一覧の1行の、面の短い見出し（絵の Main のまま）。 */
function faceShortLabel(face: Face): string {
  return face === "main" ? "本文" : "裏方";
}

// ----------------------------------------------------------------------
// 面の種類 —— 「この機で llama を起こす」と「動いているサーバに繋ぐ」（作業単位 D-19 / Issue origin#87）
// ----------------------------------------------------------------------

/**
 * **面の種類**（作業単位 D-19）。
 *
 * **ファイルに種類の鍵は無い**（`upstreams.json` の形は変えない）。サーバは **`gguf` を書いた面を起こす**
 * （`upstreams.rs` の `ProfileFace` の `launch: Option<FaceLaunch>`）。画面はそれを
 * 「起こす」「繋ぐ」として名乗り、**利用者に選ばせる**（着手前は gguf の欄が埋まっているかで暗に決まっていた）。
 */
export type FaceKind = "launch" | "connect";

/** **書いた面の種類**: `gguf` を書いていれば起こす、無ければ繋ぐ（サーバの読み方と同じ）。 */
export function kindOfFace(face: WrittenFace): FaceKind {
  return face.gguf === null ? "connect" : "launch";
}

/** 一覧の種類の印（絵の Main）。 */
export function faceKindBadge(kind: FaceKind): string {
  return kind === "launch" ? "起こす" : "繋ぐ";
}

/** 下書きの、種類を選ぶボタンの文面（絵の Edit）。 */
export function faceKindLabel(kind: FaceKind): string {
  return kind === "launch" ? "この機で llama を起こす" : "動いているサーバに繋ぐ";
}

/**
 * **種類ごとに、下書きが持つ欄**（作業単位 D-19）。`shown` はいつも出し、`folded` は「詳細」に畳む。
 *
 * | 種類 | 出す | 詳細に畳む |
 * |---|---|---|
 * | 起こす | gguf・モデル・待ち受ける場所・鍵 | 起動の引数・実行ファイル・コンテキスト長 |
 * | 繋ぐ | 宛先・モデル・鍵・コンテキスト長 | （無し） |
 *
 * **起こす面にもコンテキスト長を置く**（ADR-0054 は面の種類で分けていない）。**データが受け付ける欄を黙って落とさない。**
 * 起こした llama は窓を答えるので普段は要らず、詳細に畳む。
 *
 * **鍵は両方の種類の欄**（作業単位 A-13b-3 のレビュー是正）。起こす面にも、`args` の `--api-key X` と面の
 * `api_key: X` を組で書いて動いている構成がある。**種類を切り替えても鍵は送らない（残す）** ——
 * 着手時は鍵を繋ぐ面だけの欄にして、起こす面の鍵を `null` で送っていた（モデル名を直しただけで鍵が消え、次の起動から 401）。
 *
 * **保存で送るのは、選んだ種類の欄だけ**（[`buildSaveRequest`]）。繋ぐ面の gguf・引数・実行ファイルは送らない。
 */
export const KIND_FIELDS: Readonly<
  Record<FaceKind, { readonly shown: readonly EditableFaceKey[]; readonly folded: readonly EditableFaceKey[] }>
> = {
  launch: { shown: ["gguf", "model", "endpoint", "api_key"], folded: ["args", "command", "context_tokens"] },
  connect: { shown: ["endpoint", "model", "api_key", "context_tokens"], folded: [] },
};

/** **その種類の面が持つ欄か**（出す欄でも、詳細に畳む欄でも）。 */
function kindHasKey(kind: FaceKind, key: EditableFaceKey): boolean {
  return KIND_FIELDS[kind].shown.includes(key) || KIND_FIELDS[kind].folded.includes(key);
}

/**
 * 欄の見出し。**ファイルの鍵の綴りを添える**（利用者は JSON を手で書くこともある）。
 *
 * **起こす面の `endpoint` は「待ち受ける場所」**（起こした llama がそこで待ち受ける）、繋ぐ面は「宛先」。
 */
export function fieldLabel(kind: FaceKind, key: EditableFaceKey): string {
  switch (key) {
    case "endpoint":
      return kind === "launch" ? "待ち受ける場所（endpoint）" : "宛先（endpoint）";
    case "model":
      return "依頼するモデルの綴り（model）";
    case "gguf":
      return "gguf のパス（gguf）";
    case "args":
      return "起動の引数（args）";
    case "command":
      return "実行ファイル（command）";
    case "context_tokens":
      return "コンテキスト長（context_tokens）";
    case "api_key":
      return "鍵（api_key）";
  }
}

// ----------------------------------------------------------------------
// 一覧の1行（作業単位 D-19）
// ----------------------------------------------------------------------

/**
 * **1つの面の要点**（一覧の1行）。
 *
 * | 種類 | `target` |
 * |---|---|
 * | 起こす | `gguf のファイル名 → :待ち受けのポート`（ポートが読めなければ宛先） |
 * | 繋ぐ | 宛先 |
 *
 * **宛先は、ユーザー情報・クエリ・フラグメントを落として出す**（[`endpointForSummary`] / 作業単位 B-71）。
 *
 * `extras` は、書いてあるときだけ: **鍵**（値は出さない。末尾4文字が届いていれば添える）、**窓**（書いたコンテキスト長）。
 *
 * **書いた値だけから組む**（書いた値と観測した値を混ぜない）。**書いていない鍵を、決まる値で埋めない**
 * （既定の実行ファイル名も、既定の窓も出さない）。
 */
export interface FaceSummary {
  readonly kind: FaceKind;
  readonly target: string;
  readonly extras: readonly string[];
}

/** 一覧の1行。**`summary` が `null` なら、その面を書いていない（本文と同じ上流）。** */
export interface SummaryLine {
  readonly face: Face;
  readonly label: string;
  readonly summary: FaceSummary | null;
}

/**
 * **`sub` を書いていないときの、裏方の行の文面。**
 *
 * **本文の値を写さない** —— 写すと「裏方にも同じ値を書いた」に見える。
 */
export const SAME_AS_MAIN_SUMMARY = "本文と同じ";

/** パスの最後の区切り（`/` か `\`）の後ろ。区切りが無ければ全体。 */
function fileNameOf(path: string): string {
  const parts = path.split(/[\\/]/u).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

/** **URL として読めない宛先の代わりに出す綴り**（サーバのログの `llm::endpoint_for_log` と同じ）。 */
const UNREADABLE_ENDPOINT = "<読めない宛先>";

/**
 * **一覧の要点に出す宛先。ユーザー情報・クエリ・フラグメントを落とす**（作業単位 B-71）。
 *
 * | 書いた宛先 | 出す綴り |
 * |---|---|
 * | 秘密になりうる部分（ユーザー情報・クエリ・フラグメント。中身の無い `?` / `#` も含む）が無い | **書いたまま**（既定のポートも消さない） |
 * | ある | それを落とした URL（WHATWG の綴り。サーバのログと同じ読み方） |
 * | URL として読めない・ホストを持たない | `<読めない宛先>`（伏せ損ねた綴りをそのまま出さない） |
 *
 * **落とすのは要点だけ。** 一覧の応答と「直す」の欄は書いたままの値を持つ（書いた値を見せないと直せない）。
 */
function endpointForSummary(endpoint: string): string {
  let url: URL;
  try {
    url = new URL(endpoint.trim());
  } catch {
    return UNREADABLE_ENDPOINT;
  }
  // **ホストを持たない URL は出さない**（`http://` を書き忘れた `user:pw@host:1` は、スキーム `user` の
  // ホストの無い URL として読めてしまう / サーバの `has_host` と同じ判定）。
  if (url.host === "") {
    return UNREADABLE_ENDPOINT;
  }
  const written = url.href;
  url.username = "";
  url.password = "";
  // **`""` を入れると、中身の無い `?` / `#` も消える**（サーバと同じく落とす）。
  url.search = "";
  url.hash = "";
  // **落とすものが無ければ書いたまま**（WHATWG の綴りに直すと `/` が付き、既定のポートが消える）。
  return url.href === written ? endpoint : url.href;
}

/**
 * **起こす面の待ち受けを短く言う**（`:18081`）。**ポートを読めなければ宛先**（[`endpointForSummary`] を通す）。
 *
 * 起こす面の宛先はループバックに限られる（サーバの保存の検証）ので、見分ける手掛かりはポートである。
 */
function listenOf(endpoint: string): string {
  try {
    const port = new URL(endpoint.trim()).port;
    return port === "" ? endpointForSummary(endpoint) : `:${port}`;
  } catch {
    return endpointForSummary(endpoint);
  }
}

/**
 * 数を3桁ごとに区切る（**閲覧器の地域設定に依らない** —— `toLocaleString` を使わない）。
 * 見張り番は `tests/upstream-profiles.test.ts` の「地域設定が de-DE の閲覧器でも…」（`toLocaleString` の経路を模す）。
 */
function grouped(value: number): string {
  return String(value).replace(/\B(?=(\d{3})+(?!\d))/gu, ",");
}

function faceSummary(face: WrittenFace): FaceSummary {
  const kind = kindOfFace(face);
  const target =
    face.gguf === null
      ? endpointForSummary(face.endpoint)
      : `${fileNameOf(face.gguf)} → ${listenOf(face.endpoint)}`;
  const extras: string[] = [];
  if (face.api_key_set) {
    extras.push(face.api_key_hint === null ? "鍵あり" : `鍵あり（…${face.api_key_hint}）`);
  }
  // **「窓 N」と書く**（単位を付けない —— 観測の節の「N トークン」と見え方を分ける）。
  if (face.context_tokens !== null) {
    extras.push(`窓 ${grouped(face.context_tokens)}`);
  }
  return { kind, target, extras };
}

/**
 * **1つのプロファイルを、面ごとの要点の行にする**（本文・裏方の順）。
 *
 * **本文の行に入るのは `profile.main`、裏方の行に入るのは `profile.sub`。**
 */
export function summaryLines(profile: WrittenProfile): readonly SummaryLine[] {
  return [
    { face: "main", label: faceShortLabel("main"), summary: faceSummary(profile.main) },
    {
      face: "sub",
      label: faceShortLabel("sub"),
      summary: profile.sub === null ? null : faceSummary(profile.sub),
    },
  ];
}

// ----------------------------------------------------------------------
// 下書き
// ----------------------------------------------------------------------

/**
 * 1つの面の下書き。**全部が文字列**（入力欄の値そのもの）。
 *
 * `args` は**1行に1つ**（引数は空白を含みうるので、空白では区切らない）。
 * `context_tokens` も**文字列のまま持つ**（数として読めない入力を、送る前に言うため / [`contextTokensFailure`]）。
 *
 * **`api_key` は新しい鍵の入力**（作業単位 A-13b-3）。**開いたときは必ず空**（画面は鍵の値を持たない）。
 * **空なら置き換えない**（いまの鍵を残す）。整えない（前後の空白も送る —— サーバが理由つきで断る）。
 */
export interface FaceDraft {
  endpoint: string;
  model: string;
  gguf: string;
  args: string;
  command: string;
  context_tokens: string;
  api_key: string;
}

/**
 * **開いたときに、その面に書いてあった鍵と承認**（一覧の `WrittenFace` から / 作業単位 A-13b-3）。
 * `endpoint` は開いたときの宛先（宛先だけを変えたかを、文字列で比べる —— サーバの承認の規則と同じ）。
 */
export interface WrittenKey {
  readonly set: boolean;
  readonly hint: string | null;
  readonly approved: boolean;
  readonly endpoint: string;
}

/**
 * **1つの面の、鍵と承認の下書き**（作業単位 A-13b-3）。
 *
 * | 欄 | 意味 |
 * |---|---|
 * | `written` | 開いたときに書いてあった鍵と承認（足す・書いていない裏方は `null`） |
 * | `cleared` | 「鍵を消す」を押した（新しい鍵より勝つ） |
 * | `carriedTo` | 「この鍵を新しい宛先へも送る」と答えた宛先（**宛先に付く** —— 宛先をまた変えれば聞き直す） |
 * | `approval` | 平文送信の承認を、どの宛先について、するか・しないか（**宛先に付く**） |
 */
export interface FaceKeyDraft {
  readonly written: WrittenKey | null;
  cleared: boolean;
  carriedTo: string | null;
  approval: { readonly endpoint: string; readonly granted: boolean } | null;
}

/** 1つのプロファイルの下書き。 */
export interface ProfileDraft {
  name: string;
  main: FaceDraft;
  /** **裏方を別の上流にするか。** `false` なら `sub` の値は送らない（下書きには残す）。 */
  hasSub: boolean;
  sub: FaceDraft;
  /**
   * **面ごとの種類**（作業単位 D-19）。**種類を切り替えても、もう一方の種類の欄の値は下書きに残す**
   * （戻せば元に戻る）。**送るのは選んだ種類の欄だけ**（[`buildSaveRequest`]）。
   */
  kind: Record<Face, FaceKind>;
  /** **面ごとの鍵と承認**（作業単位 A-13b-3 / [`FaceKeyDraft`]）。 */
  keys: Record<Face, FaceKeyDraft>;
}

function emptyFaceDraft(): FaceDraft {
  return { endpoint: "", model: "", gguf: "", args: "", command: "", context_tokens: "", api_key: "" };
}

function faceKeyDraft(face: WrittenFace | null): FaceKeyDraft {
  return {
    written:
      face === null
        ? null
        : {
            set: face.api_key_set,
            hint: face.api_key_hint,
            approved: face.allow_key_over_plain_http,
            endpoint: face.endpoint,
          },
    cleared: false,
    carriedTo: null,
    approval: null,
  };
}

/**
 * **足す**ときの初期値。**種類は両面とも起こす**（利用者が承認した絵のとおり / D-19 の是正）。
 * 裏方はチェックを入れたときに出るので、出たときに起こすで開く。
 *
 * **直すときの規則（gguf があれば起こす、無ければ繋ぐ / [`draftOfProfile`]）とは別** ——
 * 足す下書きには書いた面がまだ無いので、gguf の有無で決める材料が無い。
 */
export function emptyProfileDraft(): ProfileDraft {
  return {
    name: "",
    main: emptyFaceDraft(),
    hasSub: false,
    sub: emptyFaceDraft(),
    kind: { main: "launch", sub: "launch" },
    keys: { main: faceKeyDraft(null), sub: faceKeyDraft(null) },
  };
}

function faceDraftOf(face: WrittenFace): FaceDraft {
  return {
    endpoint: face.endpoint,
    model: face.model,
    gguf: face.gguf ?? "",
    args: face.args === null ? "" : face.args.join("\n"),
    command: face.command ?? "",
    context_tokens: face.context_tokens === null ? "" : String(face.context_tokens),
    api_key: "",
  };
}

/**
 * **直す**ときの初期値（書いたままの値）。
 *
 * **開いたときの種類は、その面に gguf があれば起こす、無ければ繋ぐ**（[`kindOfFace`]）。裏方を書いていなければ繋ぐ。
 *
 * **⚠ 手で書いたファイルの「gguf が無いのに args / command がある面」は、繋ぐで開き、保存するとその args / command は消える**
 * （繋ぐ面は送らない / [`buildSaveRequest`]）。サーバは gguf の無い面の args / command を読まないので、消えるのは使われていない値である。
 */
export function draftOfProfile(profile: WrittenProfile): ProfileDraft {
  return {
    name: profile.name,
    main: faceDraftOf(profile.main),
    hasSub: profile.sub !== null,
    sub: profile.sub === null ? emptyFaceDraft() : faceDraftOf(profile.sub),
    kind: {
      main: kindOfFace(profile.main),
      sub: profile.sub === null ? "connect" : kindOfFace(profile.sub),
    },
    keys: { main: faceKeyDraft(profile.main), sub: faceKeyDraft(profile.sub) },
  };
}

/**
 * **引数の欄を配列に読む。1行に1つ。**
 *
 * **空の行（空白だけの行を含む）は落とす。** 行の中の空白は**そのまま**（書いたまま）。
 * **1つも残らなければ `null`**（書かない）。
 *
 * **⚠ 空文字・空白だけの引数と、空の配列は、この欄では書けない**（落ちる・`null` になる）。
 * どちらも `llama serve` に渡して意味のある形ではないので、受け入れている。
 */
export function argsOfText(text: string): string[] | null {
  const args = text.split(/\r?\n/u).filter((line) => line.trim() !== "");
  return args.length === 0 ? null : args;
}

/**
 * **コンテキスト長の欄を読む**（作業単位 A-13a-2）。
 *
 * | 欄 | 読んだ結果 |
 * |---|---|
 * | 空・空白だけ | `null`（書かない。上流に聞く） |
 * | 前後の空白を落として、半角の数字だけ | その整数（**範囲は見ない** —— サーバの読み込みの検証が決め、422 でこの欄に返る） |
 * | 数字だけだが、正確に送れない桁数 | 読めない（丸めた値を送らない） |
 * | それ以外（`32k` / `1.5` / `-1` / 全角の数字 …） | 読めない |
 *
 * **範囲（応答の余白より大きいか）の規則を写さない**のは、写すとサーバと画面で規則が2つになるから。
 */
type ContextTokensReading =
  | { kind: "value"; value: number | null }
  | { kind: "not_digits" }
  | { kind: "too_large" };

function readContextTokens(text: string): ContextTokensReading {
  const trimmed = text.trim();
  if (trimmed === "") {
    return { kind: "value", value: null };
  }
  if (!/^[0-9]+$/u.test(trimmed)) {
    return { kind: "not_digits" };
  }
  const value = Number(trimmed);
  return Number.isSafeInteger(value) ? { kind: "value", value } : { kind: "too_large" };
}

/** 数として読めないときの文面。 */
const CONTEXT_TOKENS_NOT_DIGITS =
  "保存しませんでした: コンテキスト長は、半角の数字だけで書いてください（空にすると書きません）。";

/** 数字だけだが、正確に送れない桁数のときの文面。 */
const CONTEXT_TOKENS_TOO_LARGE = "保存しませんでした: コンテキスト長が大きすぎます。";

/**
 * **数として読めないコンテキスト長を、送る前に見つける**（作業単位 A-13a-2）。
 *
 * サーバの依頼の型は整数なので、読めない値を送ると本文として読めない 400 になり、
 * **どの欄が悪いかが画面に届かない。** 送らずにその欄で言う（[`joinedFlagFailure`] と同じ流れ）。
 *
 * **裏方を別にしないなら、裏方の欄は送らない**ので見ない。両方の面にあれば本文の面を先に言う。
 */
export function contextTokensFailure(draft: ProfileDraft): SaveFailure | null {
  const faces: Face[] = draft.hasSub ? ["main", "sub"] : ["main"];
  for (const face of faces) {
    const reading = readContextTokens(draft[face].context_tokens);
    if (reading.kind === "not_digits") {
      return { kind: "field", face, key: "context_tokens", message: CONTEXT_TOKENS_NOT_DIGITS };
    }
    if (reading.kind === "too_large") {
      return { kind: "field", face, key: "context_tokens", message: CONTEXT_TOKENS_TOO_LARGE };
    }
  }
  return null;
}

/**
 * **宛先のパス（末尾の `/` を除く）の最後の区切りが、ちょうど `v1` か**（作業単位 B-68 / Issue origin#97）。
 *
 * personacore は宛先の後ろに `/v1/chat/completions`・`/v1/models` などを足すので、`…/v1` まで書くと
 * `…/v1/v1/…` に送る。大文字小文字は問わない（`/V1`）。**`/v1beta`・`/api/v1x`・`/v1/api` は止めない。**
 * パスが無い宛先・URL として読めない綴りは `false`。
 *
 * **サーバの `endpoint_ends_with_v1`（`src/llm/lmstudio.rs`）の写し。** どちらも WHATWG の URL として読み
 * （`\` を `/` と読む・`.` と `..` を畳む）、**両側が同じ表**（`tests/fixtures/endpoint_v1_suffix_corpus.tsv`）を読む
 * —— サーバの判定を変えれば `the_v1_suffix_corpus_matches_the_judgment` が、画面が追随していなければ
 * `web/tests/upstream-profiles.test.ts` が赤になる。
 */
export function endpointEndsWithV1(endpoint: string): boolean {
  let path: string;
  try {
    path = new URL(endpoint.trim()).pathname;
  } catch {
    return false;
  }
  const last = path.replace(/\/+$/u, "").split("/").at(-1) ?? "";
  return last.toLowerCase() === "v1";
}

/** 末尾が `/v1` の宛先を止めたときの文面（[`endpointV1Failure`]）。 */
const ENDPOINT_ENDS_WITH_V1 =
  "保存しませんでした: 末尾の /v1 は書かないでください（personacore が /v1 を足すので、/v1/v1/… に送ることになります）。ホストとポートまで書いてください（例 http://127.0.0.1:8080）。";

/**
 * **末尾が `/v1` の宛先は送らずに、その面の endpoint の欄で言う**（作業単位 B-68 / Issue origin#97）。
 *
 * **黙って落とさない** —— パスの前置きを本当に `/v1` で終えている上流と区別できない（Issue の終わりの条件）。
 * **サーバは受け付ける**（既に `upstreams.json` にそう書いた利用者の起動を止めない。警告のログを1行出すだけ）
 * ので、止めるのは画面の保存の前だけ。
 *
 * **起こす面も止める**（起こした llama へも、宛先に `/v1/…` を足して送る）。
 * **裏方を別にしないなら、裏方の欄は送らない**ので見ない。両方の面にあれば本文の面を先に言う。
 */
export function endpointV1Failure(draft: ProfileDraft): SaveFailure | null {
  const faces: Face[] = draft.hasSub ? ["main", "sub"] : ["main"];
  for (const face of faces) {
    if (endpointEndsWithV1(draft[face].endpoint)) {
      return { kind: "field", face, key: "endpoint", message: ENDPOINT_ENDS_WITH_V1 };
    }
  }
  return null;
}

/**
 * **空の欄は「書かない」**（`null`）。`endpoint` / `model` は空でもそのまま送る（サーバが理由を返す）。
 *
 * **その種類に無い欄は送らない**（`null` ＝ ファイルから消える / 作業単位 D-19）——
 * **繋ぐ面の gguf・引数・実行ファイル**。下書きの値は残っているが、送らない。
 *
 * **⚠ gguf が空の起こす面は、ここへ渡す前に [`launchWithoutGgufFailure`] で止めること**
 * （ここでは `gguf: null` になり、サーバは繋ぐ面として書く —— 画面は保存の前に必ず止めている）。
 *
 * **⚠ コンテキスト長が読めない下書きは、ここへ渡す前に [`contextTokensFailure`] で止めること**
 * （読めない値はここでは `null` になる —— 画面は保存の前に必ず止めている）。
 *
 * **鍵と承認は、変えるときだけ送る**（[`keyRequest`] / [`approvalRequest`]）。
 */
function faceOfDraft(draft: ProfileDraft, face: Face): WrittenFaceRequest {
  const values = draft[face];
  const kind = draft.kind[face];
  const window = readContextTokens(values.context_tokens);
  const written = (key: EditableFaceKey, value: string): string | null =>
    !kindHasKey(kind, key) || value === "" ? null : value;
  const request: WrittenFaceRequest = {
    endpoint: values.endpoint,
    model: values.model,
    gguf: written("gguf", values.gguf),
    args: kindHasKey(kind, "args") ? argsOfText(values.args) : null,
    command: written("command", values.command),
    context_tokens: kindHasKey(kind, "context_tokens") && window.kind === "value" ? window.value : null,
  };
  const key = keyRequest(draft, face);
  if (key !== undefined) {
    request.api_key = key;
  }
  const approval = approvalRequest(draft, face);
  if (approval !== undefined) {
    request.allow_key_over_plain_http = approval;
  }
  return request;
}

/**
 * **送る本文を組む。**
 *
 * **6つの鍵を必ず送る**（`null` と省略はサーバで同じ意味。形を1つにする）。
 * **`api_key` と `allow_key_over_plain_http` は、変えるときだけ送る**（省略は「残す」「今の規則」で、
 * `null` / `false` とは意味が違う / ADR-0056 §決定3・作業単位 A-13b-3）。
 * **`sub` も必ず送る**——`null` は「このプロファイルに裏方の面を書かない」。
 * **面ごとに、選んだ種類の欄だけを値として送る**（[`KIND_FIELDS`] / 作業単位 D-19）。
 *
 * **⚠ 保存の前に [`nextKeyQuestion`] が `null` になるまで聞くこと**（画面は必ず聞いてから送る）。
 * 聞かずにここを通すと、宛先だけを変えた面の鍵は新しい宛先へ残り、承認の要る面は承認なしで書かれる
 * （後者は送るときにサーバが止める）。
 */
export function buildSaveRequest(
  draft: ProfileDraft,
  revision: string | null,
): SaveRequest {
  return {
    revision,
    main: faceOfDraft(draft, "main"),
    sub: draft.hasSub ? faceOfDraft(draft, "sub") : null,
  };
}

// ----------------------------------------------------------------------
// 鍵と承認（作業単位 A-13b-3 / Issue origin#86 / ADR-0056 §決定3・5）
// ----------------------------------------------------------------------

/**
 * **依頼の `api_key`**（`undefined` は送らない＝いまの鍵を残す）。
 *
 * | 下書き | 送る |
 * |---|---|
 * | 消す | `null` |
 * | 新しい鍵を書いた | その綴り |
 * | 触らない | 送らない |
 *
 * **面の種類に依らない**（鍵は両方の種類の欄 / [`KIND_FIELDS`]）。
 */
function keyRequest(draft: ProfileDraft, face: Face): string | null | undefined {
  if (draft.keys[face].cleared) {
    return null;
  }
  return draft[face].api_key === "" ? undefined : draft[face].api_key;
}

/** **保存の後に、その面に鍵があるか**（消さず、新しい鍵を書いたか、書いてあった）。 */
function hasKeyAfterSave(draft: ProfileDraft, face: Face): boolean {
  if (draft.keys[face].cleared) {
    return false;
  }
  return draft[face].api_key !== "" || draft.keys[face].written?.set === true;
}

/**
 * **依頼の `allow_key_over_plain_http`**（`undefined` は送らない＝今の規則 —— 宛先が文字列として同じなら残す）。
 *
 * * **保存の後に鍵が無い面**: 書いてあった承認があれば `false`（外す。鍵の無い承認を残さない）、無ければ送らない。
 * * **鍵がある面**: いまの宛先について答えた承認があれば、それ（`true` / `false`）。無ければ送らない。
 */
function approvalRequest(draft: ProfileDraft, face: Face): boolean | undefined {
  const keys = draft.keys[face];
  if (!hasKeyAfterSave(draft, face)) {
    return keys.written?.approved === true ? false : undefined;
  }
  const decided = keys.approval;
  return decided !== null && decided.endpoint === draft[face].endpoint ? decided.granted : undefined;
}

/** **保存の後に、その面に承認が書かれているか**（送る依頼と、サーバの今の規則から）。 */
function approvedAfterSave(draft: ProfileDraft, face: Face): boolean {
  const requested = approvalRequest(draft, face);
  if (requested !== undefined) {
    return requested;
  }
  const written = draft.keys[face].written;
  return written !== null && written.approved && written.endpoint === draft[face].endpoint;
}

/** **その面の鍵を消す**（新しい鍵の欄も空にする —— 消すほうが勝つ）。 */
export function clearKey(draft: ProfileDraft, face: Face): void {
  draft.keys[face].cleared = true;
  draft[face].api_key = "";
}

/** **消すのをやめる**（いまの鍵を残す）。 */
export function keepKey(draft: ProfileDraft, face: Face): void {
  draft.keys[face].cleared = false;
}

/** **いまの宛先について、承認を外す**（`false` を送る）。 */
export function revokeApproval(draft: ProfileDraft, face: Face): void {
  draft.keys[face].approval = { endpoint: draft[face].endpoint, granted: false };
}

/**
 * **いまの宛先について、承認する**（`true` を送る / ホスト名の宛先の「承認する」から —— 確認を経てから呼ぶこと）。
 */
export function grantApproval(draft: ProfileDraft, face: Face): void {
  draft.keys[face].approval = { endpoint: draft[face].endpoint, granted: true };
}

/** **承認について答えたことを取り消す**（今の規則に戻す。外す前・承認する前の状態）。 */
export function resetApproval(draft: ProfileDraft, face: Face): void {
  draft.keys[face].approval = null;
}

/**
 * **鍵の欄の状態の一言**（値は出さない）。
 *
 * | 下書き | 文面 |
 * |---|---|
 * | 消す | 保存すると、鍵を消します。 |
 * | 新しい鍵を書いた（書いてあった鍵がある / 無い） | 保存すると、新しい鍵に置き換えます。 / 保存すると、この鍵を書きます。 |
 * | 書いてあった（末尾4文字がある / 短い鍵） | 設定済みです（…末尾4文字）。 / 設定済みです（短い鍵なので、末尾は出しません）。 |
 * | 無い | 書いていません。 |
 */
export function keyStatusText(draft: ProfileDraft, face: Face): string {
  const keys = draft.keys[face];
  const set = keys.written?.set === true;
  if (keys.cleared) {
    return "保存すると、鍵を消します。";
  }
  if (draft[face].api_key !== "") {
    return set ? "保存すると、新しい鍵に置き換えます。" : "保存すると、この鍵を書きます。";
  }
  if (!set) {
    return "書いていません。";
  }
  const hint = keys.written?.hint ?? null;
  return hint === null ? "設定済みです（短い鍵なので、末尾は出しません）。" : `設定済みです（…${hint}）。`;
}

/**
 * **「裏方を別の上流にする」のチェックを外したとき、裏方の鍵も消えることを言う**（利用者の決定2 / 2026-11-02）。
 *
 * **振る舞いは変えない**（`sub: null` で裏方の面が丸ごと消え、鍵も一緒に消える）。言うだけ。
 */
export function subKeyLossNote(draft: ProfileDraft): string | null {
  return !draft.hasSub && draft.keys.sub.written?.set === true ? SUB_KEY_LOSS_NOTE : null;
}

/** [`subKeyLossNote`] の文面。 */
const SUB_KEY_LOSS_NOTE = "保存すると裏方の鍵も消えます";

/**
 * **鍵の欄の隣に出す、送り方の状態**（保存した後の形で言う / ADR-0056 §決定5）。
 *
 * | `kind` | いつ |
 * |---|---|
 * | `none` | 保存の後に鍵が無い、か、宛先が `https://`・ループバック・読めない綴り |
 * | `local_network` | `http://` のローカルネットワーク（警告して送る） |
 * | `ask` | **IP アドレスで**ローカルネットワークの外の `http://` で、承認が無く、まだ答えていない（保存の前に聞く） |
 * | `name` | ホスト名の `http://` で、承認が無く、まだ答えていない（**保存の前には止めない**。欄の隣に「承認する」を置く） |
 * | `kept` | 書いてあった承認が、宛先が同じなので残る |
 * | `granting` | いまの宛先について承認すると答えた |
 * | `denying` | いまの宛先について承認しない・外すと答えた |
 *
 * # ホスト名では、保存の前に止めない（作業単位 A-13b-3 のレビュー是正 / 司令塔の決定）
 *
 * 着手時はホスト名でも保存の前に聞いていた。「承認せずに保存する」はファイルに何も残さないので、
 * **承認の無いホスト名の面（`http://relay.example:…` のように、宛先を名前で書いた面）は、モデル名を直すだけで毎回聞かれた。**
 * ホスト名は名前解決するまで外かどうか分からず、送る側が送るたびに判定するので、画面は止めずに言うだけにする。
 */
export type KeySendingState =
  | { readonly kind: "none" }
  | { readonly kind: "local_network" }
  | { readonly kind: "ask" }
  | { readonly kind: "name" }
  | { readonly kind: "kept" }
  | { readonly kind: "granting" }
  | { readonly kind: "denying" };

/**
 * **送り方の状態の一言**（鍵の欄の隣に出す）。`none` は何も言わない。
 *
 * **ローカルネットワークには警告を出す**（ADR-0056 §決定5 —— 警告して送る。承認は求めない）。
 */
export function keySendingNote(state: KeySendingState): string | null {
  switch (state.kind) {
    case "none":
      return null;
    case "local_network":
      return "この宛先は http:// のローカルネットワークなので、鍵は暗号化されずに送られます（承認は要りません）。同じネットワークの機器は、その鍵を読めます。";
    case "ask":
      return "この宛先は http:// でローカルネットワークの外なので、保存の前に、鍵を暗号化せずに送ることの承認を求めます。";
    case "name":
      return "名前で書いた宛先は、送るたびに名前解決して判定します。ローカルネットワークの外に解決したら、承認が無ければ送りません。";
    case "kept":
      return "この宛先へ、鍵を暗号化せずに送ることを承認しています。";
    case "granting":
      return "保存すると、この宛先へ鍵を暗号化せずに送ることを承認します。";
    case "denying":
      return "承認せずに保存します（ローカルネットワークの外へは鍵を送りません）。";
  }
}

/** **鍵の欄の注記**（値を出さないこと・空なら残すこと・どこへ付くか）。 */
export const KEY_INPUT_NOTE =
  "鍵は、この面への全部の送信に Authorization: Bearer で付きます。新しい鍵を書いたときだけ置き換え、空のままなら、いまの鍵を残します（書いていなければ書きません）。書いた鍵は、保存した後も画面に出しません。";

/** **その面の送り方の状態**（[`KeySendingState`]）。 */
export function keySendingState(draft: ProfileDraft, face: Face): KeySendingState {
  if (!hasKeyAfterSave(draft, face)) {
    return { kind: "none" };
  }
  const destination = keyDestination(draft[face].endpoint);
  if (destination === "unguarded") {
    return { kind: "none" };
  }
  if (destination === "local_network") {
    return { kind: "local_network" };
  }
  const requested = approvalRequest(draft, face);
  if (requested !== undefined) {
    return { kind: requested ? "granting" : "denying" };
  }
  if (approvedAfterSave(draft, face)) {
    return { kind: "kept" };
  }
  return destination === "name" ? { kind: "name" } : { kind: "ask" };
}

/**
 * **保存の前に聞くこと**（作業単位 A-13b-3）。
 *
 * | `kind` | いつ | 答え |
 * |---|---|---|
 * | `carry` | 鍵のある面で、鍵の欄に触れずに宛先だけを変えた（利用者の決定1。起こす面にも効く） | 送る / 鍵を消す / やめる |
 * | `approve` | 保存の後に鍵があり、宛先が **IP アドレスで**ローカルネットワークの外の `http://` で、承認が無い（ADR-0056 §決定5） | 承認する / 承認せずに保存する / やめる |
 *
 * **ホスト名の宛先では聞かない**（[`KeySendingState`] の `name`）。起こす面の宛先はループバックだけなので、承認は聞かない。
 */
export type KeyQuestion =
  | {
      readonly kind: "carry";
      readonly face: Face;
      readonly hint: string | null;
      readonly from: string;
      readonly to: string;
    }
  | {
      readonly kind: "approve";
      readonly face: Face;
      readonly endpoint: string;
    };

/** 確認への答え（`carry` には `carry` / `clear`、`approve` には `grant` / `deny`）。 */
export type KeyAnswer = "carry" | "clear" | "grant" | "deny";

/**
 * **次に聞くこと**（無ければ `null` —— 送ってよい）。
 *
 * **鍵の確認を先に、承認を後に聞く**（鍵を消すと答えれば、承認は要らなくなる）。どちらも本文 → 裏方の順。
 * **裏方を別の上流にしないなら、裏方については聞かない**（送らない）。
 * **答えは宛先に付く**（[`FaceKeyDraft`]）—— 答えた後に宛先を変えれば、聞き直す。
 */
export function nextKeyQuestion(draft: ProfileDraft): KeyQuestion | null {
  const faces: Face[] = draft.hasSub ? ["main", "sub"] : ["main"];
  for (const face of faces) {
    const keys = draft.keys[face];
    const written = keys.written;
    const endpoint = draft[face].endpoint;
    if (
      written?.set === true &&
      !keys.cleared &&
      draft[face].api_key === "" &&
      endpoint !== written.endpoint &&
      keys.carriedTo !== endpoint
    ) {
      return { kind: "carry", face, hint: written.hint, from: written.endpoint, to: endpoint };
    }
  }
  for (const face of faces) {
    const state = keySendingState(draft, face);
    if (state.kind === "ask") {
      return { kind: "approve", face, endpoint: draft[face].endpoint };
    }
  }
  return null;
}

/**
 * **確認への答えを下書きに書く**（画面はこの後、もう一度 [`nextKeyQuestion`] から始める）。
 *
 * 答えは**聞いたときの宛先**に付ける（聞いた後に宛先が変わっていれば、その答えは効かず、聞き直しになる）。
 */
export function answerKeyQuestion(draft: ProfileDraft, question: KeyQuestion, answer: KeyAnswer): void {
  const keys = draft.keys[question.face];
  switch (answer) {
    case "carry":
      if (question.kind === "carry") {
        keys.carriedTo = question.to;
      }
      return;
    case "clear":
      clearKey(draft, question.face);
      return;
    case "grant":
    case "deny":
      if (question.kind === "approve") {
        keys.approval = { endpoint: question.endpoint, granted: answer === "grant" };
      }
      return;
  }
}

/**
 * **欄の無い鍵を名指した失敗を、全体に出す**（作業単位 D-19）。
 *
 * 繋ぐ面には gguf・引数・実行ファイルの欄が無い。**出す先の無い欄に結びつけると、理由が画面から消える**
 * （[`describeSaveFailure`] が裏方を書いていない下書きでしていることと同じ）。
 * 画面は保存の失敗をすべてここに通してから出す。
 *
 * **詳細に畳んだ欄は「在る欄」**（画面が詳細を開いて見せる）。
 */
export function keepOnShownField(
  failure: SaveFailure,
  kind: Readonly<Record<Face, FaceKind>>,
): SaveFailure {
  if (failure.kind === "field" && !kindHasKey(kind[failure.face], failure.key)) {
    return { kind: "form", message: failure.message };
  }
  return failure;
}

/**
 * **いま画面に在る欄にだけ結びつける**（D-19 のレビュー）。画面は理由を出すたびにここを通す。
 *
 * 理由が欄に出た後で、利用者が種類を切り替えたり裏方のチェックを外したりすると、その欄は画面から消える。
 * **欄と一緒に理由まで消さず、下書き全体へ移す**（[`keepOnShownField`] と同じ考え方）。**戻せば欄へ戻る**
 * （持っている失敗そのものは書き換えない）。
 *
 * | 欄が画面に無いとき | |
 * |---|---|
 * | 裏方の欄で、裏方を別の上流にしていない | 全体へ |
 * | その面の種類に無い鍵（繋ぐ面の gguf・引数・実行ファイル） | 全体へ |
 */
export function failureOnScreen(
  failure: SaveFailure,
  draft: { readonly hasSub: boolean; readonly kind: Readonly<Record<Face, FaceKind>> },
): SaveFailure {
  if (failure.kind !== "field") {
    return failure;
  }
  if (failure.face === "sub" && !draft.hasSub) {
    return { kind: "form", message: failure.message };
  }
  return keepOnShownField(failure, draft.kind);
}

/** 起こす面の gguf が空のときの文面。 */
const LAUNCH_WITHOUT_GGUF =
  "保存しませんでした: 起こす面には gguf のパスを書いてください。繋ぐなら「動いているサーバに繋ぐ」を選んでください。";

/**
 * **起こす面で gguf を空のまま保存しない**（D-19 のレビュー）。
 *
 * サーバは gguf を書いた面を起こす（ファイルに種類の鍵は無い）。起こすを選んだまま gguf を空で送ると、
 * **保存は通って黙って繋ぐ面として書かれ**、一覧に「繋ぐ http://127.0.0.1:…」と出る。切り替えると
 * **誰も待ち受けていない宛先へ繋ぎに行き**、次に直して保存すると引数・実行ファイルも消える（繋ぐ面として開くので）。
 * **送らずに、その面の gguf の欄で言う**（[`joinedFlagFailure`] と同じ流れ）。
 *
 * **空白だけの gguf も空として止める**（パスとして意味が無い）。
 * **裏方を別にしないなら、裏方の欄は送らない**ので見ない。両方の面にあれば本文の面を先に言う。
 */
export function launchWithoutGgufFailure(draft: ProfileDraft): SaveFailure | null {
  const faces: Face[] = draft.hasSub ? ["main", "sub"] : ["main"];
  for (const face of faces) {
    if (draft.kind[face] === "launch" && draft[face].gguf.trim() === "") {
      return { kind: "field", face, key: "gguf", message: LAUNCH_WITHOUT_GGUF };
    }
  }
  return null;
}

/**
 * **その名前がもう一覧に在るか**（足すときに使う）。
 *
 * **在る名前へ「足す」を送ると、サーバはそのプロファイルを置き換える**（同じ `PUT`）。
 * 足すつもりの利用者が、在るプロファイルを黙って上書きしないように、画面で止める。
 * **綴りで比べる**（`--upstream <名前>` も綴りで選ぶ）。
 */
export function nameTaken(
  name: string,
  profiles: readonly WrittenProfile[],
): boolean {
  return profiles.some((profile) => profile.name === name);
}

/**
 * **旗と値を1行に書いた引数を、送る前に見つける**（作業単位 D-13 / Issue origin#29）。
 *
 * 引数の欄は**1行が引数1つ**として渡る（[`argsOfText`]。空白を含むパスを渡せるようにするため）。
 * `--ctx-size 16384` を1行に書くと**空白を含んだ1つの引数**になり、`llama` は
 * `error: invalid argument: --ctx-size 16384` で起動直後に終了する（利用者が実機で踏んだ）。
 *
 * # 何を止めるか
 *
 * **行頭の空白を読み飛ばして `-` で始まり、空白（全角・タブを含む）を含む行だけ。**
 *
 * | 行 | 止めるか | 理由 |
 * |---|---|---|
 * | `--ctx-size 16384` / `-ngl 99` | 止める | 旗と値が1つの引数になる |
 * | `--jinja␣`（後ろに空白だけ） | 止める | 空白ごと1つの引数になる |
 * | `␣--ctx-size 16384` / `␣--jinja`（行頭に空白） | 止める | **行頭の空白も刈られない**（`argsOfText` もサーバも行を刈らない） |
 * | `C:\Program Files\x.gguf` | 止めない | **空白を含む値は正当**（1行1引数にした理由そのもの） |
 * | `--alias=two words` | 止めない | 旗と値を `=` でつないだ1つの引数。空白は値の中（行頭に空白があれば止める） |
 * | `␣C:\x.gguf`（`-` で始まらない値の行に行頭の空白） | **止めない** | **この判定の外**（値の行は形を問わない） |
 *
 * **行頭の空白は、読み飛ばして判定し、あれば「先頭の空白も消す」と言う**（断る理由は同じ。
 * 読み飛ばすのは、旗と値の1行書きと行頭の空白が重なった行で、直す場所を1回で全部言うため）。
 *
 * **サーバは断らない**（`upstreams.json` の読み込みの受理集合を変えない —— 手で書いた既存のファイルを
 * 読めなくしないため）。**画面の保存だけが止める**（足すときに在る名前を打ったのと同じ流れ）。
 *
 * **裏方を別にしないなら、裏方の欄は送らない**ので見ない。両方の面にあれば本文の面を先に言う。
 * **繋ぐ面の引数も送らない**ので見ない（作業単位 D-19）。
 */
export function joinedFlagFailure(draft: ProfileDraft): SaveFailure | null {
  const faces: Face[] = draft.hasSub ? ["main", "sub"] : ["main"];
  for (const face of faces) {
    if (!kindHasKey(draft.kind[face], "args")) {
      continue;
    }
    for (const line of argsOfText(draft[face].args) ?? []) {
      const message = joinedFlagMessage(line);
      if (message !== null) {
        return { kind: "field", face, key: "args", message };
      }
    }
  }
  return null;
}

/** その1行が「旗と値を1行に書いた」形なら、画面に出す文面。そうでなければ `null`。 */
function joinedFlagMessage(line: string): string | null {
  const leading = /^\s+/u.exec(line)?.[0] ?? "";
  const body = line.slice(leading.length);
  if (!body.startsWith("-")) {
    return null;
  }
  const found = /^(\S+)\s+([\s\S]*)$/u.exec(body);
  const flag = found === null ? body : (found[1] as string);
  const rest = found === null ? "" : (found[2] as string).trim();
  if (rest !== "" && !flag.includes("=")) {
    const shown = body.trimEnd();
    const tail = leading === "" ? "" : "行の先頭の空白も消してください。";
    return `保存しませんでした: 「${shown}」は、空白ごと1つの引数として渡るので、上流が起動しません。${flag} と ${rest} は別の行に書いてください。${tail}`;
  }
  const trailing = found !== null && rest === "";
  if (leading === "" && !trailing) {
    return null;
  }
  const where = leading === "" ? "後ろ" : trailing ? "前後" : "前";
  return `保存しませんでした: 「${body.trimEnd()}」の${where}に空白があります。空白ごと1つの引数として渡るので、上流が起動しません。空白を消してください。`;
}

/** 足すときに、在る名前を打ったときの文面。 */
export const NAME_TAKEN_MESSAGE =
  "その名前のプロファイルはもうあります。直すときは、一覧のその名前の「直す」から開いてください。";

// ----------------------------------------------------------------------
// 保存の失敗
// ----------------------------------------------------------------------

/**
 * **保存を断られた理由を、画面のどこに出すか。**
 *
 * | `kind` | どこに |
 * |---|---|
 * | `field` | **その面のその鍵の欄**（422 `upstream_profile_invalid` で面と鍵が分かるとき・送る前に止めたとき） |
 * | `name` | 名前の欄（400 `upstream_profile_name_invalid`） |
 * | `changed` | 全体。**読み直す操作と一緒に**（409 `upstream_profiles_changed`） |
 * | `form` | 全体 |
 */
export type SaveFailure =
  | { kind: "field"; face: Face; key: EditableFaceKey; message: string }
  | { kind: "name"; message: string }
  | { kind: "changed"; message: string }
  | { kind: "form"; message: string };

/**
 * **409 `upstream_profiles_changed` の文面。**
 *
 * **上書きしていないことを言う**（利用者は「保存できなかった」と「消えた」を区別したい）。
 */
const CHANGED_MESSAGE =
  "upstreams.json は、この画面が読んだ後に外で書き換えられています。上書きはしていません。読み直してから、もう一度直してください。";

function isFace(value: string | null | undefined): value is Face {
  return FACES.some((face) => face === value);
}

/**
 * **画面に欄がある鍵か**（[`EDITABLE_FACE_KEYS`]）。
 *
 * **`api_key` を名指した理由は、その面の鍵の欄に出す**（作業単位 A-13b-3。A-13b-1・A-13b-2 の間は欄が無く、全体に出した）。
 * **`allow_key_over_plain_http` と `sampling` を名指した理由は全体に出す**（欄の無い鍵に結びつけると、理由が画面から消える）。
 */
function isEditableFaceKey(value: string | null | undefined): value is EditableFaceKey {
  return EDITABLE_FACE_KEYS.some((key) => key === value);
}

/**
 * **保存の失敗を読み分ける。**
 *
 * @param hasSub 送った下書きが裏方の欄を持っていたか。**持っていなければ、裏方の欄は画面に無い**ので、
 *   裏方を名指しした理由も全体に出す（出す先の無い欄に結びつけて、理由を消さない）。
 */
export function describeSaveFailure(thrown: unknown, hasSub: boolean): SaveFailure {
  if (!(thrown instanceof ApiRequestError)) {
    return { kind: "form", message: "予期しない失敗です" };
  }
  if (thrown.code === "upstream_profile_invalid") {
    const reason = thrown.details["reason_message"] ?? thrown.message;
    const message = `保存しませんでした: ${reason}`;
    const face = thrown.details["face"];
    const key = thrown.details["key"];
    if (isFace(face) && isEditableFaceKey(key) && (face === "main" || hasSub)) {
      return { kind: "field", face, key, message };
    }
    return { kind: "form", message };
  }
  if (thrown.code === "upstream_profile_name_invalid") {
    return { kind: "name", message: thrown.message };
  }
  if (thrown.code === "upstream_profiles_changed") {
    return { kind: "changed", message: CHANGED_MESSAGE };
  }
  return { kind: "form", message: thrown.message };
}

// ----------------------------------------------------------------------
// 起こした上流の状態（`GET /admin/upstream` の `launch`）
// ----------------------------------------------------------------------

/**
 * **起こした上流の状態の綴り**（`launch.rs` の `FaceState::label` の写し。**鍵は変種の名前**）。
 *
 * **鍵を変種の名前にしてある**のは、見張り番（`tests/upstream-profiles.test.ts` の
 * 「状態の綴りは、変種ごとに `FaceState::label` と一致する」）が**どの綴りがどの状態か**まで突き合わせるため。
 * **会話の画面は「`Ready` なら送れる」「`Loading` なら止める」をこの綴りで決める**（`bodyFace.ts` / 作業単位 D-16）
 * ——綴りの集合が一致していても、2つを入れ替えると「載っているのに送れない」になる。
 */
export const LAUNCH_STATE = {
  External: "external",
  NotStarted: "not_started",
  Loading: "loading",
  Ready: "ready",
  Failed: "failed",
  Stopped: "stopped",
} as const;

/**
 * **この実行ファイルが、その面の上流をどうしているか**（ADR-0042 §決定3）。
 *
 * **6つの状態を6つとも別の文面にする。** 知らない綴りはそのまま出す
 * （サーバが次の状態を足した日に、欄が消えるのがいちばん質の悪い壊れ方）。
 *
 * **`since` は読み手の時間帯の `HH:MM:SS` と、`now` からの経過で出す**（Issue origin#40）——
 * `載せています（13:25:15 から、12 秒経過）`。サーバの綴りは時差つき・小数部 9 桁の RFC 3339
 * （`2026-09-26T13:25:15.770371400+09:00`）で、そのままでは読みにくいと利用者が判断した。
 * 読むのは `timestamp.ts` の [`instantOf`]（`timeLabel` は `Z` 終わりしか受けない）。
 *
 * * **読めない綴り・読めない時間帯は、今までどおり生の綴りのまま出す**（手を出さない）。
 * * **経過が負（時計のずれ）なら、経過を出さない**（時刻だけ）。
 * * **`now` と `zone` は呼び出し側が渡す**（純粋関数に保つ）。画面は読み直すたびに
 *   読んだ時刻を渡すので、経過は読み直しごとに進む。
 *
 * @param now 経過を測る「いま」。
 * @param zone 時間帯データベースの識別子（画面が `browserZone()` で解決して渡す）。
 */
export function launchLabel(launch: LaunchView, now: Date, zone: string): string {
  switch (launch.state) {
    case LAUNCH_STATE.External:
      return "起こしていません（外の上流に依頼しています）";
    // **起こすのを待っている**（ADR-0055 §決定1・3 / 作業単位 F-10b）。**`external` と同じ書き出しにしない**
    // —— 外の上流は起こす物が無く、こちらは起こせば起きる。起こすボタンは別の行が出す
    // （本文の面だけ。[`profileToStart`]）ので、ここではボタンの話をしない（裏方の面にも同じ文面が出る）。
    case LAUNCH_STATE.NotStarted:
      return "まだ起こしていません（前回選んだプロファイルは、起動しただけでは起こしません）";
    case LAUNCH_STATE.Loading:
      return launch.since === null ? "載せています" : `載せています（${sinceLabel(launch.since, now, zone)}）`;
    case LAUNCH_STATE.Ready:
      return launch.answered_name === null
        ? "準備完了"
        : `準備完了（上流が名乗った名前: ${launch.answered_name}）`;
    case LAUNCH_STATE.Failed: {
      const reason =
        launch.reason === null ? "失敗しました（理由は届いていません）" : `失敗しました: ${launch.reason}`;
      return launch.log === null ? reason : `${reason}（ログ: upstream-logs の ${launch.log}）`;
    }
    case LAUNCH_STATE.Stopped:
      return "止めました";
    default:
      return launch.state;
  }
}

/** `launchLabel` の括弧の中身（上の doc）。 */
function sinceLabel(since: string, now: Date, zone: string): string {
  const at = instantOf(since);
  const wall = at === null ? null : wallTimeOf(at, zone);
  if (at === null || wall === null) {
    return `${since} から`;
  }
  const seconds = Math.floor((now.getTime() - at.getTime()) / 1000);
  if (seconds < 0) {
    return `${wall} から`;
  }
  return `${wall} から、${elapsedLabel(seconds)}経過`;
}

/**
 * 経過秒の綴り。**60 秒未満は `N 秒`（0 秒も含む）、それ以上は 0 の桁を省く**
 * （`1 分` / `1 時間 15 分 3 秒` / `2 時間` / `1 時間 3 秒`）。
 */
function elapsedLabel(seconds: number): string {
  if (seconds < 60) {
    return `${seconds} 秒`;
  }
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  const parts: string[] = [];
  if (hours > 0) parts.push(`${hours} 時間`);
  if (minutes > 0) parts.push(`${minutes} 分`);
  if (rest > 0) parts.push(`${rest} 秒`);
  return parts.join(" ");
}

/** **どちらかの面が載せている最中か**（画面が読み直し続けるか）。 */
export function hasLoadingFace(report: UpstreamReport): boolean {
  return (
    report.main.launch.state === LAUNCH_STATE.Loading ||
    report.sub.launch.state === LAUNCH_STATE.Loading
  );
}

// ----------------------------------------------------------------------
// 起こすのを待っている上流を起こす（ADR-0055 §決定3 / 作業単位 F-10b）
// ----------------------------------------------------------------------

/**
 * **起こすボタンが送る名前。** ボタンを出さないなら `null`。
 *
 * | 本文の面 | `selected_profile` | 返すもの |
 * |---|---|---|
 * | `not_started` | 名前がある | **その名前**（同じ名前の `POST /admin/upstream/profile` が待っている面を起こす） |
 * | `not_started` | `null` | `null`（**無い名前を頼ませない**。会話の画面は上流モデルの画面への導線に倒す） |
 * | それ以外 | どちらでも | `null` |
 *
 * **本文の面だけを見る。裏方の面の状態では出さない**（ADR-0055 §決定3・ADR-0042 §決定3 の追記）。
 * **名前は一覧から選ばない** —— 一覧（`GET /admin/upstream/profiles`）は、どれが選ばれているかを言わない。
 * サーバが言う `selected_profile` だけが、いま選ばれている名前である。
 */
export function profileToStart(report: UpstreamReport): string | null {
  return report.main.launch.state === LAUNCH_STATE.NotStarted ? report.selected_profile : null;
}

/** **起こすボタンの文面。** どのプロファイルを起こすのかを名乗る（同じ名前の差し替えなので、待っている面は全部起きる）。 */
export function startButtonLabel(name: string): string {
  return `「${name}」を起こす`;
}

/** 起こす依頼の応答を待っている間の一言。 */
export function startingLabel(name: string): string {
  return `「${name}」を起こしています…`;
}

/**
 * **起こす前に確かめてほしいこと**（ADR-0055 の背景）。
 *
 * 起動時に起こさなくしたのは、**そのあいだに別の上流（鍵で守られた中継の先の推論サーバ等）が GPU を使っているかもしれない**から。
 * ボタンはその判断を利用者に戻すものなので、何を確かめてから押すのかを言う。
 */
export const START_CAUTION = "ほかの推論サーバが GPU を使っていないことを確かめてから押してください。";

/**
 * **起こす依頼が断られたときの一言**（409 `upstream_switch_in_progress` 等）。
 *
 * **差し替え（`UpstreamProfiles.svelte` の「切り替えられませんでした: …」）と同じ形** ——
 * 頭に何をしようとしたかを置き、**サーバの文面をそのまま続ける**（読み分けない）。
 */
export function startFailureLabel(message: string): string {
  return `起こせませんでした: ${message}`;
}

// ----------------------------------------------------------------------
// 一覧の状態
// ----------------------------------------------------------------------

/**
 * **一覧の応答が、画面が組める形か**（`state` が文字列で、`profiles` が配列）。
 *
 * `api.ts` は返り値の形を検証しない（冒頭の ⚠）。**この節は `profiles.length` を読んで組む**ので、
 * 形の違う応答（UI のフォールバックが返した `{}` 等）で**画面ごと落ちる**より、
 * 「一覧を読めなかった」と言うほうに倒す。**中の1件ずつの形までは見ない。**
 */
export function isProfilesView(value: unknown): value is ProfilesView {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const view = value as { state?: unknown; profiles?: unknown };
  return typeof view.state === "string" && Array.isArray(view.profiles);
}

/** **その一覧から、足す・直すを出してよいか。** 読めた（か、まだ無い）書ける置き場所だけ。 */
export function canEdit(view: ProfilesView): boolean {
  return view.editable && (view.state === "ok" || view.state === "missing");
}

/** 止まった場所（プロファイル・面・鍵のうち分かるもの）。 */
function whereOf(view: ProfilesView): string {
  const problem = view.problem;
  if (problem === null) {
    return "";
  }
  // 面と鍵は ASCII の綴りなので、前後を空けて「の」でつなぐ。
  const names = [problem.face, problem.key].filter(
    (part): part is string => part !== null,
  );
  const tail = names.join(" の ");
  if (problem.profile === null) {
    return tail === "" ? "" : `（${tail}）`;
  }
  const head = `プロファイル「${problem.profile}」`;
  return tail === "" ? `（${head}）` : `（${head}の ${tail}）`;
}

const HAND_EDIT =
  "画面からは直せません。ファイルを手で直してから、読み直してください。";

/**
 * **一覧の上に出す文**（どのファイルか・直せるか・読めたか）。
 *
 * | `state` | 言うこと |
 * |---|---|
 * | `none` | 置き場所が無い。選ぶことも直すこともできない |
 * | `missing` | まだ無い（書ける置き場所なら、足すとそこに作る） |
 * | `unreadable` / `invalid` | 理由と、**画面からは直せない**こと |
 * | `ok` | 1つも無ければそう言う |
 *
 * **読むだけの置き場所では、読んでいるファイルと「ここからは直せない」ことを言う**（ADR-0042 §決定8）。
 */
export function listingNotices(view: ProfilesView): string[] {
  if (view.state === "none") {
    return [
      "この起動には、上流プロファイルの置き場所がありません。画面からは選ぶことも直すこともできません。",
    ];
  }
  const notices: string[] = [];
  if (view.path !== null) {
    if (view.editable) {
      notices.push(`書く先: ${view.path}`);
    } else {
      notices.push(
        `読んでいるファイル: ${view.path}`,
        "この起動では、ここからは直せません（読むだけです）。起動するときに PERSONACORE_UPSTREAMS が別のファイルを指しているためです。",
      );
    }
  }
  switch (view.state) {
    case "missing":
      notices.push(
        view.editable
          ? "まだファイルがありません。プロファイルを足して保存すると、この場所に作ります。"
          : "ファイルがありません。",
      );
      break;
    case "unreadable":
      notices.push(
        `ファイルを読めませんでした: ${view.problem?.message ?? "理由は届いていません"}`,
        HAND_EDIT,
      );
      break;
    case "invalid":
      notices.push(
        `ファイルが読み込みの検証に通りません: ${view.problem?.message ?? "理由は届いていません"}${whereOf(view)}`,
        HAND_EDIT,
      );
      break;
    case "ok":
      if (view.profiles.length === 0) {
        notices.push("プロファイルは1つも書かれていません。");
      }
      break;
    default:
      notices.push(`知らない状態です: ${view.state}`);
  }
  return notices;
}
