/**
 * **`dialogue.ts` が `src/parse/mod.rs` の写しとして成立しているか。**
 *
 * # 権威は Rust 側にある
 *
 * 読んでいる `tests/fixtures/dialogue_corpus.tsv` は
 * **`parse::parse_response` の出力そのもの**で、`tests/dialogue_corpus.rs` が
 * 「フィクスチャが parse の出力と一致すること」を別に見張っている。
 *
 * つまり赤くなる場所が3つに割れている:
 *
 * | 何が起きた | 赤くなるのは | 直すのは |
 * |---|---|---|
 * | `src/parse` の**判定**を変えた | **Rust 側**（作り直せ、と言う） | 作り直したあと `dialogue.ts` |
 * | `src/parse` の **`join_dialogues`** を変えた | **Rust 側 → 作り直すと、こちら** | **`asRust`**（`dialogue.ts` ではない） |
 * | `dialogue.ts` が Rust に追随していない | **こちら** | `dialogue.ts` |
 *
 * **ADR-0031 §6 が「写しなので黙って古びる」と書いた、その見張り番がこれである。**
 *
 * # ⚠ `asRust` は「構造の組み替えだけ」ではない
 *
 * **2026-08-26 に訂正**（D-0b のレビュー）。ここには当初
 * 「`asRust` は構造の組み替えだけでよく、整形をもう一度書き写す必要が無い」と書いたが、
 * **事実ではない。** 次の3行は `parse::join_dialogues`（＝この doc 自身が
 * 「保存のための整形」に分類したもの）**の写しである:**
 *
 * | `asRust` の行 | 写している相手 |
 * |---|---|
 * | `.map((segment) => segment.text.slice(1, -1))` | `「」` を落とすこと |
 * | `.filter((text) => text !== "")` | 空の台詞を数えないこと |
 * | `dialogues.join("\n")` | **1行1台詞**という保存側の不変条件 |
 *
 * **実測**: フィクスチャの台詞欄の区切りだけを「改行 → 空白」に変える
 * （＝ Rust が `join_dialogues` の区切りを変えて作り直した状態）と、
 * **`dialogue.ts` を1文字も触っていないのに、ここが 49 件で赤くなる。**
 *
 * **だから上の表の2行目がある。** そのときに直すのは **`asRust`** であって、
 * `dialogue.ts` ではない。**`dialogue.ts` は保存の形を知らなくてよい**
 * ——画面は保存しないので、`ParsedResponse` の詰め方に追随する義務が無い。
 *
 * **なぜ残すか**: `ParsedResponse` は台詞を**生の区間として公開していない**
 * （`dialogue: Option<String>` の1本に詰めたものしか無い）ので、
 * **どんな欄の形にしても `slice(1, -1)` と空の除去は残る。**
 * 消せるのは `join` の1つだけで、**欄の形を変える対価に見合わない**
 * （司令塔の採否 / 2026-08-26）。
 *
 * # ⚠ **`asRust` を上記以外の理由で編集しないこと**
 *
 * ここを緩めることは、**見張り番の意味を変えること**である。
 * 突き合わせが通らないから `asRust` を寄せる、という直し方をすると、
 * **`dialogue.ts` が古びたことを二度と言わなくなる。**
 * **必要だと思ったら、直す前に司令塔の採否を仰ぐこと。**
 *
 * # 字母に空白と行区切りが無いこと（**ここは変えない**）
 *
 * `fold_line_separators` / `trim_blank` / `trim_blank_end` / CRLF の隣接判定は、
 * **空白と行区切りを含まない単一行の入力の上では全部恒等写像**である
 * （D-0b のレビューで裏取り済み）。だから `asRust` にそれらの写しが無くて済んでいる。
 * **字母に空白を足すと、その瞬間に4つとも写す羽目になる。**
 *
 * # 承知している差
 *
 * **空の `「」`** を `dialogue.ts` は台詞の区間として返すが、Rust は台詞に数えない。
 * `asRust` が空の台詞を落としているのはそのため（`dialogue.ts` の冒頭に既知の差として
 * 書いてあるもの）。**区間そのものが消えていないことは、下の「繋ぐと生出力に戻る」が見ている。**
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  reasonLabel,
  split,
  type FallbackReason,
} from "../src/lib/dialogue";

/** **`tests/dialogue_corpus.rs` の `fixture_path` と同じ場所。** */
const FIXTURE = fileURLToPath(
  new URL("../../tests/fixtures/dialogue_corpus.tsv", import.meta.url),
);

