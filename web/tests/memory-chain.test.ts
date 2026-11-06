/**
 * **上書きされた記憶の鎖を組み立てる純粋関数**
 * （作業単位 D-3c / `src/lib/memoryRepair.ts` / ADR-0022）。
 *
 * # 何を見るか
 *
 * | 見るもの | なぜ |
 * |---|---|
 * | **どの行が、どれに置き換わったか** | `superseded_by` は「Y へ辿る唯一の手掛かり」 |
 * | **後継がいまどこに居るか** | 「まだ想起される」と「さらに訂正された」は別のこと |
 * | **鎖の途中の行が、そう見えるか** | 上書きされていて、かつ誰かを上書きしている行 |
 * | **読めなかった行も鎖に居るか** | 落とすと**壊れた環だけが鎖から落ちる**（ADR-0022 §2） |
 * | **出した件数と総数の差** | **返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく** |
 * | **`?include=` の綴り** | ずれると**サーバが 400 を返し、一覧ごと出なくなる** |
 *
 * **画面の側（どのセルにどれが出るか）は `memory-superseded.test.ts`。**
 *
 * **I/O を持たない**ので jsdom は要らない（既定の `node` で走る）。
 * **写し元を読む見張り番をこちらに置いているのはそのため**——
 * **`import.meta.url` は jsdom では `file:` にならない**（実測）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  MEMORY_LIST_INCLUDES,
  type SupersededListView,
  type SupersededMemoryView,
  type UnreadableSupersededMemoryView,
} from "../src/lib/api";
import {
  annotateSuperseded,
  chainLabel,
  hiddenSuperseded,
  limitLabel,
  shownSuperseded,
  successorLabel,
  totalSuperseded,
} from "../src/lib/memoryRepair";

/**
 * 鎖 —— **`45 → 41 → 42 → 43`**。
 *
 * * `45` は**読めなかった行**（鎖の一番奥）。**壊れていても後継は分かる。**
 * * `41` と `42` は**鎖の途中**（上書きされていて、かつ誰かを上書きしている）。
 * * `43` は**まだ想起される行**（この節には出てこない）。
 *
 * **`44` は鎖の外**で、後継 `55` は**どちらの一覧にも居ない**（上限で切れた先）。
 */

/** 鎖の一番奥。**読めなかった上書き済みの行。** */
const OLDEST_BROKEN: UnreadableSupersededMemoryView = {
  id: 45,
  invalid_columns: ["content", "locked"],
  superseded_by: 41,
};

/** `45` を置き換え、`42` に置き換わった行。**鎖の途中。** */
const OLD: SupersededMemoryView = {
  id: 41,
  subject: "user",
  content: "店を経営している",
  category: "経歴",
  created_at: "2026-08-26T00:00:00Z",
  superseded_by: 42,
};

/** `41` を置き換え、**まだ想起される `43`** に置き換わった行。**鎖の途中。** */
const MID: SupersededMemoryView = {
  id: 42,
  subject: "character",
  content: "喫茶店を経営している",
  category: "職業",
  created_at: "2026-08-26T00:10:00Z",
  superseded_by: 43,
};

/** 鎖の外。**後継 `55` はどちらの一覧にも居ない。** */
const LONE: SupersededMemoryView = {
  id: 44,
  subject: "user",
  content: "黒い犬を飼っている",
  category: null,
  created_at: "2026-08-26T00:20:00Z",
  superseded_by: 55,
};

/** いま想起される行の id。**`43` は `42` の後継、`46` は無関係な行。** */
const ACTIVE_IDS = [43, 46];

/** **返した件数より多い総数**（上限で切られている状態）。 */
const TOTAL = 113;

/**
 * サーバが返す上限。**返した件数とも総数とも違う値。**
 *
 * **ここでは上限を使う関数が無い**（数えるのは `total` と件数だけ）が、
 * **3つを同値にすると、画面側でどれを出しているか区別できない値が混じる。**
 * 画面の側の腕は `memory-superseded.test.ts` が持つ。
 */
const LIMIT = 9;

function section(): SupersededListView {
  return {
    memories: [MID, OLD, LONE],
    unreadable: [OLDEST_BROKEN],
    total: TOTAL,
    limit: LIMIT,
  };
}

