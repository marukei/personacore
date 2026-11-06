/**
 * **`src/server/*.rs` を「線に乗る欄名」として読む走査本体。**
 *
 * # なぜテスト本体から出ているのか
 *
 * **写しの見張り番が2つになったから**（`api-fields.test.ts` / B-18 と
 * `chat-wire.test.ts` / B-20）。**同じ `serde` の解釈を2つ持つと、
 * 片方だけが属性を取りこぼす形で古びる**（CLAUDE.md「同じ発見を2箇所に書かない」）。
 *
 * **B-20 で移したときは、中身を B-18 から1文字も変えなかった。** 置き場所だけが変わった。
 * **F-4b-3 で [`rustFieldsAs`]（向きを名乗って読む）を足した。** 向きを名乗らない
 * [`rustFieldsOf`] の判定は変えていない（両方の向きを持つ型は、今も落とす）。
 * **植えたソースの対照は `api-fields.test.ts` にそのまま残っている**
 * （「Rust 側の走査（植えたソースを通す）」の一群）。
 *
 * `serde` の属性をどう解釈するか（`rename` / `rename_all` / 向き別の形 /
 * `flatten` / `skip` / `skip_serializing_if` / `default` / 要求側の `Option<…>`）は
 * **`api-fields.test.ts` 冒頭の表が権威**。ここに写さない。
 *
 * **ここはテストではない**（`vitest.config.ts` の `include` は `tests/**\/*.test.ts`）。
 */

/** 線に乗る1欄。**省略可否まで持つ**（`?` の有無は wire 上の別の主張）。 */
export interface WireField {
  name: string;
  optional: boolean;
}

/** 突き合わせ用の並び。**順序は見ない**（JSON のキーの順序に意味は無いため）。 */
export function labelsOf(fields: WireField[]): string[] {
  return fields
    .map((field) => (field.optional ? `${field.name}?` : field.name))
    .sort();
}

/**
 * `admin.rs` の構造体1つを、**線に乗る欄名**で読む。
 *
 * `serde` の属性の扱いはこのファイルの冒頭の表。
 * **読めない行は throw する**（黙って飛ばすと、飛ばした欄が
 * 「両側に無い」ことになって緑を通る）。
 */
export function rustFieldsOf(
  source: string,
  structName: string,
  seen: ReadonlySet<string> = new Set(),
): WireField[] {
  return fieldsOf(source, structName, seen, null);
}

/** 線の向き。`serialize` は応答、`deserialize` は依頼。 */
export type WireDirection = "serialize" | "deserialize";

/**
 * **向きを指定して読む**（作業単位 F-4b-3）。
 *
 * **`Serialize` と `Deserialize` の両方を持つ型**（`profile_editor.rs` の `WrittenFace`）は、
 * [`rustFieldsOf`] では向きが決まらず落ちる。**こちらは呼び出し側が向きを名乗る。**
 * **名乗った向きの `derive` が無ければ落とす**（応答しか無い型を依頼として読まない）。
 *
 * **`rustFieldsOf` の判定はこれを足しても1文字も変わらない**（向きを名乗らない呼び出しは同じ道を通る）。
 */
export function rustFieldsAs(
  source: string,
  structName: string,
  direction: WireDirection,
): WireField[] {
  return fieldsOf(source, structName, new Set(), direction);
}

