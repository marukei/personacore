/**
 * **上流の鍵の、画面の側の判断と文面**（作業単位 A-13b-3 / Issue origin#86 / `src/lib/upstreamKey.ts`）。
 *
 * # 見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **宛先の分類が Rust の判定と同じ表で一致する** | 画面は保存の前に「承認を求めるか」「警告するか」を決める。ずれると、承認を求めずに保存して送れない面を作るか、要らない承認を取る |
 * | **`key_over_plain_http` の綴りが `KeyOverPlainHttp::label` と一致する**（変種ごと） | ずれると、カードの印も異常の帯も出ない |
 * | **エラーの `code` が `error.rs` と一致する** | ずれると、会話の画面の案内が黙って消える |
 * | **断られた（`upstream_unauthorized`）と送らなかった（`upstream_key_not_approved`）を別の文で言う** | 直す場所が違う（鍵か、承認か宛先か） |
 * | **承認の確認が、危険を実際に言っている** | ADR-0056 §決定5 が「示したうえで承認を取る」と決めた |
 *
 * **期待値の文面は直書き**（定数から作ると、文面を緩める変異が期待値も一緒に動かして緑になる）。
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  KEY_OVER_PLAIN_HTTP,
  UPSTREAM_KEY_NOT_APPROVED_CODE,
  UPSTREAM_UNAUTHORIZED_CODE,
  approvalQuestionText,
  carryQuestionText,
  keyDestination,
  keyFailureGuidance,
  keyFailureOf,
  nameApprovalText,
} from "../src/lib/upstreamKey";

const CORPUS = readFileSync(
  fileURLToPath(new URL("../../tests/fixtures/key_destination_corpus.tsv", import.meta.url)),
  "utf8",
);
const REQUEST_RS = readFileSync(
  fileURLToPath(new URL("../../src/llm/request.rs", import.meta.url)),
  "utf8",
);
const ERROR_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
  "utf8",
);

/** 表の行（`#` の行と空の行を除く）。**`\r` を落としてから割る**（作業ツリーは CRLF になりうる）。 */
function corpusRows(source: string): [string, string][] {
  return source
    .split(/\r?\n/u)
    .filter((line) => !line.startsWith("#") && line.trim() !== "")
    .map((line) => {
      const parts = line.split("\t");
      if (parts.length !== 2) {
        throw new Error(`宛先<TAB>分類 の形ではありません: ${JSON.stringify(line)}`);
      }
      return [parts[0] as string, parts[1] as string];
    });
}

describe("宛先の分類（Rust の `key_destination` と同じ表を読む）", () => {
  const rows = corpusRows(CORPUS);

  it.each(rows)("%j は %s", (endpoint, expected) => {
    expect(keyDestination(endpoint)).toBe(expected);
  });

  /** **読めていないから緑、を通さない**（表が痩せていない・4つの分類が全部出る）。 */
  it("表は痩せておらず、4つの分類が全部出る", () => {
    expect(rows.length).toBeGreaterThanOrEqual(40);
    expect([...new Set(rows.map(([, expected]) => expected))].sort()).toEqual([
      "elsewhere",
      "local_network",
      "name",
      "unguarded",
    ]);
  });

  it("対照: 表の読み方は、植えた表をその場で読む（CRLF も）", () => {
    expect(corpusRows("# c\r\nhttp://a\tname\r\n\r\n\tunguarded\r\n")).toEqual([
      ["http://a", "name"],
      ["", "unguarded"],
    ]);
    expect(() => corpusRows("http://a name\n")).toThrow();
  });
});

/**
 * **`impl KeyOverPlainHttp` の `fn label` の腕を、変種の名前つきで読む**（`Self::LocalNetwork => "local_network",`）。
 *
 * **集合だけでは、どの綴りが「送っていない」かを言えない** —— 2つの綴りを入れ替えると、
 * 送っていない面にカードの印が出て、帯が消える。
 */