it("フィクスチャの値は互いに違う（同値だと取り違えが観測できない）", () => {
  const ids = [OLDEST_BROKEN.id, OLD.id, MID.id, LONE.id, ...ACTIVE_IDS];
  expect(new Set(ids).size).toBe(ids.length);
  // **id と後継が同じ行が1つも無い**（取り違えが観測できるように）。
  for (const row of [OLDEST_BROKEN, OLD, MID, LONE]) {
    expect(row.superseded_by).not.toBe(row.id);
  }
  // **主語も本文も分類も違う。**
  expect(OLD.subject).not.toBe(MID.subject);
  expect(OLD.content).not.toBe(MID.content);
  expect(OLD.category).not.toBe(MID.category);
  // **`55` はどちらの一覧にも居ない**（「出ていません」の枝が実際に効く状態）。
  expect([...ids]).not.toContain(LONE.superseded_by);
  // **件数・総数・上限が三者三様**（どれを取り違えても見える）。
  const shown = section().memories.length + section().unreadable.length;
  expect(new Set([shown, TOTAL, LIMIT]).size).toBe(3);
});

describe("鎖", () => {
  it("行の並びを変えない（並べ替えるのは `store` の1文だけ）", () => {
    const annotated = annotateSuperseded(section(), ACTIVE_IDS);

    expect(annotated.memories.map((row) => row.memory.id)).toEqual([
      MID.id,
      OLD.id,
      LONE.id,
    ]);
    expect(annotated.unreadable.map((row) => row.broken.id)).toEqual([
      OLDEST_BROKEN.id,
    ]);
  });

  /**
   * **後継は、その行のものである。**
   *
   * `superseded_by` を `id` に取り違えると**鎖が自分を指す**
   * ——「以前はXだったが今はY」の Y へ二度と辿れない。
   */
  it("それぞれの行が、自分の後継を名乗る", () => {
    const annotated = annotateSuperseded(section(), ACTIVE_IDS);
    const successors = new Map(
      annotated.memories.map((row) => [row.memory.id, row.chain.successor]),
    );

    expect(successors.get(OLD.id)).toBe(OLD.superseded_by);
    expect(successors.get(MID.id)).toBe(MID.superseded_by);
    expect(successors.get(LONE.id)).toBe(LONE.superseded_by);
    // **自分を指していない。**
    expect(successors.get(OLD.id)).not.toBe(OLD.id);
    // **読めなかった行でも後継は分かる**（`parse` を通さない列）。
    expect(annotated.unreadable[0]?.chain.successor).toBe(
      OLDEST_BROKEN.superseded_by,
    );
  });

  /**
   * **後継がいまどこに居るか**は3通りあり、どれも別のことを意味する。
   *
   * | 居場所 | 意味 |
   * |---|---|
   * | `active` | **その版がいま想起される**（鎖の先端） |
   * | `superseded` | **さらに訂正された**（鎖が続く） |
   * | `absent` | **この一覧には出ていない**（上限の先） |
   */
  it("後継の居場所を3通りに分ける", () => {
    const annotated = annotateSuperseded(section(), ACTIVE_IDS);
    const places = new Map(
      annotated.memories.map((row) => [row.memory.id, row.chain.place]),
    );

    expect(places.get(MID.id)).toBe("active");
    expect(places.get(OLD.id)).toBe("superseded");
    expect(places.get(LONE.id)).toBe("absent");
    expect(annotated.unreadable[0]?.chain.place).toBe("superseded");
  });

  /**
   * **鎖の途中の行が、そう見えること。**
   *
   * 上書きされていて、**かつ誰かを上書きしている**行。
   * **読めなかった行が指している先も数える**——数えないと、
   * **壊れた環だけが鎖から落ちる。**
   */
  it("鎖の途中の行は、自分が置き換えた行を名乗る", () => {
    const annotated = annotateSuperseded(section(), ACTIVE_IDS);
    const predecessors = new Map(
      annotated.memories.map((row) => [row.memory.id, row.chain.predecessors]),
    );

    // `41` を指しているのは**読めなかった行** `45`。
    expect(predecessors.get(OLD.id)).toEqual([OLDEST_BROKEN.id]);
    expect(predecessors.get(MID.id)).toEqual([OLD.id]);
    // 鎖の外の行と、鎖の一番奥の行には、置き換えた相手が居ない。
    expect(predecessors.get(LONE.id)).toEqual([]);
    expect(annotated.unreadable[0]?.chain.predecessors).toEqual([]);
  });

  it("上書き済みが1件も無ければ、鎖も空", () => {
    const annotated = annotateSuperseded(
      { memories: [], unreadable: [], total: 0, limit: LIMIT },
      ACTIVE_IDS,
    );

    expect(annotated.memories).toEqual([]);
    expect(annotated.unreadable).toEqual([]);
  });
});