function fieldsOf(
  source: string,
  structName: string,
  seen: ReadonlySet<string>,
  direction: WireDirection | null,
): WireField[] {
  if (seen.has(structName)) {
    throw new Error(`struct ${structName} が自分自身を flatten しています`);
  }
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  // **同じ名前が2つあれば落とす**（TypeScript 側と揃える）。`admin.rs` には
  // `#[cfg(test)] mod tests` があるので、そこに `serde` 派生の同名ヘルパが入ると
  // **黙って片方が選ばれる。**「最初の1つを読む」は選択ではなく偶然になる。
  const hits = lines.flatMap((line, index) =>
    isStructHeader(line, structName) ? [index] : [],
  );
  if (hits.length !== 1 || hits[0] === undefined) {
    throw new Error(`struct ${structName} の宣言が ${hits.length} 個あります`);
  }
  const at = hits[0];

  const attributes = attributesAbove(lines, at);
  const derive = attributes.find((line) => line.startsWith("#[derive("));
  const hasSerialize = derive?.includes("Serialize") === true;
  const hasDeserialize = derive?.includes("Deserialize") === true;
  let serialized: boolean;
  if (direction === null) {
    if (hasSerialize === hasDeserialize) {
      throw new Error(
        `struct ${structName} の向きが決まりません（Serialize / Deserialize のどちらか一方が要る）`,
      );
    }
    serialized = hasSerialize;
  } else {
    // **名乗った向きの `derive` が無ければ落とす。**
    if (!(direction === "serialize" ? hasSerialize : hasDeserialize)) {
      throw new Error(`struct ${structName} は ${direction} の向きを持ちません`);
    }
    serialized = direction === "serialize";
  }
  const renameAll = renameOf(attributes.join("\n"), "rename_all", serialized);
  if (renameAll !== null && renameAll !== "snake_case") {
    throw new Error(`知らない rename_all です: ${renameAll}`);
  }

  const fields: WireField[] = [];
  let pending: string[] = [];
  let depth = 0;
  for (let index = at; index < lines.length; index += 1) {
    const line = (lines[index] ?? "").trim();
    if (index === at) {
      depth = 1;
      continue;
    }
    if (line === "" || line.startsWith("//")) {
      continue;
    }
    if (line.startsWith("#[")) {
      pending.push(line);
      continue;
    }
    depth += countOf(line, "{") - countOf(line, "}");
    if (depth <= 0) {
      break;
    }

    const found = /^(?:pub(?:\([^)]*\))?\s+)?([a-z_][A-Za-z0-9_]*)\s*:\s*(.+?),?$/u.exec(
      line,
    );
    if (found?.[1] === undefined || found[2] === undefined) {
      throw new Error(`struct ${structName} の読めない行: ${line}`);
    }
    const attribute = pending.join("\n");
    pending = [];

    if (
      /\bskip\b/u.test(attribute) ||
      /\bskip_serializing\b(?!_if)/u.test(attribute)
    ) {
      continue;
    }
    if (attribute.includes("flatten")) {
      const inner = found[2];
      if (!/^[A-Za-z0-9_]+$/u.test(inner)) {
        throw new Error(`flatten できない型です: ${inner}`);
      }
      fields.push(
        ...fieldsOf(source, inner, new Set([...seen, structName]), direction),
      );
      continue;
    }

    const renamed = renameOf(attribute, "rename", serialized);
    const name =
      renamed ?? (renameAll === "snake_case" ? toSnakeCase(found[1]) : found[1]);
    // **省略可否は向きで意味が変わる。** 応答は `skip_serializing_if` だけ。
    // **要求は `#[serde(default)]` と `Option<…>` の「どちらか」**
    // ——`serde` は `Option<T>` の欠落を `default` 無しでも `None` として受ける
    // （B-18 のレビューが `serde` だけの小さなクレートで実測）。
    // **`default` の綴りだけを見ると、線の上の契約を1バイトも変えない変異が赤になり、
    // その赤を「写しを直す」と読んだ人が、省略可の欄を必須と書き直す経路ができる。**
    const optional = serialized
      ? attribute.includes("skip_serializing_if")
      : /\bdefault\b/u.test(attribute) || /^Option\s*</u.test(found[2]);
    fields.push({ name, optional });
  }

  if (fields.length === 0) {
    throw new Error(`struct ${structName} に欄がありません`);
  }
  return fields;
}

