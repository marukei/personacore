/**
 * **作成・編集の要求を組み立てる部分**（作業単位 D-2 / `src/lib/characterEdit.ts`）。
 *
 * # ここが見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **変えていない欄はキーごと落ちる** | `PATCH` は「キーの有無」で「変えない」を表す（`admin.rs` の `UpdateCharacterRequest`） |
 * | **形式指示だけは3値** | 「変えない」（キー無し）と「消す」（`null`）を同じ表現にしない |
 * | **上限の写しが、写し元と一致している** | 下の `the_limits_are_copied_from_rust` |
 * | **数え方が符号位置である** | Rust は `chars().count()`。JS の `.length` は UTF-16 の個数で、絵文字が2つに化ける |
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import type { CharacterView } from "../src/lib/api";
import {
  LIMITS,
  buildCreate,
  buildUpdate,
  countChars,
  emptyDraft,
  draftOf,
  type CharacterDraft,
} from "../src/lib/characterEdit";

/** 編集の対象。**全部の欄をわざと別の値にしてある**（取り違えが見えるように）。 */
const ORIGINAL: CharacterView = {
  id: 7,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** `ORIGINAL` をそのまま写した下書き（＝1つも変えていない状態）。 */
function untouched(): CharacterDraft {
  return draftOf(ORIGINAL);
}

describe("上限は Rust 側の写しである", () => {
  /**
   * **`src/domain/character.rs` を実際に読んで突き合わせる。**
   *
   * **ADR-0031 §6 は「型は手で写す。気付く仕掛けは痛みが出た時点で入れる」**と書いている。
   * 上限は**型ではなく値**で、**ずれると画面が黙って嘘の数を出す**
   * （「600 文字まで」と書いてあるのにサーバが 400 を返す、あるいはその逆）ので、
   * ここだけは写し元を読む。
   *
   * **⚠ `Handle` と `DisplayName` はどちらも 50 なので、この2つの取り違えは
   * この検査では捕まらない**（写し元で同値なので、突き合わせでは区別できない）。
   */
  it("`src/domain/character.rs` の `MAX_CHARS` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/domain/character.rs", import.meta.url)),
      "utf8",
    );

    expect(maxCharsOf(source, "Handle")).toBe(LIMITS.handle);
    expect(maxCharsOf(source, "DisplayName")).toBe(LIMITS.display_name);
    expect(maxCharsOf(source, "Persona")).toBe(LIMITS.base_persona);
    expect(maxCharsOf(source, "OutputFormat")).toBe(
      LIMITS.output_format_instruction,
    );
  });

  /**
   * **写し元を読む側が壊れていないこと。**
   *
   * `maxCharsOf` が何も見つけられなくても `undefined === undefined` で
   * 緑になる形を避ける（`15b` の「そもそも探せていないから緑」）。
   * **`impl` の綴りを1文字変えた入力**を通して、実際に落ちることを見る。
   */
  it("写し元から読めなければ落ちる（探せていないことを緑にしない）", () => {
    const planted = "impl Persona {\n    pub const MAX_CHARS: usize = 601;\n}";
    expect(maxCharsOf(planted, "Persona")).toBe(601);
    expect(() => maxCharsOf(planted, "Handle")).toThrow();
  });
});

/**
 * その型の `impl` ブロックから `MAX_CHARS` を1つだけ取り出す。
 *
 * **次の `impl` の手前で切る。** 切らないと、`MAX_CHARS` を持たない型を
 * 尋ねたときに**次の型の値を返してしまう**（探せていないのに緑になる）。
 */