describe("鎖の見出し", () => {
  const annotated = (): ReturnType<typeof annotateSuperseded> =>
    annotateSuperseded(section(), ACTIVE_IDS);

  function chainOf(id: number): { successor: string; predecessor: string } {
    const row = annotated().memories.find((entry) => entry.memory.id === id);
    if (row === undefined) {
      throw new Error(`id ${id} が鎖に居ません`);
    }
    return {
      successor: successorLabel(row.chain),
      predecessor: chainLabel(row.chain),
    };
  }

  it("後継の見出しは、居場所ごとに別の文で、必ず id を名乗る", () => {
    expect(chainOf(MID.id).successor).toContain(String(MID.superseded_by));
    expect(chainOf(MID.id).successor).toContain("いま想起されます");
    expect(chainOf(OLD.id).successor).toContain(String(OLD.superseded_by));
    expect(chainOf(OLD.id).successor).toContain("さらに訂正されました");
    expect(chainOf(LONE.id).successor).toContain(String(LONE.superseded_by));
    expect(chainOf(LONE.id).successor).toContain("この一覧には出ていません");

    // **3通りが互いに別の文**（潰れると居場所が読めない）。
    const labels = [
      chainOf(MID.id).successor,
      chainOf(OLD.id).successor,
      chainOf(LONE.id).successor,
    ];
    expect(new Set(labels).size).toBe(3);
  });

  it("鎖の途中は「置き換えた行」を名乗り、そうでない行は名乗らない", () => {
    expect(chainOf(OLD.id).predecessor).toContain(String(OLDEST_BROKEN.id));
    expect(chainOf(OLD.id).predecessor).toContain("鎖の途中");
    expect(chainOf(MID.id).predecessor).toContain(String(OLD.id));

    expect(chainOf(LONE.id).predecessor).not.toContain("鎖の途中");
    // **「ありません」で終わらせない**——何が無いのかを書く。
    expect(chainOf(LONE.id).predecessor).toContain("置き換えた行はありません");
  });
});

describe("出した件数と総数", () => {
  /**
   * **読めなかった行も件数に入る。**
   *
   * 入れないと、**上限で切られた件数が実際より少なく見え**、
   * 「見えていない行」の数が合わなくなる。
   */
  it("出した件数は、読めた行と読めなかった行の合計", () => {
    expect(shownSuperseded(section())).toBe(4);
    expect(shownSuperseded(section())).not.toBe(section().memories.length);
  });

  it("見えていない行は、総数と出した件数の差", () => {
    expect(hiddenSuperseded(section())).toBe(TOTAL - 4);
    // **総数が件数と同じなら、隠れている行は無い。**
    expect(hiddenSuperseded({ ...section(), total: 4 })).toBe(0);
  });

  /**
   * **総数が件数より小さいことは起きないが、負の数を出さない。**
   *
   * 数えるのと読み出すのは**別の文**（`count_superseded_memories` と
   * `list_superseded_memories`）なので、**その間に訂正が入りうる。**
   */
  it("総数が件数を下回っても、負の数を出さない", () => {
    expect(hiddenSuperseded({ ...section(), total: 1 })).toBe(0);
  });
});

/**
 * **応答の欄が欠けたとき**（D-3c のレビューで実測 / 2026-08-28）。
 *
 * **型は `admin.rs` から手で写した主張であって、保証ではない**（`api.ts` 冒頭の ⚠）。
 * **欄名がずれてもサーバは 400 を返さない**ので、ここが素通りすると
 * **`$derived` の中で `TypeError` が投げ、画面が丸ごと固まる。**
 *
 * **画面の側の見え方は `memory-superseded.test.ts`。ここは組み立ての側。**
 */
