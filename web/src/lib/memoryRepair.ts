/**
 * 記憶の**訂正版を組み立てる**（作業単位 D-3 / ADR-0022）。
 *
 * **I/O を持たない。** 送るのは `api.ts`、画面は `MemoryList.svelte`。
 * ここに寄せてあるのは、**「何を送るか」だけを単体で見張れるようにする**ため
 * （`characterEdit.ts` と同じ分け方）。
 *
 * # **消す道はここにも無い**（絶対制約 / ADR-0022 §1）
 *
 * 組み立てられるのは**訂正版**だけで、**後継の無い取り消しは表現できない**
 * ——`content` が空なら `buildCorrection` は失敗を返す。
 * 「消したい」は「今は何が正しいか」を言えるはず、という立場をそのまま持つ。
 *
 * # 写していないもの（**増やさないこと**）
 *
 * | 写していない | 代わりに |
 * |---|---|
 * | `MemoryContent::MAX_CHARS` / `MemoryCategory::MAX_CHARS` | 打たせて、サーバの理由を見せる |
 * | 分類の語彙（サーバは自由文として受ける） | 同上 |
 *
 * **`characterEdit.ts` は上限を写している**が、あちらは**文字数計を画面に出す**ため
 * （写さないと画面が嘘の数を出す）。**ここには計が無いので、写す理由も無い。**
 *
 * # 上書きされた記憶の**鎖**（作業単位 D-3c）
 *
 * 後半に `annotateSuperseded` 以下がある。**送るものとは関係が無い**が、
 * **画面から出す前に組み上がっている値**なので、こちらへ寄せてある
 * （`MemoryList.svelte` の中で組むと、jsdom を立てないと1つも見張れない）。
 */

import type {
  CorrectMemoryRequest,
  MemorySubject,
  MemoryView,
  SupersededListView,
  SupersededMemoryView,
  UnreadableSupersededMemoryView,
} from "./api";

/** 選べる主語。**画面の選択肢はここから作る**（綴りの写しを増やさない）。 */
export const SUBJECTS: readonly MemorySubject[] = ["user", "character"];

/**
 * 主語の見出し。
 *
 * **`user` を「あなた」と書かない。** 抽出のプロンプトが使う綴りとは別物で、
 * 管理の画面では**どちらの記憶か**が一目で分かることだけが要る。
 */
export function subjectLabel(subject: MemorySubject): string {
  switch (subject) {
    case "user":
      return "利用者";
    case "character":
      return "キャラクター";
  }
}

/**
 * 訂正の下書き。**主語は「まだ選んでいない」を持つ。**
 *
 * **`null` を既定値の代わりに使っているのではない。**
 * 読めなかった行では**元の主語が分からない**ので、
 * **こちらが勝手に決めると、押し間違いで持ち主の入れ替わった記憶ができる**
 * ——しかも持ち主を取り違えた記憶は、訂正版も同じ持ち主の下に入るので
 * **正しい持ち主へ移す手段が無い**（CLAUDE.md の ⚠）。
 */
export interface CorrectionDraft {
  subject: MemorySubject | null;
  content: string;
  /** **空文字は「未設定」。** `null` を画面へ出さない。 */
  category: string;
}

/** 読めた行の下書き（**その行の値から始まる**）。 */
export function draftForMemory(memory: MemoryView): CorrectionDraft {
  return {
    subject: memory.subject,
    content: memory.content,
    category: memory.category ?? "",
  };
}

/**
 * 読めなかった行の下書き。**何も埋めない。**
 *
 * 元の値は API から返ってこない（`UnreadableMemoryView` は `id` と
 * 壊れた列名しか持たない）。**推測で埋めない**——推測した本文が
 * 「今は何が正しいか」として記憶に残る。
 */
export function draftForUnreadable(): CorrectionDraft {
  return { subject: null, content: "", category: "" };
}