function maxCharsOf(source: string, typeName: string): number {
  const start = source.indexOf(`impl ${typeName} {`);
  if (start < 0) {
    throw new Error(`impl ${typeName} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("\nimpl ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = /pub const MAX_CHARS: usize = (\d+);/.exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`impl ${typeName} に MAX_CHARS がありません`);
  }
  return Number.parseInt(found[1], 10);
}

describe("文字数の数え方", () => {
  it("符号位置で数える（UTF-16 の個数ではない）", () => {
    // Rust の `chars().count()` は 1。JS の `"🐈".length` は 2。
    expect("🐈".length).toBe(2);
    expect(countChars("🐈")).toBe(1);
    expect(countChars("𠮟る")).toBe(2);
  });

  it("前後の空白は数えない（サーバも正規化してから数える）", () => {
    expect(countChars("　 あ \n")).toBe(1);
    // 中間の空白は残る（`DisplayName::parse` は「中間の空白は保持する」）。
    expect(countChars("あ い")).toBe(3);
  });

  /**
   * **数え漏らす向きにだけ外れる**（＝早まって弾かない）。
   *
   * JS の `trim()` が落とす集合は `domain::text::TRIMMABLE` の**上位集合**
   * （U+2028 / U+2029 / U+FEFF を余分に落とす）。
   * **`TRIMMABLE` をここへ写さない**ための選択で、外れる向きは
   * 「こちらが少なく数える」＝**サーバだけが弾く**側になる。
   * その回はサーバのメッセージが画面に出る。
   */
  it("サーバより多く数えることはない", () => {
    const padded = `${"あ".repeat(LIMITS.base_persona)}\u{2028}`;
    // サーバ（`Persona::parse`）は U+2028 を落とさないので 601 と数えて弾く。
    expect([...padded].length).toBe(LIMITS.base_persona + 1);
    // こちらは 600。**弾かずに送り、サーバの理由を見せる。**
    expect(countChars(padded)).toBe(LIMITS.base_persona);
  });
});

describe("作成の要求", () => {
  it("識別子とペルソナを載せる", () => {
    const draft: CharacterDraft = {
      ...emptyDraft(),
      handle: "rin",
      display_name: "凛",
      base_persona: "早口で話す。",
    };
    const built = buildCreate(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.kind).toBe("create");
    expect(built.body).toEqual({
      handle: "rin",
      display_name: "凛",
      base_persona: "早口で話す。",
    });
  });

  it("空の欄はキーごと落ちる（サーバの既定に任せる）", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      base_persona: "早口で話す。",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    // **表示名を省くとサーバが `handle` で埋める**（`admin.rs` の `display_name_of`）。
    expect("display_name" in built.body).toBe(false);
    // **尺度を省くとサーバの既定になる**（`StatScale::DEFAULT`。値をこちらに写さない）。
    expect("stat_scale" in built.body).toBe(false);
    expect("output_format_instruction" in built.body).toBe(false);
  });

  /**
   * **識別子を画面側で直さない**（ADR-0017）。
   *
   * `Handle::parse` は `" rin "` を `"rin"` に直さず**拒否する** ——
   * 「前後に空白の付いた識別子」を**表現できなくする**のが要点。
   * **こちらで黙って `trim` すると、貼り付け事故が
   * 「気付かないまま別の綴りで作られた」形**になり、
   * 識別子は不変（ADR-0016 §1）なので後から直せない。
   */
  it("識別子を送る前に直さない（サーバに拒否させる）", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: " rin ",
      base_persona: "早口で話す。",
    });
    expect(built.ok && built.kind).toBe("create");
    if (!built.ok || built.kind !== "create") {
      return;
    }
    expect(built.body.handle).toBe(" rin ");
  });

  /**
   * **表示名とペルソナは逆**——**サーバが正規化する**（`DisplayName::parse` /
   * `Persona::parse` は `trim_blank` を通す）ので、**生のまま送る。**
   * こちらで直すと、**2つの正規化が一致し続けることに依存する**ことになる。
   */
  it("表示名とペルソナは生のまま送る（サーバが正規化する）", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      display_name: " 凛 ",
      base_persona: " 早口で話す。\n",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body.display_name).toBe(" 凛 ");
    expect(built.body.base_persona).toBe(" 早口で話す。\n");
  });

  it("尺度は整数として載る（文字列で送らない）", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      base_persona: "早口で話す。",
      stat_scale: "7",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body.stat_scale).toBe(7);
  });

  it("整数でない尺度は、送らずに理由を返す", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      base_persona: "早口で話す。",
      stat_scale: "5.5",
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("尺度");
  });
});

describe("編集の要求は差分だけを載せる", () => {
  it("1つも変えていなければ、キーが1つも無い", () => {
    const built = buildUpdate(ORIGINAL, untouched());
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.kind).toBe("update");
    expect(Object.keys(built.body)).toEqual([]);
  });

  it("変えた欄だけが載る", () => {
    const draft = { ...untouched(), display_name: "藍" };
    const built = buildUpdate(ORIGINAL, draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body).toEqual({ display_name: "藍" });
    // **ペルソナは触っていないのでキーごと無い。**
    // 送ると「変えない」ではなく「同じ値で上書きする」になり、
    // 別の窓から先に直された値を黙って踏み潰す。
    expect("base_persona" in built.body).toBe(false);
  });

  it("形式指示を空にすると `null` が載る（「変えない」と別の表現）", () => {
    const built = buildUpdate(ORIGINAL, {
      ...untouched(),
      output_format_instruction: "",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect("output_format_instruction" in built.body).toBe(true);
    expect(built.body.output_format_instruction).toBeNull();
  });

  it("元から未設定の形式指示を空のままにしても、消す指示は載らない", () => {
    const never: CharacterView = {
      ...ORIGINAL,
      output_format_instruction: null,
    };
    const built = buildUpdate(never, draftOf(never));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(Object.keys(built.body)).toEqual([]);
  });

  it("尺度を変えたときだけ載る", () => {
    const same = buildUpdate(ORIGINAL, {
      ...untouched(),
      stat_scale: String(ORIGINAL.stat_scale),
    });
    expect(same.ok && "stat_scale" in same.body).toBe(false);

    const changed = buildUpdate(ORIGINAL, { ...untouched(), stat_scale: "9" });
    expect(changed.ok).toBe(true);
    if (!changed.ok) {
      return;
    }
    expect(changed.body.stat_scale).toBe(9);
  });

  /**
   * **識別子は不変**（ADR-0016 §1）。要求に混ぜると `deny_unknown_fields` で
   * **1列も書かれない 400** になる（`admin.rs` の `UpdateCharacterRequest` の doc）。
   */
  it("識別子は絶対に載らない", () => {
    const built = buildUpdate(ORIGINAL, {
      ...untouched(),
      handle: "another",
      display_name: "藍",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect("handle" in built.body).toBe(false);
    expect(Object.keys(built.body)).toEqual(["display_name"]);
  });
});

describe("上限を超えた欄は、送る前に止める", () => {
  it("上限ちょうどは通り、1文字超えると止まる", () => {
    const atLimit = "あ".repeat(LIMITS.base_persona);
    const overLimit = "あ".repeat(LIMITS.base_persona + 1);

    const ok = buildUpdate(ORIGINAL, {
      ...untouched(),
      base_persona: atLimit,
    });
    expect(ok.ok).toBe(true);

    const stopped = buildUpdate(ORIGINAL, {
      ...untouched(),
      base_persona: overLimit,
    });
    expect(stopped.ok).toBe(false);
    if (stopped.ok) {
      return;
    }
    // **どの欄かを名乗る。** サーバのメッセージも欄を名乗るが、
    // ここで止めた回はサーバへ届かないので、名乗るのはこちらの仕事。
    expect(stopped.message).toContain("ペルソナ");
  });

  it("作成でも同じ上限で止まる（`POST` にだけ穴を開けない）", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "a".repeat(LIMITS.handle + 1),
      base_persona: "早口で話す。",
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("識別子");
  });

  it("超えている欄が2つなら、2つとも名乗る", () => {
    const built = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      display_name: "あ".repeat(LIMITS.display_name + 1),
      base_persona: "あ".repeat(LIMITS.base_persona + 1),
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("表示名");
    expect(built.message).toContain("ペルソナ");
  });
});

describe("上限の帰属 —— どの欄の上限か", () => {
  /**
   * **止まるかどうかは正しいのに、名乗る上限だけが他人のものになりうる。**
   *
   * `overLimitMessage` の `LIMITS[field]` を `LIMITS.base_persona` に決め打つ変異は、
   * **判定を1文字も変えない**（判定は `overLimitFields` の側）ので、
   * **「識別子（上限 600 文字）が長すぎます」という嘘だけが残る。**
   *
   * **自分の上限が出ることと、他の欄の上限が出ないことを両方見る。**
   * **⚠ `LIMITS` から期待値を作ってよい** —— 比べているのが**欄どうし**なので、
   * 定数を丸ごと動かす変異は別の見張り番（写し検査）が受け持つ。
   */
  it("メッセージは、その欄の上限を出す", () => {
    const handleOver = buildCreate({
      ...emptyDraft(),
      handle: "a".repeat(LIMITS.handle + 1),
      base_persona: "早口で話す。",
    });
    expect(handleOver.ok).toBe(false);
    if (!handleOver.ok) {
      expect(handleOver.message).toContain(String(LIMITS.handle));
      expect(handleOver.message).not.toContain(String(LIMITS.base_persona));
    }

    const personaOver = buildUpdate(ORIGINAL, {
      ...untouched(),
      base_persona: "あ".repeat(LIMITS.base_persona + 1),
    });
    expect(personaOver.ok).toBe(false);
    if (!personaOver.ok) {
      expect(personaOver.message).toContain(String(LIMITS.base_persona));
      expect(personaOver.message).not.toContain(
        String(LIMITS.output_format_instruction),
      );
    }

    const formatOver = buildUpdate(ORIGINAL, {
      ...untouched(),
      output_format_instruction: "あ".repeat(
        LIMITS.output_format_instruction + 1,
      ),
    });
    expect(formatOver.ok).toBe(false);
    if (!formatOver.ok) {
      expect(formatOver.message).toContain(
        String(LIMITS.output_format_instruction),
      );
      expect(formatOver.message).not.toContain(String(LIMITS.base_persona));
    }
  });

  /**
   * **出力形式指示も止まる。**
   *
   * 4欄ぶんの上限を持っているのに、**振る舞いで押さえていたのは3欄だけ**だった
   * （`CREATE_FIELDS` / `UPDATE_FIELDS` から `"output_format_instruction"` を
   * 落とす変異が 56 全緑 / D-2 のレビュー）。**作成と更新の両方で見る。**
   */
  it("出力形式指示は、作成でも更新でも止まる", () => {
    const tooLong = "あ".repeat(LIMITS.output_format_instruction + 1);

    const created = buildCreate({
      ...emptyDraft(),
      handle: "rin",
      base_persona: "早口で話す。",
      output_format_instruction: tooLong,
    });
    expect(created.ok).toBe(false);
    if (!created.ok) {
      expect(created.message).toContain("出力形式指示");
    }

    const updated = buildUpdate(ORIGINAL, {
      ...untouched(),
      output_format_instruction: tooLong,
    });
    expect(updated.ok).toBe(false);
    if (!updated.ok) {
      expect(updated.message).toContain("出力形式指示");
    }
  });

  /**
   * **編集では識別子を数えない**（`UPDATE_FIELDS` に入れない）。
   *
   * **入っていないのは値ではなく「状態」だった** ——
   * **上限を超えた識別子で保存されている行**（＝上限を下げた後の DB）を
   * 作るフィクスチャがどこにも無かった（D-2 のレビュー）。
   *
   * 入れてしまうと、**その行だけ編集そのものができなくなる。**
   * 識別子は不変（ADR-0016 §1）なので**直す入口が塞がって二度と直せない。**
   */
  it("保存済みの識別子が上限を超えていても、その行を編集できる", () => {
    const overLimitHandle: CharacterView = {
      ...ORIGINAL,
      handle: "a".repeat(LIMITS.handle + 1),
    };
    const built = buildUpdate(overLimitHandle, {
      ...draftOf(overLimitHandle),
      display_name: "藍",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body).toEqual({ display_name: "藍" });
  });
});

describe("写し検査の切り出しそのもの", () => {
  /**
   * **`impl` ブロックの切り出しを外すと、写し検査が静かに無効になる。**
   *
   * 単独では観測できない（`const block = rest;` にする変異は 56 全緑）が、
   * **写し元のリネームと組にすると等価でなくなる**（CLAUDE.md の第5分類）——
   * 切り出しが無いと **`impl Handle {` の後ろから最初に見つかる `MAX_CHARS` は
   * `DisplayName` の 50** で、**`LIMITS.handle` も 50 なので一致してしまい、
   * **`Handle` の上限が消えたことに誰も気づけない。**
   *
   * **実ファイルではなく、植えたソースで見る**（`15b` の「自己成就する対照」）。
   */
  it("次の `impl` の値を拾わない", () => {
    const two =
      "impl Handle {\n}\nimpl DisplayName {\n    pub const MAX_CHARS: usize = 50;\n}";
    // **`Handle` には無い。** 次の型の 50 を返してはいけない。
    expect(() => maxCharsOf(two, "Handle")).toThrow();
    // 対照: **同じ入力で `DisplayName` は読める**
    // （「何を渡しても投げる」だけの実装と区別する）。
    expect(maxCharsOf(two, "DisplayName")).toBe(50);
  });
});
