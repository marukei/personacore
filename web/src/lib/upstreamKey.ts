/**
 * **上流の鍵の、画面の側の判断と文面**（作業単位 A-13b-3 / Issue origin#86 / ADR-0056 §決定3〜5）。**I/O を持たない。**
 *
 * 使うのは3つの画面 —— プロファイルの編集（`UpstreamProfiles.svelte` / `upstreamProfiles.ts`）、
 * いまの上流のカードと帯（`upstreamStatus.ts`）、会話の画面（`ChatView.svelte`）。
 *
 * # 宛先の分類は、画面が自分でする（作業単位 A-13b-3 の判断）
 *
 * 保存の前に「承認を求めるか」「ローカルネットワークの警告を出すか」を決めるのに、宛先の分類が要る。
 * **サーバに聞く口（新しい経路）は作らず、画面が綴りから分類する**（[`keyDestination`]）。理由:
 *
 * * **画面の分類は、送るかどうかを決めない。** 決めるのはいつも送る側（`llm::LmStudioClient` の判定 ——
 *   IP アドレスは組むとき、ホスト名は接続のたびの名前解決）。画面が分類を誤っても、起きるのは
 *   「要らない承認を求める」か「承認を求めずに保存し、送るときに `upstream_key_not_approved` で止まる」のどちらかで、
 *   **承認の無い鍵が外へ出る向きには割れない。**
 * * **サーバに聞いても、ホスト名は分からないまま**である。名前解決の結果は時と場所で変わり、
 *   送る側は接続のたびに引き直す（ADR-0056 §決定5）。保存の時点で一度引いた結果で承認を省けば、
 *   後で解決先が外へ変わったときに承認なしで外へ出る形を、画面が作ることになる。
 * * 経路を1本足すと、`host_guard`・仕様書・欄名の写しの見張り番が1組ずつ増える。
 *
 * **写しの見張り番**: Rust（`src/llm/request.rs` の `key_destination` と `address_class`）と
 * 同じ表（`tests/fixtures/key_destination_corpus.tsv`）を両側が読む。Rust の判定を変えれば
 * `the_key_destination_corpus_matches_the_judgment` が、画面が追随していなければ
 * `web/tests/upstream-key.test.ts` が赤になる。
 *
 * ## ホスト名では、保存の前に止めない（作業単位 A-13b-3 のレビュー是正 / 司令塔の決定）
 *
 * **名前を引くまで分類できない**（画面は名前を引かない）。着手時は外のアドレスと同じく保存の前に承認を聞いていたが、
 * 「承認せずに保存する」はファイルに何も残さないので、**LAN に解決する名前の面は、ほかの欄を直すたびに聞かれた。**
 * いまは、保存の前に止めるのは **IP アドレスでローカルネットワークの外**の宛先だけ。ホスト名の面には、
 * 鍵の欄の隣で「送るたびに名前解決して判定する」ことを言い、承認していなければ「承認する」を置く
 * （押すと [`nameApprovalText`] で危険を示して聞き、答えを保存で書く）。
 *
 * # 写し元
 *
 * | ここ | 写し元 | 見張り番 |
 * |---|---|---|
 * | [`keyDestination`] | `src/llm/request.rs` の `key_destination` ＋ `address_class` | `tests/upstream-key.test.ts`（同じ表） |
 * | [`KEY_OVER_PLAIN_HTTP`] | 同 `KeyOverPlainHttp::label`（**変種ごと**） | 同上 |
 * | [`UPSTREAM_UNAUTHORIZED_CODE`] / [`UPSTREAM_KEY_NOT_APPROVED_CODE`] | `src/server/error.rs` の `code` | 同上 |
 */

/**
 * **鍵を付けた面の宛先の分類**（ADR-0056 §決定5 の表）。
 *
 * | 値 | 宛先 | 鍵を送る扱い |
 * |---|---|---|
 * | `unguarded` | `https://`・ループバック（`127.0.0.0/8`・`::1`・`localhost`）・読めない綴り・ほかのスキーム | そのまま（判定しない） |
 * | `local_network` | `http://` ＋ ローカルネットワークのアドレス | 警告して送る |
 * | `elsewhere` | `http://` ＋ それ以外のアドレス（`100.64.0.0/10`・IPv4 を写した IPv6 を含む） | 承認があるときだけ |
 * | `name` | `http://` ＋ ホスト名 | 接続のたびの名前解決で決まる |
 */
export type KeyDestination = "unguarded" | "local_network" | "elsewhere" | "name";