describe("応答の欄が欠けたとき", () => {
  /** 型の上では作れない応答（欄を落とす）。**テストなので押し込む。** */
  function broken(damaged: unknown): SupersededListView {
    return damaged as SupersededListView;
  }

  it("`memories` が無くても落ちず、残りは鎖に載る", () => {
    const annotated = annotateSuperseded(
      broken({ unreadable: [OLDEST_BROKEN], total: TOTAL, limit: LIMIT }),
      ACTIVE_IDS,
    );

    expect(annotated.memories).toEqual([]);
    expect(annotated.unreadable.map((row) => row.broken.id)).toEqual([
      OLDEST_BROKEN.id,
    ]);
  });

  it("`unreadable` が無くても落ちず、残りは鎖に載る", () => {
    const annotated = annotateSuperseded(
      broken({ memories: [MID, OLD], total: TOTAL, limit: LIMIT }),
      ACTIVE_IDS,
    );

    expect(annotated.memories.map((row) => row.memory.id)).toEqual([
      MID.id,
      OLD.id,
    ]);
    expect(annotated.unreadable).toEqual([]);
  });

  /**
   * **後継が無い行は「辿れない」。`undefined` を id として持たない。**
   *
   * 持つと画面が **`id undefined に置き換わりました`** と書く
   * ——**辿れる先があるかのように読める。**
   */
  it("`superseded_by` が無ければ、後継は `null` で居場所は `unknown`", () => {
    const annotated = annotateSuperseded(
      broken({
        memories: [{ ...MID, superseded_by: undefined }],
        unreadable: [],
        total: TOTAL,
        limit: LIMIT,
      }),
      ACTIVE_IDS,
    );

    const chain = annotated.memories[0]?.chain;
    expect(chain?.successor).toBeNull();
    expect(chain?.place).toBe("unknown");
    expect(successorLabel(chain as NonNullable<typeof chain>)).toBe(
      "置き換わった先が応答にありませんでした（鎖を辿れません）",
    );
    // **`undefined` を文字として出さない。**
    expect(successorLabel(chain as NonNullable<typeof chain>)).not.toContain(
      "undefined",
    );
  });

  /**
   * **`undefined === undefined` で鎖が繋がらないこと。**
   *
   * 自分の id が読めない行を「後継が無い行ぜんぶの先行」にしてしまうと、
   * **壊れた応答の中で、身に覚えのない鎖が生える。**
   */
  it("id が無い行は、誰の先行にもならない", () => {
    const annotated = annotateSuperseded(
      broken({
        memories: [
          { ...MID, id: undefined },
          { ...LONE, superseded_by: undefined },
        ],
        unreadable: [],
        total: TOTAL,
        limit: LIMIT,
      }),
      ACTIVE_IDS,
    );

    expect(annotated.memories[0]?.chain.predecessors).toEqual([]);
    expect(annotated.memories[1]?.chain.predecessors).toEqual([]);
  });

  it("件数は、実際に来た行だけを数える", () => {
    expect(
      shownSuperseded(broken({ unreadable: [OLDEST_BROKEN], total: TOTAL })),
    ).toBe(1);
    expect(shownSuperseded(broken({ total: TOTAL }))).toBe(0);
  });

  /**
   * **総数が無ければ `null`。0 と区別する。**
   *
   * 0 として扱うと「まだ1件もありません」、素通しすると
   * 「**全　件…すべて出しています**」——**どちらも嘘。**
   */
  it("総数が無ければ `null`（0 ではない）", () => {
    expect(totalSuperseded(section())).toBe(TOTAL);
    expect(totalSuperseded(broken({ ...section(), total: undefined }))).toBeNull();
    // **0 は「無い」ではない。**
    expect(totalSuperseded({ ...section(), total: 0 })).toBe(0);
    // **差も出さない**（言えないので 0 に落として、画面は別の文に分岐する）。
    expect(hiddenSuperseded(broken({ ...section(), total: undefined }))).toBe(0);
  });

  it("上限が無ければ、`undefined 件` と書かずにそう言う", () => {
    expect(limitLabel(section())).toBe(`${LIMIT} 件`);
    expect(limitLabel(broken({ ...section(), limit: undefined }))).toBe(
      "応答にありませんでした",
    );
    expect(limitLabel(broken({ ...section(), limit: undefined }))).not.toContain(
      "undefined",
    );
  });
});

/**
 * **`?include=` の綴りはサーバの写しである**（`admin.rs` の `MemoryListInclude`）。
 *
 * ずれると**サーバは 400 を返す**ので、**一覧ごと出なくなる**——
 * 上書き済みだけが消えるのではなく、修復の入口そのものが閉じる。
 */