/** `admin.rs` で `serde` を通る構造体の名前を、**現れる順に**全部。 */
export function rustStructNamesOf(source: string): string[] {
  const lines = source.split("\n").map((line) => line.replace(/\r$/u, ""));
  const names: string[] = [];
  for (let index = 0; index < lines.length; index += 1) {
    const found = /^(?:pub(?:\([^)]*\))?\s+)?struct ([A-Za-z0-9_]+)\s*\{$/u.exec(
      lines[index] ?? "",
    );
    if (found?.[1] === undefined) {
      continue;
    }
    const derive = attributesAbove(lines, index).find((line) =>
      line.startsWith("#[derive("),
    );
    if (derive?.includes("Serialize") === true || derive?.includes("Deserialize") === true) {
      names.push(found[1]);
    }
  }
  if (names.length === 0) {
    throw new Error("serde を通る構造体が1つも見つかりません");
  }
  // **重複は落とす。** 呼び出し側が `Set` に入れるので、黙っていると消える
  // ——2つあることが見えなくなり、`rustFieldsOf` がどちらを読んだかも分からない。
  if (new Set(names).size !== names.length) {
    throw new Error("同じ名前の構造体が2つ以上あります");
  }
  return names;
}

/** その行が `struct <名前> {` か。**前方一致では拾わない。** */
function isStructHeader(line: string, structName: string): boolean {
  const found = /^(?:pub(?:\([^)]*\))?\s+)?struct ([A-Za-z0-9_]+)\s*\{$/u.exec(
    line.trim(),
  );
  return found?.[1] === structName;
}

/** その行の直上に連なる `#[…]` の行。**doc コメントで止まる。** */
function attributesAbove(lines: readonly string[], at: number): string[] {
  const found: string[] = [];
  for (let index = at - 1; index >= 0; index -= 1) {
    const line = (lines[index] ?? "").trim();
    if (!line.startsWith("#[")) {
      break;
    }
    found.unshift(line);
  }
  return found;
}

/**
 * `rename` / `rename_all` の値を、**向きを踏まえて**読む。**無ければ `null`。**
 *
 * # ⚠ **向き別の形がある**（B-18 のレビューが実測）
 *
 * ```rust
 * #[serde(rename_all = "snake_case")]                 // 両向きに効く
 * #[serde(rename_all(serialize = "camelCase"))]       // 応答にだけ効く
 * #[serde(rename(serialize = "x", deserialize = "y"))] // 欄ごと・向きごと
 * ```
 *
 * **`key = "…"` の形しか見ないと、方向別の形が `null` になって素通しする。**
 * 実測では **`rename_all(serialize = "camelCase")` を `StatsView` に足しても
 * `rename(serialize = "characterZZ")` を `CorrectionView.character` に足しても 479 全緑**だった
 * ——**`stat_scale` が `statScale` として線に乗るのに、写しはずれたまま。**
 * この作業単位が塞ごうとしている穴そのもの。
 *
 * **向きが合わないものは効かない**（応答の構造体に付いた `deserialize = …` は
 * 線に乗る綴りを変えない）ので、**その向きだけを取り出す。**
 */
function renameOf(
  attribute: string,
  key: "rename" | "rename_all",
  serialized: boolean,
): string | null {
  for (const line of attribute.split("\n")) {
    if (key === "rename" && line.includes("rename_all")) {
      // **`rename_all` は構造体全体の話**なので、欄の `rename` と混ぜない。
      continue;
    }
    // 両向きに効く形。
    const both = new RegExp(`\\b${key}\\s*=\\s*"([^"]*)"`, "u").exec(line);
    if (both?.[1] !== undefined) {
      return both[1];
    }
    // 向き別の形。**自分の向きの綴りだけが線に乗る。**
    const scoped = new RegExp(`\\b${key}\\s*\\(([^)]*)\\)`, "u").exec(line);
    if (scoped?.[1] !== undefined) {
      const want = serialized ? "serialize" : "deserialize";
      const found = new RegExp(`\\b${want}\\s*=\\s*"([^"]*)"`, "u").exec(
        scoped[1],
      );
      return found?.[1] ?? null;
    }
  }
  return null;
}

/** `alreadySnake` → `already_snake`（serde の `snake_case` と同じ訳し方）。 */
function toSnakeCase(name: string): string {
  return name.replace(/([a-z0-9])([A-Z])/gu, "$1_$2").toLowerCase();
}

/** その文字列が何個あるか。 */
function countOf(line: string, needle: string): number {
  return line.split(needle).length - 1;
}