/**
 * **宛先を分類する**（`key_destination` の写し）。
 *
 * **送信と同じ読み方（WHATWG の URL）で読む** —— 閲覧器の `URL` も、Rust の `reqwest::Url` も WHATWG である
 * （`http://203.0.113.5\@127.0.0.1` は `\` を `/` と読むので、宛先は `203.0.113.5`）。
 * **前後の空白は落とす**（Rust は `trim()`）。**⚠ JavaScript の `trim()` は U+FEFF も落とす**（Rust は落とさない）。
 * その綴りでは画面が「分類できる」側に読むが、割れる向きは上の doc のとおり鍵が出る向きではない。
 */
export function keyDestination(endpoint: string): KeyDestination {
  let url: URL;
  try {
    url = new URL(endpoint.trim());
  } catch {
    return "unguarded";
  }
  if (url.protocol !== "http:") {
    return "unguarded";
  }
  const host = url.hostname;
  if (host === "" || host.toLowerCase() === "localhost") {
    return "unguarded";
  }
  const v4 = ipv4Class(host);
  if (v4 !== null) {
    return v4;
  }
  if (host.startsWith("[") && host.endsWith("]")) {
    return ipv6Class(host.slice(1, -1));
  }
  return "name";
}

/** WHATWG が正規化した IPv4（`a.b.c.d`）なら分類、そうでなければ `null`。 */
function ipv4Class(host: string): KeyDestination | null {
  const found = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/u.exec(host);
  if (found === null) {
    return null;
  }
  const first = Number(found[1]);
  const second = Number(found[2]);
  if (first === 127) {
    return "unguarded";
  }
  if (
    first === 10 ||
    (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168) ||
    (first === 169 && second === 254)
  ) {
    return "local_network";
  }
  return "elsewhere";
}

/**
 * WHATWG が正規化した IPv6（角括弧の中。小文字・`::` で縮めた形・埋め込みの IPv4 は16進に直してある）の分類。
 *
 * **ループバックは `::1` だけ**（Rust の `is_loopback`）。**IPv4 を写した IPv6 は外**（元の種類に読み替えない）。
 */
function ipv6Class(inner: string): KeyDestination {
  if (inner === "::1") {
    return "unguarded";
  }
  const head = inner.startsWith("::") ? 0 : Number.parseInt(inner.split(":")[0] ?? "", 16);
  if (Number.isNaN(head)) {
    return "elsewhere";
  }
  // fc00::/7 と fe80::/10
  if ((head & 0xfe00) === 0xfc00 || (head & 0xffc0) === 0xfe80) {
    return "local_network";
  }
  return "elsewhere";
}

/**
 * **`GET /admin/upstream` の面の `key_over_plain_http` の綴り**（`KeyOverPlainHttp::label` の写し。**鍵は変種の名前**）。
 *
 * 変種の名前で持つのは、見張り番が**どの綴りがどの状態か**まで突き合わせるため
 * （`not_approved` だけが「送っていない」—— 入れ替えると、送っていない面にカードの印が出て帯が消える）。
 */
export const KEY_OVER_PLAIN_HTTP = {
  LocalNetwork: "local_network",
  Approved: "approved",
  NotApproved: "not_approved",
} as const;

/** **推論サーバが鍵を受け付けなかった**（`error.rs` の `upstream_unauthorized` の `code`）。 */
export const UPSTREAM_UNAUTHORIZED_CODE = "upstream_unauthorized";

/** **承認が無いので鍵を送らなかった**（`error.rs` の `upstream_key_not_approved` の `code`）。 */
export const UPSTREAM_KEY_NOT_APPROVED_CODE = "upstream_key_not_approved";

/** 鍵の失敗の種類。 */
export type KeyFailure = "unauthorized" | "not_approved";

/** **その `code` が鍵の失敗か。** 違えば `null`（**ほかの失敗に案内を足さない**）。 */
export function keyFailureOf(code: string | null | undefined): KeyFailure | null {
  switch (code) {
    case UPSTREAM_UNAUTHORIZED_CODE:
      return "unauthorized";
    case UPSTREAM_KEY_NOT_APPROVED_CODE:
      return "not_approved";
    default:
      return null;
  }
}