/** 組み立ての結果。**失敗は「送らない」で、理由を画面に出す。** */
export type BuiltCorrection =
  | { ok: true; body: CorrectMemoryRequest }
  | { ok: false; message: string };

/**
 * 訂正版の本文を組み立てる。
 *
 * **本文は生のまま送る**（サーバが正規化する / `characterEdit.ts` の `buildCreate` と同じ）。
 * 空かどうかの判定にだけ `trim()` を使う。
 *
 * **分類が空ならキーごと落とす。** 空文字を送るとサーバの
 * `MemoryCategory::parse` が「空です」で 400 にする。落とせば「未設定」になる。
 */
export function buildCorrection(draft: CorrectionDraft): BuiltCorrection {
  if (draft.subject === null) {
    return {
      ok: false,
      message: "主語（誰についての記憶か）を選んでください。",
    };
  }
  if (draft.content.trim() === "") {
    return {
      ok: false,
      // **「消す」への誘導を書かない。** 消す道は存在しない（ADR-0022 §1）。
      message: "訂正版の本文を入れてください（いま何が正しいかを書きます）。",
    };
  }

  const body: CorrectMemoryRequest = {
    subject: draft.subject,
    content: draft.content,
  };
  if (draft.category.trim() !== "") {
    body.category = draft.category;
  }
  return { ok: true, body };
}

/**
 * 元の行と何も変わっていないか。**変わっていなければ送らない。**
 *
 * 同じ内容の訂正版を足しても**想起される内容は変わらず、行だけが増える。**
 *
 * # ⚠ **本文だけで判定しないこと**
 *
 * 実測で最も多い誤りは**主語の取り違え**（ADR-0021 §10 / 全行に均して約 6%）で、
 * その訂正は**本文が1文字も変わらない。**
 * 本文だけを見ると、**いちばん直したい誤りだけが送れなくなる。**
 * 見張り番は `tests/memory-repair.test.ts::主語だけを直す訂正は送れる`。
 */
export function isUnchanged(
  memory: MemoryView,
  draft: CorrectionDraft,
): boolean {
  return (
    draft.subject === memory.subject &&
    draft.content === memory.content &&
    draft.category === (memory.category ?? "")
  );
}

/**
 * **上書きされた行の後継が、いまどこに居るか。**
 *
 * | | 意味 |
 * |---|---|
 * | `active` | **その版がいま想起される**（鎖の先端） |
 * | `superseded` | **さらに訂正された**（鎖が続く） |
 * | `absent` | **この一覧には出ていない**（上限の先 / `total` にだけ残っている） |
 * | `unknown` | **応答に後継が無かった**（鎖を辿れない / 下の ⚠） |
 *
 * **4つを畳まないこと。** 畳むと「もう直っている」と「まだ先がある」が
 * 同じ表示になり、**どこまで辿ればいまの版なのかが読めなくなる。**
 *
 * # ⚠ **`unknown` は「応答の形が違った」ときにだけ出る**
 *
 * **応答の形は誰も検証していない**（`api.ts` 冒頭の ⚠ ——
 * 型は手で写した主張であって、保証ではない）。
 * **欄が欠けたまま読むと `id undefined に置き換わりました` と書く**
 * ——**鎖が黙って嘘をつく。** そう書くくらいなら**辿れないと言う。**
 */
export type SuccessorPlace = "active" | "superseded" | "absent" | "unknown";

/** 1行ぶんの鎖の位置。 */
export interface SupersededChain {
  /**
   * **この行が置き換わった先**（`superseded_by`）。**この行の id ではない。**
   *
   * **`null` は「応答に無かった／数ではなかった」**（`place` は `unknown`）。
   */
  successor: number | null;
  /** 後継の居場所。 */
  place: SuccessorPlace;
  /**
   * **この一覧の中で**、この行を後継として指している行の id。
   *
   * **空でなければ、この行は鎖の途中**——上書きされていて、かつ誰かを上書きしている。
   * **「この一覧の中で」**が要点で、上限の先にある行は数えられない
   * （数えられないことは `total` との差で読む）。
   */
  predecessors: number[];
}