/** 見出しに書いてある件数（`# 件数: 22811`）。**読み落としの検出に使う。** */
const COUNT_PATTERN = /^# 件数: (\d+)$/;

/**
 * 件数の下限。**`tests/dialogue_corpus.rs` の `MIN_INPUTS` と同じ値をここにも置く。**
 *
 * **写しに見えるが、そうではない。** あちらは「これから書くもの」を、
 * こちらは**置いてあるファイルの行数**を数えている。
 * **作り直しを忘れた回はこちらだけが赤くなる。**
 *
 * **⚠ 実測**（D-0b のレビュー）: `MAX_LEN_MAIN` を 7 → 4 に落として作り直すと
 * **22,811 → 1,403 件になり、Rust 5本・TS 13本が全部緑のまま通った。**
 */
const MIN_ROWS = 20_000;

/** 最も長い入力の下限（**コードポイント数**）。件数だけでは長さの縮小を捕まえられない。 */
const MIN_LONGEST_INPUT_CODE_POINTS = 7;

/** フィクスチャの1行。 */
interface Row {
  input: string;
  /** `S` / `N` / `R` / `E` / `U` / `C`。 */
  code: string;
  /** Rust の `ParsedResponse.dialogue`（`None` は空文字列）。 */
  dialogue: string;
  /** Rust の `ParsedResponse.narration`（`None` は空文字列）。 */
  narration: string;
}

/** `dialogue.ts` の結果を、Rust の `ParsedResponse` と同じ形に組み替えたもの。 */
type RustShaped = Pick<Row, "code" | "dialogue" | "narration">;

/** 欄を読む。**書いた側（Rust の `encode`）と対になっている。** */
function decode(field: string): string {
  return field.replaceAll("\\n", "\n");
}

/** フィクスチャを読む。**行末の CR は落とす**（`core.autocrlf=true` のリポジトリ）。 */
function readCorpus(): { rows: Row[]; declaredCount: number } {
  const text = readFileSync(FIXTURE, "utf8");
  const rows: Row[] = [];
  let declaredCount: number | null = null;

  for (const raw of text.split("\n")) {
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (line.startsWith("#")) {
      const found = COUNT_PATTERN.exec(line);
      if (found?.[1] !== undefined) {
        declaredCount = Number.parseInt(found[1], 10);
      }
      continue;
    }
    if (line === "") {
      // 末尾の改行だけ。**見出しと本文の間には空行を置いていない。**
      continue;
    }
    const fields = line.split("\t");
    if (fields.length !== 4) {
      throw new Error(`フィクスチャの欄が4つではありません: ${JSON.stringify(line)}`);
    }
    const [input, code, dialogue, narration] = fields as [
      string,
      string,
      string,
      string,
    ];
    rows.push({
      input: decode(input),
      code,
      dialogue: decode(dialogue),
      narration: decode(narration),
    });
  }

  if (declaredCount === null) {
    throw new Error("フィクスチャの見出しに件数がありません");
  }
  return { rows, declaredCount };
}

/**
 * `split()` の結果を Rust の `ParsedResponse` と同じ形に組み替える。
 *
 * # ⚠ **ここは `join_dialogues` の写しを3つ含んでいる**
 *
 * **どれがどれかと、なぜ残すかは上のモジュール doc にある。**
 * **編集は見張り番の意味を変える。司令塔の採否を仰ぐこと。**
 *
 * 空白の始末や行の畳み込みを書き足したくなったら、
 * それは**フィクスチャの字母に空白が混ざった**ということなので、
 * `tests/dialogue_corpus.rs` の側を直すこと。
 */
function asRust(raw: string): RustShaped {
  const result = split(raw);

  if (result.kind === "fallback") {
    // 崩れたら生出力を丸ごと地の文（ADR-0007）。Rust の `fallback` と同じ。
    return {
      code: result.reason === "unclosed" ? "U" : "C",
      dialogue: "",
      narration: raw,
    };
  }

  const dialogues = result.segments
    .filter((segment) => segment.kind === "dialogue")
    // ⚠ 写し 1/3（`join_dialogues`）: Rust の `dialogue` は `「` `」` を含まない。
    .map((segment) => segment.text.slice(1, -1))
    // ⚠ 写し 2/3（`join_dialogues`）: 空の `「」` は台詞に数えない。
    .filter((text) => text !== "");
  const narration = result.segments
    .filter((segment) => segment.kind === "narration")
    .map((segment) => segment.text)
    .join("");

  let code: string;
  if (dialogues.length > 0) {
    code = result.nested ? "N" : "S";
  } else if (narration !== "") {
    code = "R";
  } else {
    code = "E";
  }

  // ⚠ 写し 3/3（`join_dialogues`）: **1行1台詞**という保存側の不変条件。
  return { code, dialogue: dialogues.join("\n"), narration };
}

