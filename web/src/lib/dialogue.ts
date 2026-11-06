/**
 * **応答を台詞と地の文に割る**（表示のためだけ / ADR-0007・ADR-0014）。
 *
 * # ⚠ **これは `src/parse/mod.rs` の規則を手で写したものである**
 *
 * **生成していない。** 写しの性質（Rust 側を変えると黙って古びる）は
 * API の型と同じで、ADR-0031 §6 が既に持っている。**ここで書き足さない。**
 *
 * **写したのは「括弧の対応だけを見る」という判定そのもの**で、
 * 保存のための整形（改行の畳み込み・空行の始末・前後の空白落とし）は写していない。
 * **画面は保存しないから**であり、下の不変条件のほうが表示には効く。
 *
 * # 不変条件 —— **画面は、強調の印以外を1文字も変えない**
 *
 * `split(raw)` が返す区間の本文を順に繋ぐと、**必ず `stripEmphasis(raw).text` に戻る。**
 * 色が変わるだけで、**落とすのは Markdown の強調の印だけ**、足すものは無い。
 *
 * **⚠ 2026-09-17（作業単位 B-46）に、この不変条件の右辺が `raw` から変わった。**
 * 実機で**モデルが本文に強調を書く**ことが分かり（`C-18`）、
 * **サーバ側は保存する本文から落としているのに、画面には出たまま**だったため。
 * **落とす規則は `./emphasis` が持つ**（`src/parse/extraction.rs` の写し）。
 *
 * **崩れた回は落とさない**（`fallback` の `text` は `raw` そのもの）。
 * Rust 側も同じで、理由は下の「崩れたら救わない」と同じ——
 * **崩れた出力を整形すると、何を見て崩れと判断したかが分からなくなる。**
 *
 * **`「` `」` も台詞の側に残す**（Rust 側は `message.dialogue` から落とすが、
 * あれは1列に詰めるための都合で、日本語の本文としては括弧ごと見えているのが自然）。
 *
 * ## 既知の差 —— 空の `「」`
 *
 * **空の `「」` を、ここは台詞の区間として着色するが、Rust は台詞にも地の文にも残さない**
 * （`ParseOutcome::Empty` / `NarrationOnly`）。**上の不変条件を選んだ以上の必然**で、
 * **`「」` を地の文の色で出すのも同じくらい正しくない。** 直さずに記録だけしておく。
 *
 * # 崩れたら救わない
 *
 * **ADR-0007 の要は「崩れたら生出力を丸ごと地の文にする」で、部分的に拾わない。**
 * ここでも同じにする。**画面側で拾い直すと、サーバが保存した形と画面が食い違う。**
 *
 * 崩れは2つだけ（`Unclosed` / `StrayClose`）。**入れ子は崩れではない**（ADR-0014）。
 */

import { stripEmphasis } from "./emphasis";

/** 台詞の開き括弧（U+300C）。 */
const OPEN = "「";

/** 台詞の閉じ括弧（U+300D）。 */
const CLOSE = "」";

/** 割った結果の1区間。 */
export interface Segment {
  kind: "dialogue" | "narration";
  text: string;
}

/** 崩れの種類（`parse::FallbackReason` の写し）。 */
export type FallbackReason = "unclosed" | "stray-close";

/** 割った結果。 */
export type SplitResult =
  | {
      kind: "split";
      segments: Segment[];
      /** 入れ子の `「」` があった（崩れではない / ADR-0014）。 */
      nested: boolean;
    }
  | {
      kind: "fallback";
      reason: FallbackReason;
      /** **生出力そのもの。** 丸ごと地の文として出す。 */
      text: string;
    };

/** 崩れの説明（画面に出す文言）。**内部の事情は書かない。** */
export function reasonLabel(reason: FallbackReason): string {
  switch (reason) {
    case "unclosed":
      return "閉じられていない「";
    case "stray-close":
      return "対応する「の無い」";
  }
}

/**
 * 生出力を区間に割る。**崩れたら丸ごと地の文（`fallback`）。**
 *
 * 走査は `「` と `」` の対応だけを見る。長さも中身も見ない
 * （閾値を入れると、正当な長台詞が理由不明で地の文になる / ADR-0007）。
 *
 * `「` `」` はどちらも BMP なので、UTF-16 の添字で切ってもサロゲートペアを割らない。
 */
export function split(raw: string): SplitResult {
  // **割る前に強調の印を落とす**（作業単位 B-46 / 冒頭の不変条件）。
  // **崩れた枝では使わない** —— 下の `fallback` は `raw` をそのまま返している。
  const body = stripEmphasis(raw).text;
  const segments: Segment[] = [];
  let nested = false;
  let depth = 0;
  /** いま開いている台詞（または直前の台詞の終わり）の開始位置。 */
  let start = 0;

  for (let i = 0; i < body.length; i += 1) {
    const ch = body.charAt(i);
    if (ch === OPEN) {
      if (depth === 0) {
        pushNarration(segments, body.slice(start, i));
        start = i;
      } else {
        // 入れ子。**台詞本文にそのまま残す**（ADR-0014）。
        nested = true;
      }
      depth += 1;
      continue;
    }
    if (ch !== CLOSE) {
      continue;
    }
    if (depth === 0) {
      // 対応する `「` が無い。**区間の左端が決まらない。**
      return { kind: "fallback", reason: "stray-close", text: raw };
    }
    depth -= 1;
    if (depth === 0) {
      segments.push({ kind: "dialogue", text: body.slice(start, i + 1) });
      start = i + 1;
    }
  }

  if (depth > 0) {
    // 閉じないまま終わった。**区間の右端が決まらない。**
    return { kind: "fallback", reason: "unclosed", text: raw };
  }

  pushNarration(segments, body.slice(start));
  return { kind: "split", segments, nested };
}

/** 空でなければ地の文として積む（空区間を積むと余計な要素が出る）。 */
function pushNarration(segments: Segment[], text: string): void {
  if (text !== "") {
    segments.push({ kind: "narration", text });
  }
}
