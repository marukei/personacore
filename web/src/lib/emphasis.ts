/**
 * **本文から Markdown の強調の印を落とす**（表示のためだけ / 作業単位 B-46）。
 *
 * # ⚠ **これは `src/parse/extraction.rs` の `strip_emphasis` を手で写したものである**
 *
 * **生成していない。** 写しの性質（Rust 側を変えると黙って古びる）は
 * ADR-0031 §6 が既に持っている。**ここで書き足さない。**
 *
 * **写し元は3つある。** 規則そのもの（`strip_emphasis` と `EMPHASIS_MARKS`）に加えて、
 * **`src/domain/text.rs` の `TRIMMABLE`（8点）と `LINE_BREAKS`（7点）**を持ってきている。
 * 規則が「行の中で」「空白に挟まれて」で決まる以上、**この2つの表が要る。**
 *
 * # なぜ落とすのか
 *
 * 実機の観測（`C-18` / 公式 Qwen3.6-35B-A3B）で、**モデルは本文に強調を書く。**
 * サーバは保存する `dialogue` / `narration` から落とすが、
 * **画面へ流れてくるのは上流の生出力そのもの**（SSE も非ストリーミングの応答も）なので、
 * **落とすのは画面の側の仕事**である。
 *
 * # 見張り番は Rust 側と共有の表
 *
 * **`tests/fixtures/emphasis_corpus.tsv`**（リポジトリのルート側）を、
 * `tests/emphasis_corpus.rs`（Rust）と `web/tests/emphasis-corpus.test.ts`（こちら）の
 * **両方が読む。** 表は**手書き**で、実装から作り直す仕掛けを持たない。
 *
 * **表は、上の3つの写しを1行ずつ突いている** ——
 * 行区切り7点と CRLF、`TRIMMABLE` のうち行区切りでない4点、
 * そして**JavaScript だけが空白と認める2点**（U+2009 / U+FEFF）。
 * **`trim()` や正規表現の空白類に寄りかかった実装は、最後の2点で落ちる。**
 */

/**
 * 落とす印。**`[印の文字, 2文字重ねる形か]`**（`EMPHASIS_MARKS` の写し）。
 *
 * **2文字の形だけを対象にする。** 1文字の形（`*` 1つ / `_` 1つ）を入れると
 * 算術（`2*3*4`）が強調の条件を満たしてしまう。**観測されたのは2文字の形だけ。**
 */
const EMPHASIS_MARKS: readonly (readonly [string, boolean])[] = [
  ["*", true],
  ["_", true],
  ["`", false],
];

/**
 * 前後の空白（`src/domain/text.rs` の `TRIMMABLE` の写し / **8点**）。
 *
 * **`trim()` や正規表現の空白類で代用しないこと。** JavaScript はここに無い文字
 * （U+2009 などの Unicode 空白と U+FEFF）も空白と認めるので、
 * **Rust が残す印を落としてしまう。**
 *
 * **`thinking.ts` も読む**（作業単位 B-61。思考を剥がした境目の空行の判定）ので `export` してある。
 * 写しを増やさないための `export` で、写し元は今も `src/domain/text.rs` の1つ。
 */
export const TRIMMABLE: readonly string[] = [
  " ", // U+0020 半角スペース
  "\u0009", // TAB
  "\u000A", // LF
  "\u000B", // VT
  "\u000C", // FF
  "\u000D", // CR
  "\u3000", // 全角空白
  "\u00A0", // NBSP
];

/**
 * 行区切り（`src/domain/text.rs` の `LINE_BREAKS` の写し / **7点**）。
 *
 * **`TRIMMABLE` とは別物。** あちらは U+0085 / U+2028 / U+2029 を含まず、
 * こちらは TAB / 全角空白 / NBSP を含まない。
 *
 * **`thinking.ts` も読む**（作業単位 B-61。`TRIMMABLE` と同じ理由で `export` してある）。
 */
export const LINE_BREAKS: readonly string[] = [
  "\u000A", // LF
  "\u000D", // CR
  "\u000B", // VT
  "\u000C", // FF
  "\u0085", // NEL
  "\u2028", // LINE SEPARATOR
  "\u2029", // PARAGRAPH SEPARATOR
];

/** 落とした結果。 */
export interface Stripped {
  /** 落とした後の本文。**行区切りは1文字も変わっていない。** */
  text: string;
  /** 落とした**対**の数。**宙に浮いた印は対ではないので数えない。** */
  pairs: number;
}

/** 行を走査した結果の1片。**添字を作らずに印を扱うための中間形。** */
type Piece =
  | { kind: "text"; text: string }
  | { kind: "mark"; which: number; mark: string; width: number };

/** 印の種類。`[並び順, 2文字重ねる形か]`。印でなければ `null`。 */
function emphasisKind(character: string): readonly [number, boolean] | null {
  const at = EMPHASIS_MARKS.findIndex(([mark]) => mark === character);
  const found = EMPHASIS_MARKS[at];
  return found === undefined ? null : [at, found[1]];
}