const { rows, declaredCount } = readCorpus();

describe("dialogue.ts は src/parse の写しである", () => {
  it("フィクスチャを見出しの件数どおり読めている", () => {
    // **読み落とすと、下の突き合わせが「少ない件数で全部緑」になる。**
    expect(rows.length).toBe(declaredCount);
    expect(rows.length).toBeGreaterThan(0);
  });

  it("6つの判定が全部フィクスチャに出ている", () => {
    // 入れ子・閉じない・対応しない `」`・空・地の文だけ・台詞だけ。
    // **どれかが 0 件なら、その分岐は一度も通っていない。**
    const seen = new Set(rows.map((row) => row.code));
    expect([...seen].sort()).toEqual(["C", "E", "N", "R", "S", "U"]);
  });

  it("全ての入力で Rust と同じ判定・同じ台詞・同じ地の文になる", () => {
    const mismatches: string[] = [];

    for (const row of rows) {
      const mine = asRust(row.input);
      if (
        mine.code !== row.code ||
        mine.dialogue !== row.dialogue ||
        mine.narration !== row.narration
      ) {
        mismatches.push(
          `${JSON.stringify(row.input)}: ` +
            `rust=${JSON.stringify([row.code, row.dialogue, row.narration])} ` +
            `ts=${JSON.stringify([mine.code, mine.dialogue, mine.narration])}`,
        );
      }
    }

    expect(
      mismatches.length,
      `${mismatches.length} / ${rows.length} 件が Rust と食い違いました:\n` +
        mismatches.slice(0, 10).join("\n"),
    ).toBe(0);
  });

  it("区間を順に繋ぐと生出力に戻る（画面は1文字も変えない）", () => {
    // **`dialogue.ts` 自身の不変条件。** 上の突き合わせは Rust の形へ
    // 組み替えてから比べるので、**空の `「」` の区間のように
    // 組み替えで消えるもの**はここでしか見ていない。
    const broken: string[] = [];

    for (const row of rows) {
      const result = split(row.input);
      const rebuilt =
        result.kind === "fallback"
          ? result.text
          : result.segments.map((segment) => segment.text).join("");
      if (rebuilt !== row.input) {
        broken.push(`${JSON.stringify(row.input)} -> ${JSON.stringify(rebuilt)}`);
      }
    }

    expect(
      broken.length,
      `${broken.length} 件で生出力に戻りませんでした:\n${broken.slice(0, 10).join("\n")}`,
    ).toBe(0);
  });

  it("BMP の外の文字を含む入力を実際に通している", () => {
    // **Rust は コードポイント、TypeScript は UTF-16 の符号単位**で走査する。
    // 通る入力が無ければ「サロゲートペアを割らない」はただの主張になる。
    const astral = rows.filter((row) =>
      [...row.input].some((character) => (character.codePointAt(0) ?? 0) > 0xffff),
    );
    expect(astral.length).toBeGreaterThan(0);
  });

  it("フィクスチャが痩せていない（件数と、最も長い入力の長さ）", () => {
    // **`MIN_ROWS` の doc に、これが無いと何が通り抜けるかの実測がある。**
    // 6判定も astral も残るので、他のどの見張り番も「痩せた」と言わない。
    const longest = Math.max(...rows.map((row) => [...row.input].length));

    expect(rows.length).toBeGreaterThanOrEqual(MIN_ROWS);
    expect(longest).toBeGreaterThanOrEqual(MIN_LONGEST_INPUT_CODE_POINTS);
  });

  it("崩れの2つの理由は、画面に出る文言まで別物になる", () => {
    // **`reasonLabel` を誰も見ていなかった**（D-0b のレビュー）。
    // 2つを同じ文字列にする変異が通ると、**画面で崩れの理由が区別できなくなる。**
    const reasons: FallbackReason[] = ["unclosed", "stray-close"];
    const labels = reasons.map((reason) => reasonLabel(reason));

    expect(labels.every((label) => label !== "")).toBe(true);
    expect(new Set(labels).size).toBe(reasons.length);
  });
});