/**
 * **会話の画面で、鍵の失敗の後に出す案内**（上流モデルの画面への導線を挟む —— `before` ＋ 導線 ＋ `after`）。
 *
 * **2つを別の文で言う**（直す場所が違う）:
 *
 * | 失敗 | 上流は鍵を見たか | 直す場所 |
 * |---|---|---|
 * | `unauthorized` | 見た（断った）**か、宛先が違う** | 鍵の欄と宛先（保存したら「切り替え直す」） |
 * | `not_approved` | **見ていない**（personacore が接続する前に止めた） | 承認か、宛先（`https://`） |
 *
 * **`unauthorized` は鍵だけを疑わせない**（作業単位 B-69 / Issue origin#98）。利用者は、上流の側で鍵を有効にしていない・
 * 宛先を `…/v1` まで書いていた（`…/v1/v1/chat/completions` に送った）のどちらでもこの失敗で止まった。
 * **宛先違いで上流が 401 を返すかは、利用者の操作順からの推論で、単独では測っていない**（origin#98 のコメント）ので、
 * 文面では「知らないパスにも 401 を返す」と言い切らない。**保存だけでは動いている上流は変わらない**ので、「切り替え直す」まで言う。
 *
 * サーバの文面（`error.rs` の `message`）は、この前の段落にそのまま出る。ここは**直す場所**だけを足す。
 * **導線の綴りは写さない**（`router.ts` の `labelFor` から出す / `ChatView.svelte`）。
 */
export function keyFailureGuidance(failure: KeyFailure): { readonly before: string; readonly after: string } {
  switch (failure) {
    case "unauthorized":
      return {
        before:
          "推論サーバは鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。",
        after: "の、いま選んでいるプロファイルの「直す」で鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。",
      };
    case "not_approved":
      return {
        before:
          "personacore は鍵を送っていません。推論サーバは鍵を見ていないので、鍵を替えても直りません。宛先が http:// でローカルネットワークの外にあり、暗号化せずに送る承認が無いためです。",
        after: "の、いま選んでいるプロファイルの「直す」で承認するか、宛先を https:// にしてください。",
      };
  }
}

/** 確認の文面（見出しと段落）。 */
export interface QuestionText {
  readonly title: string;
  readonly body: readonly string[];
}

/**
 * **平文送信の承認の確認**（ADR-0056 §決定5 —— 危険を示したうえで承認を取る）。
 *
 * **言うこと**: どの面の・どの宛先か / 鍵が暗号化されずに送られること / 経路の途中で記録されれば、
 * その記録は残り、悪用されうること / 承認しなければ送らないこと。
 *
 * **保存の前に止めて聞くのは、IP アドレスでローカルネットワークの外の宛先だけ**（作業単位 A-13b-3 のレビュー是正）。
 * ホスト名の宛先は [`nameApprovalText`]（鍵の欄の隣の「承認する」から出す）。
 */
export function approvalQuestionText(faceLabel: string, endpoint: string): QuestionText {
  return {
    title: approvalTitle(faceLabel),
    body: [
      `宛先 ${endpoint} は http:// で、ローカルネットワークの外にあります。`,
      APPROVAL_DANGER,
      "承認せずに保存すると、ローカルネットワークの外へは鍵を送りません（会話は、承認が無いので推論サーバへ送らなかったことを言って止まります）。",
    ],
  };
}

/**
 * **ホスト名の宛先の承認の確認**（鍵の欄の隣の「承認する」を押したとき / 作業単位 A-13b-3 のレビュー是正）。
 *
 * **危険の文は [`approvalQuestionText`] と同じ。** 答えは「承認する」「やめる」で、保存はしない（保存で書く）。
 */
export function nameApprovalText(faceLabel: string, endpoint: string): QuestionText {
  return {
    title: approvalTitle(faceLabel),
    body: [
      `宛先 ${endpoint} はホスト名です。送るたびに名前解決して判定し、ローカルネットワークの外のアドレスに解決したら、承認が無ければ鍵を送りません。`,
      APPROVAL_DANGER,
    ],
  };
}

function approvalTitle(faceLabel: string): string {
  return `${faceLabel}の鍵を、暗号化せずに送ることを承認しますか`;
}

/** 承認の確認が示す危険（ADR-0056 §決定5）。 */
const APPROVAL_DANGER =
  "承認すると、鍵は暗号化されずに送られます。経路の途中で記録されれば、その記録は残り、鍵が悪用されることがあります。";

/**
 * **宛先だけを変えた面の、鍵の確認**（利用者の決定1 / 2026-11-02）。
 *
 * 編集 API は、鍵を送らなければ残す（ADR-0056 §決定3）ので、何も聞かずに保存すると
 * **前の宛先の鍵が新しい宛先へ送られる**（§決定1「鍵は宛先に付く」と緊張する）。
 */
export function carryQuestionText(faceLabel: string, hint: string | null, from: string, to: string): QuestionText {
  const held = hint === null ? "この面には鍵が書いてあります。" : `この面には鍵が書いてあります（…${hint}）。`;
  return {
    title: `${faceLabel}の鍵を、新しい宛先へも送りますか`,
    body: [
      `${held}宛先を ${from} から ${to} に変えました。`,
      "鍵は宛先に付くものなので、新しい宛先へ送ってよいかを確かめます。鍵を消すと、この面は鍵を送らなくなります。",
    ],
  };
}