/** 鎖を添えた行。**元の行をそのまま持つ**（写さない）。 */
export interface AnnotatedSupersededMemory {
  memory: SupersededMemoryView;
  chain: SupersededChain;
}

/** 鎖を添えた、読めなかった行。 */
export interface AnnotatedUnreadableSuperseded {
  broken: UnreadableSupersededMemoryView;
  chain: SupersededChain;
}

/** 節ぜんぶに鎖を添えたもの。**読めた行と読めなかった行を混ぜない。** */
export interface AnnotatedSuperseded {
  memories: AnnotatedSupersededMemory[];
  unreadable: AnnotatedUnreadableSuperseded[];
}

/**
 * 上書きされた記憶に**鎖の位置を添える**（作業単位 D-3c / ADR-0022）。
 *
 * # **並べ替えない**
 *
 * 並びを決めているのは `store` の1文（`Store::list_superseded_memories`）。
 * ここで組み直すと、**同じ「新しい順」という言葉が2つの意味を持つ。**
 *
 * # **読めなかった行を落とさない**
 *
 * 落とすと**壊れた環だけが鎖から落ち、`total` にだけ残る**（ADR-0022 §2）。
 * **後継を探すときも、読めなかった行を数に入れる**——
 * `45 → 41` の `45` が読めない行なら、**落とした瞬間に `41` は
 * 「鎖の途中」ではなく「一番奥」に見える。**
 *
 * @param activeIds いま想起される行の id（**応答の `memories` と `unreadable` の両方**）。
 */
export function annotateSuperseded(
  section: SupersededListView,
  activeIds: Iterable<number>,
): AnnotatedSuperseded {
  const active = new Set(activeIds);
  // **`?? []` を外さないこと**（`MemoryList.svelte` の `list?.memories ?? []` と同じ守り）。
  // 外すと、**欄が1つ欠けただけで `$derived` の中で `TypeError` が投げ、
  // 画面が丸ごと「読み込み中…」で固まる** ——
  // **有効な行も、読めない行も、「訂正版を足す」ボタンも1つも出なくなる。**
  // **サーバは 400 も返さない**ので、外からは遅いようにしか見えない。
  const memories = section.memories ?? [];
  const unreadable = section.unreadable ?? [];
  const listed: readonly { id: number; superseded_by: number }[] = [
    ...memories,
    ...unreadable,
  ];
  const supersededIds = new Set(listed.map((row) => row.id).filter(isId));

  const chainFor = (row: {
    id: number;
    superseded_by: number;
  }): SupersededChain => {
    // **数でなければ `null`。** `undefined` をそのまま持つと、
    // 画面が `id undefined に置き換わりました` と書く（鎖が嘘をつく）。
    const successor = isId(row.superseded_by) ? row.superseded_by : null;
    return {
      successor,
      place: placeOf(successor, active, supersededIds),
      // **自分の id が読めない行は、誰の後継でもない**
      // （`undefined === undefined` で全部を拾ってしまうのを防ぐ）。
      predecessors: isId(row.id)
        ? listed
            .filter((other) => other.superseded_by === row.id)
            .map((other) => other.id)
            .filter(isId)
        : [],
    };
  };

  return {
    memories: memories.map((memory) => ({
      memory,
      chain: chainFor(memory),
    })),
    unreadable: unreadable.map((broken) => ({
      broken,
      chain: chainFor(broken),
    })),
  };
}

/**
 * **id として読める値か。**
 *
 * **応答の形は検証していない**ので、欄が欠けていれば `undefined` が来る
 * （`api.ts` 冒頭の ⚠）。**数でないものを id として扱わない。**
 */