describe("`?include=` の綴りは写し元と一致する", () => {
  it("`src/server/admin.rs` の `MemoryListInclude` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/server/admin.rs", import.meta.url)),
      "utf8",
    );

    expect(includeSpellingsOf(source)).toEqual([...MEMORY_LIST_INCLUDES]);
  });

  it("対照: 植えたソースからは、その場の綴りを読む", () => {
    const planted = [
      '#[serde(rename_all = "snake_case")]',
      "enum MemoryListInclude {",
      "    PlantedVariant,",
      "}",
    ].join("\n");
    expect(includeSpellingsOf(planted)).toEqual(["planted_variant"]);
  });

  /**
   * **受理集合を、本番のパーサより狭くしない**（CLAUDE.md `16l`）。
   *
   * **`serde` はどちらも普通に受ける。** 見落とすと、
   * **線に乗る綴りがずれたまま、この検査だけが緑を通る。**
   */
  it("末尾のコンマが無い列挙子も読む", () => {
    const planted = [
      '#[serde(rename_all = "snake_case")]',
      "enum MemoryListInclude {",
      "    /// doc コメントは読み飛ばす。",
      "    FirstOne,",
      "    LastOne",
      "}",
    ].join("\n");
    expect(includeSpellingsOf(planted)).toEqual(["first_one", "last_one"]);
  });

  it("列挙子ごとの `rename` は、そちらが線に乗る綴り", () => {
    const planted = [
      '#[serde(rename_all = "snake_case")]',
      "enum MemoryListInclude {",
      '    #[serde(rename = "wire_name")]',
      "    Superseded,",
      "    OtherOne,",
      "}",
    ].join("\n");
    // **`rename` は、その列挙子1つにだけ効く。**
    expect(includeSpellingsOf(planted)).toEqual(["wire_name", "other_one"]);
  });

  it("対照: 探せていないことを緑にしない", () => {
    // 列挙そのものが無い。
    expect(() =>
      includeSpellingsOf("enum Other {\n    Superseded,\n}"),
    ).toThrow();
    // 列挙子が1つも無い。
    expect(() =>
      includeSpellingsOf(
        '#[serde(rename_all = "snake_case")]\nenum MemoryListInclude {\n}',
      ),
    ).toThrow();
    // **`rename_all` が無ければ、線に乗る綴りは snake_case ではない。**
    expect(() =>
      includeSpellingsOf("enum MemoryListInclude {\n    Superseded,\n}"),
    ).toThrow();
  });
});

/**
 * `enum MemoryListInclude` の列挙子を、**線に乗る綴り**で取り出す。
 *
 * # ⚠ **受理集合を、本番のパーサより狭くしないこと**（CLAUDE.md `16l`）
 *
 * **当初は「末尾にコンマがある行」しか見ておらず、
 * 列挙子ごとの `#[serde(rename = "…")]` も見ていなかった**（D-3c のレビューで指摘）。
 * どちらも **`serde` は普通に受ける**ので、
 * **最後の列挙子からコンマを外す／`rename` を1つ足す**だけで、
 * **線に乗る綴りが黙ってずれたまま、この検査は緑を通る。**
 *
 * **`rename_all = "snake_case"` を確かめてから訳す。** 確かめないと、
 * **属性が外れた日に、こちらは snake_case のまま緑になる。**
 */
function includeSpellingsOf(source: string): string[] {
  const start = source.indexOf("enum MemoryListInclude {");
  if (start < 0) {
    throw new Error("enum MemoryListInclude が見つかりません");
  }
  const attributes = source.slice(Math.max(0, start - 200), start);
  if (!attributes.includes('rename_all = "snake_case"')) {
    throw new Error("MemoryListInclude に rename_all = snake_case がありません");
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("\n}");
  const block = end < 0 ? rest : rest.slice(0, end);

  const spellings: string[] = [];
  /** 直前の行にあった `#[serde(rename = "…")]`。**その列挙子だけに効く。** */
  let renamed: string | null = null;
  for (const raw of block.split("\n")) {
    const line = raw.replace(/\r$/u, "").trim();
    if (line === "" || line.startsWith("//")) {
      continue;
    }
    if (line.startsWith("#[")) {
      const found = /rename\s*=\s*"([^"]*)"/u.exec(line);
      // **`rename_all` は列挙全体の話**なので、列挙子の `rename` と混ぜない。
      if (found?.[1] !== undefined && !line.includes("rename_all")) {
        renamed = found[1];
      }
      continue;
    }
    // **コンマは任意**（最後の列挙子には無いことがある）。
    const variant = /^([A-Z][A-Za-z0-9]*)\s*,?$/u.exec(line);
    if (variant?.[1] === undefined) {
      continue;
    }
    spellings.push(renamed ?? toSnakeCase(variant[1]));
    renamed = null;
  }

  if (spellings.length === 0) {
    throw new Error("MemoryListInclude に列挙子がありません");
  }
  return spellings;
}

/** `Superseded` → `superseded`（serde の `snake_case` と同じ訳し方）。 */
function toSnakeCase(name: string): string {
  return name.replace(/([a-z0-9])([A-Z])/gu, "$1_$2").toLowerCase();
}