function keyOverPlainHttpLabelsOf(source: string): Record<string, string> {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const impl = lines.findIndex((line) => line.trim() === "impl KeyOverPlainHttp {");
  if (impl < 0) {
    throw new Error("impl KeyOverPlainHttp が見つかりません");
  }
  const at = lines.findIndex(
    (line, index) => index > impl && line.trim().startsWith("pub const fn label(self) -> &'static str {"),
  );
  if (at < 0) {
    throw new Error("fn label が見つかりません");
  }
  const variants: Record<string, string> = {};
  for (let index = at + 1; index < lines.length; index += 1) {
    const line = (lines[index] ?? "").trim();
    if (line === "}") {
      break;
    }
    const found = /^Self::([A-Za-z]+) => "([^"]*)",$/u.exec(line);
    if (found?.[1] !== undefined && found[2] !== undefined) {
      variants[found[1]] = found[2];
    }
  }
  if (Object.keys(variants).length === 0) {
    throw new Error("fn label に綴りを返す腕が1つもありません");
  }
  return variants;
}

/** `error.rs` のその構築子から `code` を1つ取り出す（次の `pub const fn` の手前で切る）。 */
function codeOf(source: string, constructor: string): string {
  const start = source.indexOf(`pub const fn ${constructor}() -> Self {`);
  if (start < 0) {
    throw new Error(`${constructor} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("pub const fn ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = /code: Some\("([^"]+)"\)/u.exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`${constructor} に code がありません`);
  }
  return found[1];
}

describe("写し元の綴り", () => {
  it("`key_over_plain_http` の綴りは、変種ごとに `KeyOverPlainHttp::label` と一致する", () => {
    expect({ ...KEY_OVER_PLAIN_HTTP }).toEqual(keyOverPlainHttpLabelsOf(REQUEST_RS));
  });

  it("鍵の失敗の `code` は `error.rs` と一致する", () => {
    expect([UPSTREAM_UNAUTHORIZED_CODE, UPSTREAM_KEY_NOT_APPROVED_CODE]).toEqual([
      codeOf(ERROR_RS, "upstream_unauthorized"),
      codeOf(ERROR_RS, "upstream_key_not_approved"),
    ]);
  });

  describe("対照: 植えたソースからは、その場の綴りを読む", () => {
    it("`fn label` の腕（別の impl の `fn label` は読まない）", () => {
      const planted = [
        "impl Other {",
        "    pub const fn label(self) -> &'static str {",
        '            Self::LocalNetwork => "wrong",',
        "    }",
        "}",
        "impl KeyOverPlainHttp {",
        "    pub const fn label(self) -> &'static str {",
        "        match self {",
        '            Self::LocalNetwork => "planted_local",',
        '            Self::NotApproved => "planted_refused",',
        "        }",
        "    }",
        "}",
      ].join("\r\n");
      expect(keyOverPlainHttpLabelsOf(planted)).toEqual({
        LocalNetwork: "planted_local",
        NotApproved: "planted_refused",
      });
      expect(() => keyOverPlainHttpLabelsOf("")).toThrow();
    });

    it("`code` は、その構築子のものだけを読む", () => {
      const planted = [
        "    pub const fn upstream_unauthorized() -> Self {",
        "        Self {",
        "            status: StatusCode::BAD_GATEWAY,",
        "        }",
        "    }",
        "    pub const fn upstream_key_not_approved() -> Self {",
        '            code: Some("next_one"),',
        "    }",
      ].join("\n");
      expect(() => codeOf(planted, "upstream_unauthorized")).toThrow();
      expect(codeOf(planted, "upstream_key_not_approved")).toBe("next_one");
    });
  });
});

describe("鍵の失敗を読み分ける", () => {
  it("2つの `code` だけを鍵の失敗と読み、ほかは読まない", () => {
    expect([
      keyFailureOf("upstream_unauthorized"),
      keyFailureOf("upstream_key_not_approved"),
      keyFailureOf("upstream_failed"),
      keyFailureOf(null),
      keyFailureOf(undefined),
    ]).toEqual(["unauthorized", "not_approved", null, null, null]);
  });

  /**
   * **断られた（上流は鍵を見た）と、送らなかった（上流は鍵を見ていない）を、別の文で言う。**
   * 直す場所が違う —— 前者は鍵、後者は承認か宛先。**文面は直書きで固定する。**
   */
  /*
   * **断られたときは、鍵のほかの候補も挙げる**（作業単位 B-69 / Issue origin#98）——
   * 上流の側で鍵を有効にしていない・宛先が違う（末尾の /v1 の二重）。
   * 宛先違いで 401 が返るかは利用者の操作順からの推論で、単独では測っていないので、文面では言い切らない。
   * 直す場所は鍵の欄と宛先で、**保存した後に「切り替え直す」まで言う**（保存だけでは、動いている上流は変わらない）。
   */
  it("断られたときは鍵と宛先を確かめて切り替え直すように、送らなかったときは承認か宛先を直すように言う", () => {
    expect(keyFailureGuidance("unauthorized")).toEqual({
      before:
        "推論サーバは鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。",
      after: "の、いま選んでいるプロファイルの「直す」で鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。",
    });
    expect(keyFailureGuidance("not_approved")).toEqual({
      before:
        "personacore は鍵を送っていません。推論サーバは鍵を見ていないので、鍵を替えても直りません。宛先が http:// でローカルネットワークの外にあり、暗号化せずに送る承認が無いためです。",
      after: "の、いま選んでいるプロファイルの「直す」で承認するか、宛先を https:// にしてください。",
    });
  });
});

describe("承認の確認", () => {
  /**
   * **確認は危険を言う**（ADR-0056 §決定5 —— 鍵が暗号化されずに送られる。経路の途中で記録されれば、
   * その記録は残り、悪用されうる）。**面と宛先も名乗る**（どの面のどの宛先の話かを取り違えない）。
   */
  it("外のアドレスなら、宛先と、暗号化されないことと、記録が残って悪用されうることを言う", () => {
    expect(approvalQuestionText("裏方（抽出・変容）", "http://203.0.113.5:46001")).toEqual({
      title: "裏方（抽出・変容）の鍵を、暗号化せずに送ることを承認しますか",
      body: [
        "宛先 http://203.0.113.5:46001 は http:// で、ローカルネットワークの外にあります。",
        "承認すると、鍵は暗号化されずに送られます。経路の途中で記録されれば、その記録は残り、鍵が悪用されることがあります。",
        "承認せずに保存すると、ローカルネットワークの外へは鍵を送りません（会話は、承認が無いので推論サーバへ送らなかったことを言って止まります）。",
      ],
    });
  });

  /**
   * **ホスト名の宛先の確認**（鍵の欄の隣の「承認する」から / 作業単位 A-13b-3 のレビュー是正）。
   * 名前解決で決まることと、**保存の前の確認と同じ危険の文**を言う。保存の話（承認せずに保存する）はしない。
   */
  it("ホスト名なら、送るたびに名前解決して判定することと、同じ危険を言う", () => {
    expect(nameApprovalText("本文生成", "http://relay.example:46001")).toEqual({
      title: "本文生成の鍵を、暗号化せずに送ることを承認しますか",
      body: [
        "宛先 http://relay.example:46001 はホスト名です。送るたびに名前解決して判定し、ローカルネットワークの外のアドレスに解決したら、承認が無ければ鍵を送りません。",
        "承認すると、鍵は暗号化されずに送られます。経路の途中で記録されれば、その記録は残り、鍵が悪用されることがあります。",
      ],
    });
  });
});

describe("宛先を変えたときの鍵の確認", () => {
  it("どの面の鍵か（末尾4文字）と、変える前と後の宛先を言う", () => {
    expect(carryQuestionText("本文生成", "k3Qz", "http://127.0.0.1:46001", "http://192.0.2.7:46001")).toEqual({
      title: "本文生成の鍵を、新しい宛先へも送りますか",
      body: [
        "この面には鍵が書いてあります（…k3Qz）。宛先を http://127.0.0.1:46001 から http://192.0.2.7:46001 に変えました。",
        "鍵は宛先に付くものなので、新しい宛先へ送ってよいかを確かめます。鍵を消すと、この面は鍵を送らなくなります。",
      ],
    });
  });

  it("末尾4文字が無い（短い鍵）なら、末尾を出さない", () => {
    expect(carryQuestionText("本文生成", null, "a", "b").body[0]).toBe(
      "この面には鍵が書いてあります。宛先を a から b に変えました。",
    );
  });
});