function isId(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

/**
 * 後継の居場所。**有効な行を先に見る。**
 *
 * 同じ id が両方に居ることは無い（想起の条件 `superseded_by IS NULL` は
 * 排他）が、**順序を決めておかないと、片方が壊れた応答で意味が揺れる。**
 */
function placeOf(
  successor: number | null,
  active: ReadonlySet<number>,
  superseded: ReadonlySet<number>,
): SuccessorPlace {
  if (successor === null) {
    return "unknown";
  }
  if (active.has(successor)) {
    return "active";
  }
  if (superseded.has(successor)) {
    return "superseded";
  }
  return "absent";
}

/**
 * 後継の見出し。**必ず id を名乗る。**
 *
 * **居場所ごとに別の文にする。** 同じ文にすると、
 * 「もう直っている」と「まだ先がある」が画面から区別できなくなる。
 */
export function successorLabel(chain: SupersededChain): string {
  // **id を名乗れないなら、名乗らない。**
  // `id undefined に置き換わりました` は、辿れる先があるかのように読める。
  if (chain.place === "unknown" || chain.successor === null) {
    return "置き換わった先が応答にありませんでした（鎖を辿れません）";
  }
  switch (chain.place) {
    case "active":
      return `id ${chain.successor} に置き換わりました（いま想起されます）`;
    case "superseded":
      return `id ${chain.successor} に置き換わりました（さらに訂正されました）`;
    case "absent":
      return `id ${chain.successor} に置き換わりました（この一覧には出ていません）`;
  }
}

/**
 * **鎖の途中かどうか**の見出し。
 *
 * **「ありません」で終わらせない。** 何が無いのかを書く——
 * この一覧に置き換えた相手が居ないだけで、**上限の先に居るかもしれない。**
 */
export function chainLabel(chain: SupersededChain): string {
  if (chain.predecessors.length === 0) {
    return "この一覧に、この行が置き換えた行はありません";
  }
  return `id ${chain.predecessors.join("、")} を置き換えた版です（鎖の途中）`;
}

/**
 * **この節に出した行の数。**
 *
 * **読めなかった行も数える**——サーバの上限は行の種別で分けていない
 * （`list_superseded_memories` は両方を1つの列で返す）。
 * 数えないと、**「見えていない行」の数が合わなくなる。**
 */
export function shownSuperseded(section: SupersededListView): number {
  // **`?? []` を外さないこと**（`annotateSuperseded` と同じ理由）。
  return (section.memories?.length ?? 0) + (section.unreadable?.length ?? 0);
}

/**
 * **保存されている総数。`null` は「応答に無かった」。**
 *
 * # ⚠ **無かったものを 0 や空白として出さない**
 *
 * 欄が欠けたまま描くと「**全　件…すべて出しています**」になる
 * ——**数を言わないまま「これで全部」と言い切る**、この画面がいちばん避けたい形。
 * **総数が読めないなら、読めないと言う。**
 */
export function totalSuperseded(section: SupersededListView): number | null {
  return typeof section.total === "number" && Number.isFinite(section.total)
    ? section.total
    : null;
}

/**
 * **1回に返る上限の見出し。`null` は「応答に無かった」。**
 *
 * **上限はサーバが決めて応答に載せてくる**（写しを持たない / `api.ts`）ので、
 * **載ってこなければ言えない。** `undefined 件` と書かない。
 */
export function limitLabel(section: SupersededListView): string {
  return typeof section.limit === "number" && Number.isFinite(section.limit)
    ? `${section.limit} 件`
    : "応答にありませんでした";
}

/**
 * **ここに出ていない行の数**（`total` との差）。
 *
 * **負の数を出さない。** 数えるのと読み出すのは別の文なので、
 * **その間に訂正が入って総数が動きうる。**
 *
 * **総数が読めないときは 0**（差を言えないので、画面は別の文に分岐する）。
 */
export function hiddenSuperseded(section: SupersededListView): number {
  const total = totalSuperseded(section);
  if (total === null) {
    return 0;
  }
  return Math.max(0, total - shownSuperseded(section));
}