/**
 * 行を「印」と「それ以外の文字」に割る。
 *
 * 2文字の形は**同じ文字が2つ続いたときだけ**印になる（`2*3*4` は印ではない）。
 * 3つ続いたときの3つ目は印にならず本文に残る。
 *
 * **コードポイントで回す**ので、サロゲートペアを割らない。
 */
function splitIntoPieces(line: string): Piece[] {
  const characters = [...line];
  const pieces: Piece[] = [];

  for (let at = 0; at < characters.length; at += 1) {
    const character = characters[at] as string;
    const kind = emphasisKind(character);
    if (kind === null) {
      pieces.push({ kind: "text", text: character });
      continue;
    }
    const [which, doubled] = kind;
    if (!doubled) {
      pieces.push({ kind: "mark", which, mark: character, width: 1 });
      continue;
    }
    if (characters[at + 1] === character) {
      // 2文字目を食べる。
      at += 1;
      pieces.push({ kind: "mark", which, mark: character, width: 2 });
      continue;
    }
    pieces.push({ kind: "text", text: character });
  }

  return pieces;
}

/** 印の片側に文字が無いか。**行の端（`undefined`）と空白を同じに扱う。** */
function sideIsEmpty(piece: Piece | undefined): boolean {
  if (piece === undefined) {
    return true;
  }
  // 隣も印なら「文字がある」側に倒す（対の判定に任せる）。
  return piece.kind === "text" && TRIMMABLE.includes(piece.text);
}

/**
 * **宙に浮いた印か。** 条件は2つとも位置だけで決まる。
 *
 * 1. **前後のどちらにも文字が無い**（行の端か空白）。
 * 2. **少なくとも片側が行の端である。**
 *
 * 2 が無いと、**空白で囲まれたべき乗が壊れる**（`2 ** 10` の中身が消える）。
 */
function isFloating(pieces: Piece[], at: number): boolean {
  const before = at === 0 ? undefined : pieces[at - 1];
  const after = pieces[at + 1];
  const touchesTheEnd = before === undefined || after === undefined;
  return touchesTheEnd && sideIsEmpty(before) && sideIsEmpty(after);
}

/**
 * 1行から印を落とす。**落とすものが無ければ `null`。**
 *
 * # 落とすもの
 *
 * * **対になった印。** 同じ種類の印が並んでいたら、**前から順に2つずつ対にする。**
 *   奇数個なら最後の1つが余る。
 * * **行の端で宙に浮いた印**（[`isFloating`]）。囲む中身が無い装飾の残骸。
 *
 * # 落とさないもの
 *
 * * **1文字の形。**
 * * **対になっていない印**（閉じない印）。落とすと本文を推測で書き換えることになる。
 * * **行の途中で空白に挟まれただけの印**（べき乗の記法）。
 */
function stripLine(line: string): Stripped | null {
  const pieces = splitIntoPieces(line);

  // 種類ごとの総数。**対にできるのは偶数個まで。**
  const totals = EMPHASIS_MARKS.map(() => 0);
  for (const piece of pieces) {
    if (piece.kind === "mark") {
      totals[piece.which] = (totals[piece.which] ?? 0) + 1;
    }
  }

  const seen = EMPHASIS_MARKS.map(() => 0);
  let text = "";
  let dropped = 0;
  let pairedMarks = 0;

  for (let at = 0; at < pieces.length; at += 1) {
    const piece = pieces[at] as Piece;
    if (piece.kind === "text") {
      text += piece.text;
      continue;
    }

    const total = totals[piece.which] ?? 0;
    const soFar = seen[piece.which] ?? 0;
    // 前から2つずつ対にする（奇数個なら最後の1つが対から漏れる）。
    const paired = soFar < total - (total % 2);

    if (paired || isFloating(pieces, at)) {
      dropped += 1;
      if (paired) {
        pairedMarks += 1;
      }
    } else {
      text += piece.mark.repeat(piece.width);
    }
    seen[piece.which] = soFar + 1;
  }

  // **印2つで1対。** 宙に浮いた印はここに入っていない。
  return dropped > 0 ? { text, pairs: pairedMarks / 2 } : null;
}

/**
 * 本文から印を落とす。**行ごとに当てる。**
 *
 * **対は行をまたがない**（別々の行に落ちた印は対にならない）。
 * **行区切りは1文字も変えずに戻す** ——
 * 畳むのは表示する側の仕事で、ここで先に畳むと本文が別物になる。
 */
export function stripEmphasis(raw: string): Stripped {
  let text = "";
  let pairs = 0;
  let line = "";

  const flush = (): void => {
    const stripped = stripLine(line);
    if (stripped === null) {
      // **落とすものが無ければ元の行**（`null` はその区別そのもの）。
      text += line;
    } else {
      text += stripped.text;
      pairs += stripped.pairs;
    }
    line = "";
  };

  for (const character of raw) {
    if (LINE_BREAKS.includes(character)) {
      flush();
      text += character;
    } else {
      line += character;
    }
  }
  flush();

  return { text, pairs };
}
