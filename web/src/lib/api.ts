/**
 * 管理 API（`/admin/`）の型付き境界。
 *
 * # ⚠ **この型は手で写したものである**（ADR-0031 §6）
 *
 * **生成していない。** 写し元は `src/server/admin.rs` の `*View` 構造体
 * （`CharacterListView` / `CharacterView` / `UnreadableCharacterView` /
 * `StatsView` / `StatAllocView` / `UpdatedCharacterView`）、
 * **要求側の `CreateCharacterRequest` / `UpdateCharacterRequest`**（作業単位 D-2）、
 * および `src/server/error.rs` の `ErrorBody`。
 *
 * **⚠ 上限（`Persona::MAX_CHARS` など）はここに無い。** 写し元が違う
 * （`src/domain/character.rs`）ので `src/lib/characterEdit.ts` が持つ。
 * **そちらには写し元を読む見張り番がある**（`tests/character-edit.test.ts`）。
 *
 * **したがって Rust 側を変えると、ここは黙って古びる。**
 * 型検査は「TypeScript の中で辻褄が合っている」ことしか見ていない
 * ——**`admin.rs` 側を1文字変えても、TypeScript は何も知らない。**
 * 画面が空になる・欄が消えるという形で、実行時に初めて現れる。
 *
 * # **欄名だけは、写し元を読んで突き合わせている**（作業単位 B-18）
 *
 * **見張り番は `tests/api-fields.test.ts`。** `src/server/admin.rs` を実際に読み、
 * **`serde` の属性まで解いた「線に乗る欄名と省略可否」**を、
 * このファイルの 21 個の型と**過不足なく**突き合わせる（**植えたソースの対照つき**）。
 * **構造体の一覧そのものも数えている**ので、
 * **`admin.rs` に `*View` が増えた日にも赤くなる。**
 *
 * **`?` のキーの綴りも同じ見張り番が見る**
 * （`MemoryListQuery.include` ↔ `listMemories` が `URLSearchParams` に組ませる名前）。
 * **ずれるとサーバが `deny_unknown_fields` で 400 を返し、記憶の一覧そのものが出なくなる。**
 *
 * **⚠ 見ているのは欄名と省略可否だけ。**
 * **欄の「型」（`number` か `string` か）は見ていない**——そこは今も写しのままで、
 * ずれても誰も気付かない（ADR-0031 §6）。
 *
 * **⚠ `error.rs` の `ErrorBody` は対象外**（写し元が `admin.rs` ではなく、
 * こちら側も無名の入れ子）。`code` の綴りには別の見張り番がある
 * （`tests/character-delete.test.ts` / `tests/character-repair.test.ts`）。
 *
 * # 最初の画面が使う分だけ
 *
 * **全部の型を先に書かない**（作業単位 D-0）。使っていない型は、
 * 写し元と食い違っていても誰も気付けない——ただの嘘が増えるだけになる。
 */

/** 読み出せたキャラクター1件（`admin.rs` の `CharacterView`）。 */
export interface CharacterView {
  id: number;
  handle: string;
  display_name: string;
  base_persona: string;
  /** 未設定は `null`（キーごと落とさない、という Rust 側の約束）。 */
  output_format_instruction: string | null;
  /** 性格ステータスの尺度（ADR-0015）。 */
  stat_scale: number;
  /**
   * **書いた変容の走らせ方**（ADR-0045 / 作業単位 A-8）。**未設定は `null`**
   * （起動時の既定に従う）。**効いている値ではない** —— 効いている値は
   * 変容の応答（`DriftView.run_mode`）が言う。
   *
   * **読み手は編集の画面**（`CharacterForm.svelte` が下書きの初期値にする / 作業単位 D-15）。
   */
  stat_drift_mode: string | null;
  /**
   * **見いだす枠の数**（ADR-0046 §決定1 / 作業単位 A-9a）。0〜4、既定 0。
   *
   * **読み手はまだ居ない**（画面は ADR-0046 の第2段）。写しの走査（`api-fields`）が
   * `CharacterView` を丸ごと突き合わせるので、欄だけを写してある。
   */
  axis_discovery_slots: number;
  /** RFC 3339。 */
  created_at: string;
  /**
   * **しまった時刻**（RFC 3339 / ADR-0049 §決定1 / 作業単位 A-10）。**しまっていなければ `null`**
   * （キーごと落とさない、という Rust 側の約束）。
   *
   * **読み手は `characterArchive.ts`**（上の一覧としまった一覧の出し分け・会話の相手の選択肢）。
   */
  archived_at: string | null;
}

/**
 * **読み出せなかった行**（`admin.rs` の `UnreadableCharacterView`）。
 *
 * **これを捨てないこと**（ADR-0016 §3）。捨てると
 * 「一覧に出ないキャラクターがいる」という壊れ方になり、
 * **修復の入口が画面から消える。**
 */
export interface UnreadableCharacterView {
  id: number;
  /** `null` は「識別子そのものが壊れていて API からは指せない」。 */
  handle: string | null;
  invalid_columns: string[];
}

/** `GET /admin/characters` の応答（`admin.rs` の `CharacterListView`）。 */
export interface CharacterListView {
  characters: CharacterView[];
  unreadable: UnreadableCharacterView[];
}

/** 1軸分（`admin.rs` の `StatAllocView`）。 */
export interface StatAllocView {
  axis: string;
  value: number;
}

/** `GET /admin/characters/{handle}/stats` の応答（`admin.rs` の `StatsView`）。 */
export interface StatsView {
  character: string;
  stat_scale: number;
  stats: StatAllocView[];
}

/**
 * `PUT /admin/characters/{handle}/stats` の要求（`admin.rs` の `ReplaceStatsRequest`）。
 *
 * # ⚠ **この配列が全体である。含まれない軸は消える**
 *
 * **`PATCH` の「キーの有無が意味を持つ」とは正反対**で、
 * **「変えない」を表す手段が無い。** 空の配列を渡すと**全軸が消える。**
 *
 * **⚠ したがって、画面は必ず全軸を送ること**（`statsEdit.ts` の `buildReplace`）。
 * 差分だけ送る `buildUpdate` の作法をここへ持ち込むと、
 * **送らなかった軸が黙って消える。**
 *
 * **⚠ サーバは `deny_unknown_fields`。** ここに無いキーを送らないこと。
 */
export interface ReplaceStatsRequest {
  stats: StatAllocRequest[];
}

/**
 * 1軸分の指定（`admin.rs` の `StatAllocRequest`）。
 *
 * **`value` の 0 は正当な値**（落とすのは `prompt` 層の責務）。
 * **上端はそのキャラクターの尺度**だが、**こちらでは判定しない**
 * ——超えていればサーバが `stat_value_above_scale` を返す。
 */
export interface StatAllocRequest {
  axis: string;
  value: number;
}

/**
 * 変容の**向き**（`admin.rs` の `delta_to_wire` の綴り）。
 *
 * **⚠ これも手で写した型である。** サーバが将来この2つ以外を返しても、
 * TypeScript は何も言わない（このファイル冒頭の ⚠）。
 * **画面は生の値も併記する**（`StatsPanel.svelte` の `data-delta`）ので、
 * 外れたときに気付ける形にしてある。
 * **綴りの突き合わせは `tests/stats-edit.test.ts`**（`delta_to_wire` を実際に読む）。
 */
export type DriftDelta = "up" | "down";

/**
 * 1軸分の変容の姿（`admin.rs` の `DriftAxisView`）。
 *
 * **`charge` は行が無ければ 0 で返る。**
 * **⚠ その 0 は「一度も採点されていない」とも「0 に戻った」とも読める** ——
 * **分けるのは `charge_updated_at` の方**（作業単位 D-5a）。
 *
 * **`ups` と `downs` は畳まれていない。** 差にすると
 * 「1回も動いていない」と「押し合った」が同じ値になる（`DriftAxisView` の doc）
 * ——**画面でも畳まないこと。**
 */
export interface DriftAxisView {
  axis: string;
  /**
   * いまの値。**割り当てが無ければ `null`**（作業単位 B-16）。
   *
   * # `null` が返る状態
   *
   * `PUT .../stats` は**丸ごと置き換え**なので、**軸を改名すると綴りが変わる。**
   * サーバ側の累積と票の行は**軸の割り当てを参照していない**ので消えず、
   * **総数には数えられ続ける。**
   * その票がどの行にも現れないのは辻褄が合わないので、
   * サーバは**その軸も返す。ただし、いまの値は持たないので `null`。**
   *
   * # ⚠ **`?? 0` で埋めないこと**
   *
   * **`0` は人が書いた宣言**（「この軸は語らない」/ ADR-0024 §4）で、
   * **変容の対象外**という別の意味を持つ。埋めた瞬間に
   * **「まだ動かない軸」と「もう無い軸」が画面で同じ行になる。**
   * サーバが `null` を選んだ理由がそれ（`admin.rs` の `DriftAxisView::value`）。
   *
   * 画面の見出しは `statsEdit.ts` の `axisValueLabel` が1箇所で決める。
   */
  value: number | null;
  charge: number;
  /**
   * **その軸の累積が最後に書き戻された時刻**（RFC 3339 / 作業単位 D-5a）。
   *
   * # `null` は「累積の行が無い」＝**一度も適用まで届いていない**
   *
   * **`charge` の 0 とは別のことを言っている。**
   * 行があって `charge: 0` なら「**溜まっていたものが 0 まで薄れた**」、
   * 行が無ければ「**一度も採点されていない**」。
   * **サーバ側は `null` を選んでいる**（`admin.rs` の `charge_updated_at`）——
   * `0` でも現在時刻でも埋めると、**「たったいま動いた」と読める。**
   *
   * # ⚠ 票の `created_at` とは別の時刻
   *
   * 票は**判定のたび**に立つ（記録専用モードでも立つ）が、
   * 累積が動くのは**値を動かしてよいモードのときだけ**。
   * **`run_mode: "shadow"` で回している間は、票の時刻だけが進んでここは止まる。**
   */
  charge_updated_at: string | null;
  ups: number;
  downs: number;
}

/** 票1件（`admin.rs` の `DriftVoteView`）。**`id` は無い**（1件を指すパスが無い）。 */
export interface DriftVoteView {
  axis: string;
  delta: DriftDelta;
  /** **どの往復で立った票か。** 落とすと、どの会話が動かしたのかを言えない。 */
  session_id: number;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * `GET /admin/characters/{handle}/drift` の応答（`admin.rs` の `DriftView`）。
 *
 * # **これが無いと「変容したのか、人が設定したのか」が分からない**
 *
 * 変容は本文にもユーザーにも見せない（ADR-0024 §9）ので、
 * **`GET .../stats` の「いまの値」だけでは、動いたことすら見えない。**
 *
 * # ⚠ **`total_votes` は `recent_votes` の件数ではない**
 *
 * **返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく**
 * （`admin.rs` の `RECENT_DRIFT_VOTES` の doc）。
 * **`total_votes - recent_votes.length` が「見えていない票」。**
 *
 * **⚠ `recent_limit` を写さないこと。** 上限はサーバが決めて、応答に載せてくる。
 * 写すと**サーバ側の定数を変える変異が画面側で緑を通る。**
 */
export interface DriftView {
  character: string;
  stat_scale: number;
  /** 軸ごとの姿。**値 0 の軸も返る**（落とすのは `prompt` 層の責務）。 */
  axes: DriftAxisView[];
  /** **保存されている票の総数**（上限で切られていない）。 */
  total_votes: number;
  /** `recent_votes` に載る上限。**`total_votes` との差が「見えていない票」。** */
  recent_limit: number;
  /** **直近の票。新しい順。** ここで並べ替えないこと（並びは `store` の1文が決める）。 */
  recent_votes: DriftVoteView[];
  /**
   * **いまサーバが走らせている変容の走らせ方**（`off` / `shadow` / `on` /
   * 作業単位 D-5a / `admin.rs` の `DriftView::run_mode`）。
   *
   * # **これが無いと「壊れている」と「off である」が区別できない**
   *
   * **サーバ側の既定は `off`** で、`off` は判定もせず票も累積も1行も作らない。
   * **つまり既定の配置では `total_votes: 0` / 全軸 `charge: 0` /
   * `recent_votes: []` が返る**——**変容が壊れている状態と応答が同じになる。**
   *
   * # ⚠ 綴りを `DriftDelta` のように閉じた型にしていない
   *
   * **`string` のまま受ける。** サーバが4つ目の走らせ方を足した日に、
   * **画面から走らせ方の欄が消えるのがいちばん質の悪い壊れ方**なので、
   * **訳せない綴りはそのまま出す**（`statsEdit.ts` の `runModeLabel` /
   * `deltaLabel` と同じ立場）。
   */
  run_mode: string;
  /**
   * **`run_mode` がどこから来たか**（`character` / `default` / ADR-0045 §決定4 / 作業単位 A-8）。
   *
   * **読み手は `StatsPanel.svelte`**（出どころの一言と、止まっているときの案内の言い分け /
   * 作業単位 D-15）。`run_mode` と同じく `string` のまま受ける。
   */
  run_mode_source: string;
  /**
   * **起動時の既定**（`PERSONACORE_STAT_DRIFT`。無ければ `off` / ADR-0045 §決定4）。
   * キャラクターが値を持っていても載る。**読み手は編集の画面**（`CharacterList.svelte` が引いて
   * `CharacterForm.svelte` の「既定に従う（いまは: …）」の括弧に渡す / 作業単位 D-15）。
   */
  default_run_mode: string;
}

/**
 * `POST /admin/characters` の要求（`admin.rs` の `CreateCharacterRequest`）。
 *
 * **⚠ サーバは `deny_unknown_fields`。** 綴りを間違えたキーを1つ足すだけで
 * 400 になり、**1件も作られない。** 逆に言えば、**ここに無いキーを送らないこと。**
 *
 * **省略できるキーは「省く」こと**（`undefined` を明示的に入れない）。
 * `exactOptionalPropertyTypes` が効いているので、型の側でもそう強制される。
 *
 * | 省いたとき | サーバの既定 |
 * |---|---|
 * | `display_name` | **`handle` と同じ値**（`admin.rs` の `display_name_of`） |
 * | `output_format_instruction` | 未設定（`prompt` が節ごと落とす） |
 * | `stat_scale` | `StatScale::DEFAULT`（**ここには写さない。** 画面の説明文にだけ写してある —— `CharacterForm.svelte`） |
 */
export interface CreateCharacterRequest {
  /** 識別子。**制限された文字集合**（ADR-0017）。**作成のときだけ決められる。** */
  handle: string;
  display_name?: string;
  base_persona: string;
  output_format_instruction?: string;
  stat_scale?: number;
  /**
   * 変容の走らせ方（ADR-0045 / 作業単位 A-8）。**省けば未設定**（起動時の既定に従う）。
   * 受けるのは `off` / `shadow` / `on` だけ（別名は 400）。
   * **送り手は `CharacterForm.svelte` の新規作成**（「既定に従う」なら省く / 作業単位 D-15）。
   */
  stat_drift_mode?: string;
}

/**
 * `PATCH /admin/characters/{handle}` の要求（`admin.rs` の `UpdateCharacterRequest`）。
 *
 * # **キーの有無が意味を持つ**
 *
 * | JSON | 意味 |
 * |---|---|
 * | キーごと省略 | **変えない** |
 * | `"..."` | その値にする |
 * | `null`（`output_format_instruction` だけ） | **未設定に戻す**（消す） |
 *
 * **`display_name` / `base_persona` / `stat_scale` に `null` を送らないこと。**
 * サーバはこの3つの `null` を「変えない」として受けるが、
 * **「変えない」の表現を2つ持つと、片方だけが `null` の扱いを取りこぼす形で古びる。**
 * 変えないならキーごと省く（型の上でもそれしか書けない）。
 *
 * # 識別子のキーは**無い**
 *
 * `handle` は不変（ADR-0016 §1）。**`deny_unknown_fields` なので、
 * 混ぜると 400 になって1列も書かれない。**
 */
export interface UpdateCharacterRequest {
  display_name?: string;
  base_persona?: string;
  /** **3値。** `null` は「消す」。**「変えない」はキーごと省く。** */
  output_format_instruction?: string | null;
  stat_scale?: number;
  /**
   * 変容の走らせ方（ADR-0045 / 作業単位 A-8）。**形式指示と同じ3値** ——
   * 省けば変えない、`null` で未設定へ戻す、`off` / `shadow` / `on` で設定。
   * **送り手は `CharacterForm.svelte` の編集**（触ったときだけ）と
   * **`CharacterRepairForm.svelte`**（修復 / 作業単位 D-15）。
   */
  stat_drift_mode?: string | null;
  /**
   * 見いだす枠の数（ADR-0046 §決定1 / 作業単位 A-9a）。0〜4（5 以上は 400）。
   * **送り手はまだ居ない**（画面は第2段）。`stat_scale` と同じく、変えないならキーごと省く。
   */
  axis_discovery_slots?: number;
  /**
   * **しまう（`true`）／戻す（`false`）**（ADR-0049 §決定1 / 作業単位 A-10）。変えないならキーごと省く
   * （`null` も「変えない」だが、`stat_scale` と同じく送らない）。
   * **送り手は `CharacterList.svelte`** の「しまう」と、しまった一覧の「戻す」。
   */
  archived?: boolean;
}

/**
 * `PATCH /admin/characters/{handle}` の応答（`admin.rs` の `UpdatedCharacterView`）。
 *
 * **`character` が `null` でも失敗ではない**（更新は成功していて、
 * 読み直せなかっただけ）。**理由と次の一手は `admin.rs` の
 * `UpdatedCharacterView` の doc が持つ**——ここに写さない。
 */
export interface UpdatedCharacterView {
  /** 更新した行の主キー。**必ず入る。** */
  id: number;
  /** 更新後の姿。**`null` は「読み直せなかった」。** */
  character: CharacterView | null;
}

/**
 * 記憶の**主語**（`admin.rs` の `subject_to_wire` / `subject_of` の綴り）。
 *
 * **省略しない・落とさない**（絶対制約）。`user` / `character` の区別を落とすと
 * **一人称が混線する** —— 「猫が好き」が誰のことなのか分からなくなり、
 * 訂正版を書くときに主語ごと取り違える。
 *
 * **⚠ これも手で写した型である。** サーバが将来この2つ以外を返しても、
 * TypeScript は何も言わない（`api.ts` 冒頭の ⚠）。**画面は生の値も併記する**
 * （`MemoryList.svelte` の `data-subject`）ので、外れたときに気付ける形にしてある。
 */
export type MemorySubject = "user" | "character";

/**
 * 読み出せた記憶1件（`admin.rs` の `MemoryView`）。
 *
 * **`locked` は載っていない**（ADR-0022 §4 —— 意味が未確定なので API に出さない）。
 * **`superseded_by` も載っていない** —— `memories` に並ぶのは**有効な行だけ**だから
 * （`store::memory_repair` の `SELECT_ACTIVE_MEMORY_ROWS`）。
 * **つまり「`memories` に出ている行は、いま想起される行」である。**
 *
 * **⚠ 上書きされた行は別の節に並ぶ**（`SupersededMemoryView` /
 * `?include=superseded`）。あちらは**この型を包んでいて**、`superseded_by` が1つ増える。
 */
export interface MemoryView {
  /** 主キー。**訂正のパスに貼る値。** */
  id: number;
  subject: MemorySubject;
  content: string;
  /** 未設定は `null`（キーごと落とさない）。 */
  category: string | null;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * **読み出せなかった記憶**（`admin.rs` の `UnreadableMemoryView`）。
 *
 * **これを捨てないこと**（ADR-0022 §2）。捨てると
 * **チャットと抽出を止めている当の行が、修復の入口から消える。**
 * `id` は必ず訂正のパスに貼れる（記憶の主キーは `parse` を通らない）。
 */
export interface UnreadableMemoryView {
  id: number;
  /** 読めなかった列（DB の列名と同じ綴り）。 */
  invalid_columns: string[];
}

/**
 * `GET .../memories?include=` に足せるもの（`admin.rs` の `MemoryListInclude`）。
 *
 * # ⚠ **これはサーバの綴りの写しである**
 *
 * 写し元は `enum MemoryListInclude` で、`#[serde(rename_all = "snake_case")]` が
 * 付いているので**列挙子の名前を snake_case にしたものが線に乗る綴り**になる。
 * **サーバは知らない綴りを 400 で落とす**（`deny_unknown_fields` と列挙）ので、
 * ここがずれると**画面は節を1つも受け取れず、一覧ごと失敗する。**
 *
 * **見張り番は `tests/memory-chain.test.ts::「?include= の綴りは写し元と一致する」`**
 * ——`src/server/admin.rs` を実際に読んで突き合わせる（**植えたソースの対照つき**）。
 * **jsdom では `import.meta.url` が `file:` にならない**ので、
 * ソースを読む側は `node` で走る測定に置いてある。
 *
 * **⚠ 上限（`RECENT_SUPERSEDED_MEMORIES`）は写していない。**
 * 応答の `limit` を出す（`DriftView.recent_limit` と同じ立場）。
 */
export const MEMORY_LIST_INCLUDES = ["superseded"] as const;

/** `?include=` に渡せる値。**`MEMORY_LIST_INCLUDES` の要素だけ。** */
export type MemoryListInclude = (typeof MEMORY_LIST_INCLUDES)[number];

/**
 * **上書きされた記憶1件**（`admin.rs` の `SupersededMemoryView`）。
 *
 * 有効な行と**同じ形**で、`superseded_by` が1つ増えるだけ（サーバ側も
 * `MemoryView` を包んでいて、列を書き写してはいない）。
 */
export interface SupersededMemoryView extends MemoryView {
  /**
   * **どの行に置き換わったか。**
   *
   * **落とさないこと。** 落とすと鎖が辿れず、残るのは「消えた記憶の山」でしかなくなる
   * ——「以前はXだったが今はY」の **Y へ辿る唯一の手掛かり**である。
   */
  superseded_by: number;
}

/**
 * **上書きされた、しかも読み出せなかった記憶1件**
 * （`admin.rs` の `UnreadableSupersededMemoryView`）。
 *
 * **捨てないこと。** 読み飛ばすと**壊れた環だけが鎖から落ち、`total` にだけ残る**
 * ——読み手には数が合わないとしか見えない。
 * **`superseded_by` は壊れた行でも載る**（`parse` を通さない列なので、
 * **鎖はここで切れない**）。
 */
export interface UnreadableSupersededMemoryView extends UnreadableMemoryView {
  superseded_by: number;
}

/**
 * **上書きされた記憶の節**（`admin.rs` の `SupersededListView` / ADR-0022）。
 *
 * 並ぶのは**想起の条件（`superseded_by IS NULL`）から外れた行**——
 * つまり**二度と想起に載らない行**である。**もう訂正もできない**
 * （サーバは `memory_already_superseded` で拒む）。**読むためだけの記録。**
 *
 * # ⚠ **`total` は返した件数ではない**
 *
 * **返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく**
 * （`admin.rs` の `RECENT_SUPERSEDED_MEMORIES` の doc / `DriftView.total_votes` と同じ）。
 * **`total - (memories.length + unreadable.length)` が「見えていない行」。**
 * **読めなかった行も総数に入る。**
 *
 * # ⚠ **この型は主張であって、保証ではない**（このファイル冒頭の ⚠）
 *
 * **欄名がずれてもサーバは 400 を返さない**（`?include=` の綴りと違って、
 * 応答側は誰も検証していない）。**素直に読むと、こう壊れる** ——
 * D-3c のレビューで5通り実測:
 *
 * | 欠けた欄 | 素直に読んだときの壊れ方 |
 * |---|---|
 * | `memories` / `unreadable` | **画面全体が「読み込み中…」で固まる**（`$derived` の中で `TypeError`）。**修復の入口ごと閉じる** |
 * | `total` | 「**全　件…すべて出しています**」——**空白のまま「これで全部」と言い切る** |
 * | `limit` | 「上限は **undefined** 件」 |
 * | 行の `superseded_by` | 「id **undefined** に置き換わりました」——**鎖が黙って嘘をつく** |
 *
 * **したがって読む側（`memoryRepair.ts`）が `?? []` と `null` に落とし、
 * 「応答にありませんでした」と書く。**
 * 見張り番は `tests/memory-superseded.test.ts::「応答の欄が欠けたとき」`（画面の見え方）と
 * `tests/memory-chain.test.ts::「応答の欄が欠けたとき」`（組み立て）。
 */
export interface SupersededListView {
  /**
   * 読めた行。**新しい順**（`created_at` の降順、同着は `id` の降順）。
   *
   * **並びを決めているのは `store` の1文**（`Store::list_superseded_memories`）。
   * **画面でも並べ替えないこと** ——同じ「新しい順」という言葉が2つの意味を持つ。
   *
   * **並ぶのは「訂正した時刻」ではない。** いつ上書きされたかは保存していないので、
   * **その記憶が書かれた時刻**である。
   */
  memories: SupersededMemoryView[];
  /** **読み出せなかった行。** 上書き済みでも指し示す。 */
  unreadable: UnreadableSupersededMemoryView[];
  /** **保存されている上書き済みの行の総数。** 返した件数**ではない。** */
  total: number;
  /** この節に載る上限。**`total` との差が「見えていない行」。写しを持たないこと。** */
  limit: number;
}

/** `GET /admin/characters/{handle}/memories` の応答（`admin.rs` の `MemoryListView`）。 */
export interface MemoryListView {
  /** どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。 */
  character: string;
  memories: MemoryView[];
  unreadable: UnreadableMemoryView[];
  /**
   * **上書きされた記憶**（`?include=superseded` を付けたときだけ）。
   *
   * # ⚠ **「訊いていないから無い」と「訊いたが1件も無い」は別**
   *
   * `unreadable` と違って、**訊かなければキーごと来ない**（サーバの
   * `skip_serializing_if = "Option::is_none"`）。**空なら
   * `{"memories": [], "unreadable": [], "total": 0, "limit": …}` が来る。**
   *
   * **⚠ したがって `undefined` を「0 件」と読まないこと。**
   * 読むと、**問い合わせが届いていない画面が「上書きされた記憶はありません」と
   * 言い切る**（いちばん質の悪い嘘）。`MemoryList.svelte` は
   * **節そのものが無いことを、そう書いて出す。**
   */
  superseded?: SupersededListView;
}

/**
 * `POST /admin/characters/{handle}/memories/{memory_id}/corrections` の要求
 * （`admin.rs` の `CorrectMemoryRequest`）。
 *
 * **⚠ サーバは `deny_unknown_fields`。** ここに無いキーを送らないこと。
 *
 * **`superseded` は本文に持たない** —— 訂正する相手はパスが決める。
 * **`locked` も受け付けない**（ADR-0022 §4）。
 *
 * **`content` は省略できない**（後継の無い取り消しを作らない / ADR-0022 §1）。
 * 「この記憶を無かったことにする」だけの要求は、**型として組み立てられない。**
 */
export interface CorrectMemoryRequest {
  subject: MemorySubject;
  content: string;
  /** 省略は「未設定」。**省くこと**（`undefined` を明示的に入れない）。 */
  category?: string;
}

/**
 * 訂正の応答（`admin.rs` の `CorrectionView`）。**id を2つ返す。**
 *
 * **`superseded_id` の行は消えていない。** 想起（`superseded_by IS NULL`）から
 * 外れただけで、本文も分類も残る（時系列保持 / 絶対制約）。
 */
export interface CorrectionView {
  character: string;
  /** **想起から外れた**行。 */
  superseded_id: number;
  /** 新しく書かれた行。**次に直すときはこの id を指す。** */
  correction_id: number;
}

/**
 * 属性の**主語**（作業単位 D-12a）。
 *
 * **綴りの表は記憶と同じ**（`admin.rs` の `subject_of` / `subject_to_wire` は
 * 属性と記憶で共有されている）。**別の型を作って写しを2つ持たない。**
 *
 * **省略しない・落とさない**（絶対制約 / ADR-0028 §決定4）——
 * **属性は「削れない枠」に毎ターン載る**ので、主語を取り違えると
 * **そのキャラクターの以後の全ターンが、利用者の事実を自分の事実として喋る。**
 */
export type AttributeSubject = MemorySubject;

/**
 * 1件分の属性（`admin.rs` の `AttributeView`）。
 *
 * **`subject` を必ず持つ** ——「年齢: 19」が誰のものかを、
 * 応答を見た側が判断できるように（`admin.rs` の `AttributeView` の doc）。
 */
export interface AttributeView {
  subject: AttributeSubject;
  key: string;
  value: string;
}

/** `GET /admin/characters/{handle}/attributes` の応答（`admin.rs` の `AttributesView`）。 */
export interface AttributesView {
  /** どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。 */
  character: string;
  /** 属性の一覧。**1件も無ければ空の配列**（キーごと落ちない）。 */
  attributes: AttributeView[];
}

/**
 * `PUT /admin/characters/{handle}/attributes` の要求
 * （`admin.rs` の `ReplaceAttributesRequest`）。
 *
 * # ⚠ **この配列が全体である。含まれない属性は消える**
 *
 * **`PATCH` の「キーの有無が意味を持つ」とは正反対**で、
 * **「変えない」を表す手段が無い。** 空の配列を渡すと**全属性が消える。**
 *
 * **⚠ したがって、画面は必ず全件を送ること**
 * （`attributesEdit.ts` の `buildReplaceAttributes`）。
 * 差分だけ送る `buildUpdate` の作法をここへ持ち込むと、
 * **送らなかった属性が黙って消える。**
 *
 * **⚠ サーバは `deny_unknown_fields`。** ここに無いキーを送らないこと。
 */
export interface ReplaceAttributesRequest {
  attributes: AttributeRequest[];
}

/**
 * 1件分の属性の指定（`admin.rs` の `AttributeRequest`）。
 *
 * **`subject` は `AttributeSubject`** —— サーバは知らない綴りを
 * `attribute_subject_unknown`（400）で拒む（`assistant` の取り違えが現実に起きるので、
 * 専用の理由が付いている）。
 *
 * **キーも値も正規化しないで送る** ——`AttributeKey::parse` /
 * `AttributeValue::parse` がサーバ側で行う。
 */
export interface AttributeRequest {
  subject: AttributeSubject;
  key: string;
  value: string;
}

/**
 * 属性の変更1件（`admin.rs` の `AttributeRevisionView` / ADR-0028 §決定5）。
 *
 * # `before` / `after` の `null`
 *
 * | | `before` | `after` |
 * |---|---|---|
 * | 追加 | `null`（**無かった**） | 値 |
 * | 変更 | 値 | 値 |
 * | 削除 | 値 | `null`（**消えた**） |
 *
 * **⚠ 2つを同じ見え方にしないこと** —— 潰すと**追加と削除の区別が消える。**
 * 綴りは `attributesEdit.ts` の `beforeLabel` / `afterLabel` が1箇所で決める。
 *
 * **⚠ `subject` を落とさない**（ADR-0028 §決定4）——
 * 落とすと「年齢が 19 から 20 になった」が誰の話なのか読めない。
 */
export interface AttributeRevisionView {
  /** 主キー。**`sqlite3` で同じ行を指せる唯一の手掛かり**（1件を指すパスは無い）。 */
  id: number;
  subject: AttributeSubject;
  key: string;
  /** 変更前の値。`null` は**その属性が無かった**こと。 */
  before: string | null;
  /** 変更後の値。`null` は**消えた**こと。 */
  after: string | null;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * `GET /admin/characters/{handle}/attributes/revisions` の応答
 * （`admin.rs` の `AttributeRevisionsView` / 作業単位 A-3a）。
 *
 * # **これが無いと「属性が静かに変わったこと」に気付けない**
 *
 * 属性は削れない枠に毎ターン載るので、変わると以後の全ターンに効く。
 * **`GET .../attributes` が答えるのは「いまどうなっているか」だけ**で、
 * **いつ何から何に変わったかは、この応答にしか無い。**
 *
 * # ⚠ **`total_revisions` は `revisions` の件数ではない**
 *
 * **返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく**
 * （`DriftView.total_votes` / `SupersededListView.total` と同じ）。
 * **`total_revisions - revisions.length` が「見えていない件数」。**
 *
 * **⚠ `revisions_limit` を写さないこと。** 上限はサーバが決めて、応答に載せてくる。
 * 写すと**サーバ側の定数を変える変異が画面側で緑を通る。**
 */
export interface AttributeRevisionsView {
  /** どのキャラクターのものか。 */
  character: string;
  /** **新しい順。** ここで並べ替えないこと（並びは `store` の1文が決める）。 */
  revisions: AttributeRevisionView[];
  /** **保存されている変更の総数**（上限で切られていない）。 */
  total_revisions: number;
  /** `revisions` に載る上限。**`total_revisions` との差が「見えていない件数」。** */
  revisions_limit: number;
}

/**
 * 取り決めの種類（`admin.rs` の `AgreementView.kind` / ADR-0051）。
 *
 * **綴りの正は `src/domain/agreement.rs` の `AgreementKind::label`**
 * （見張り番は `tests/agreements.test.ts`）。
 */
export type AgreementKind = "promise" | "rule";

/**
 * 取り決めを**守る側**（`admin.rs` の `AgreementView.keeper`）。**落とさない。**
 *
 * **記憶・属性の主語（`MemorySubject`）とは別の型** —— 「両方」がある。
 */
export type AgreementKeeper = "user" | "character" | "both";

/**
 * 取り決めの状態（`admin.rs` の `AgreementView.state`）。**本文に載るのは `active` だけ。**
 *
 * **⚠ 応答の `enum` は「いま返しうる値」**（ADR-0047）。知らない綴りは画面でそのまま出す
 * （`agreements.ts` の `agreementStateLabel`）。
 *
 * **`proposed`（提案中）と `rejected`（却下）は作業単位 A-12c-1 で増えた**（API 1.3）。
 * 提案は裏方が毎往復書く。提案中は本文に載らず、有効な件数の上限にも数えない。
 * **画面は作業単位 A-12c-2**: 提案中は約束・決まりの画面の「提案」の欄と、会話の画面の問いかけに出す。
 * 却下は記録の側にだけ出す（**どちらも「終わったもの」の側に出さない** —— `agreements.ts` の `endedAgreements`）。
 */
export type AgreementState =
  | "proposed"
  | "active"
  | "rejected"
  | "fulfilled"
  | "broken"
  | "withdrawn";

/** 終わらせ方（`admin.rs` の `EndAgreementRequest.state`）。**`active` は送れない。** */
export type AgreementEnding = "fulfilled" | "broken" | "withdrawn";

/** 取り決め1件（`admin.rs` の `AgreementView` / 作業単位 A-12a）。 */
export interface AgreementView {
  id: number;
  content: string;
  kind: AgreementKind;
  keeper: AgreementKeeper;
  state: AgreementState;
  /** 生まれた会話。**手で書いたものは `null`。** */
  source_session_id: number | null;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * `GET /admin/characters/{handle}/agreements` の応答（`admin.rs` の `AgreementsView`）。
 *
 * **⚠ `active_limit` を写さないこと。** 上限はサーバが決めて、応答に載せてくる
 * （写すとサーバ側の定数を変える変異が画面側で緑を通る）。
 */
export interface AgreementsView {
  character: string;
  /** 書いた順。**有効なものも終わったものも。** */
  agreements: AgreementView[];
  /** **有効な取り決めの件数の上限。** 達したら書く欄を止める。 */
  active_limit: number;
  /**
   * **直近のターンで本文に載った件数と落ちた件数**（`admin.rs` の `AgreementTurnView`）。
   *
   * **この起動でまだターンが無ければ `null`**（サーバは DB に持たない）。
   * `dropped > 0` なら、そのターンでは古い側の取り決めが本文に無かった（予算に入りきらなかった）。
   */
  last_turn: AgreementTurnView | null;
}

/** 直近のターンの取り決めの載り方（`admin.rs` の `AgreementTurnView` / 作業単位 A-12a）。 */
export interface AgreementTurnView {
  /** 本文に載った件数（新しい側から）。 */
  included: number;
  /** 予算に入りきらずに落ちた件数（古い側から）。 */
  dropped: number;
  /** そのターンの時刻（RFC 3339）。 */
  at: string;
}

/**
 * `POST /admin/characters/{handle}/agreements` の要求（`admin.rs` の `CreateAgreementRequest`）。
 *
 * **⚠ サーバは `deny_unknown_fields`。** 状態や出どころを送らないこと（書いたものは有効で始まる）。
 */
export interface CreateAgreementRequest {
  content: string;
  kind: AgreementKind;
  keeper: AgreementKeeper;
}

/** 終わらせる要求（`admin.rs` の `EndAgreementRequest`）。 */
export interface EndAgreementRequest {
  state: AgreementEnding;
}

/**
 * **提案を採る**要求（`admin.rs` の `AdoptAgreementRequest` / 作業単位 A-12c-1 で口、A-12c-2 で写した）。
 *
 * **3つとも省ける**（省いた欄は提案のまま。`{}` は「そのまま採る」）。
 * **⚠ サーバは `deny_unknown_fields`。** 状態を送らないこと（採ると必ず有効になる）。
 */
export interface AdoptAgreementRequest {
  content?: string;
  kind?: AgreementKind;
  keeper?: AgreementKeeper;
}

/**
 * 提案を捨てた結果（`admin.rs` の `AgreementRejectedView`）。
 *
 * **取り決めの姿は返らない**（`AgreementEndedView` と同じ理由）。画面は一覧を読み直す。
 */
export interface AgreementRejectedView {
  character: string;
  agreement_id: number;
  /** いつも `rejected`。 */
  state: AgreementState;
}

/**
 * 終わらせた結果（`admin.rs` の `AgreementEndedView`）。
 *
 * **取り決めの姿は返らない**（サーバは中身を読まずに終わらせる）。画面は一覧を読み直す。
 */
export interface AgreementEndedView {
  character: string;
  agreement_id: number;
  state: AgreementState;
}

/**
 * 状態の変化1件（`admin.rs` の `AgreementTransitionView`）。
 *
 * **`before` の `null` は「書かれた」**（無かった → `after`）。潰さない。
 */
export interface AgreementTransitionView {
  id: number;
  agreement_id: number;
  content: string;
  kind: AgreementKind;
  keeper: AgreementKeeper;
  before: AgreementState | null;
  after: AgreementState;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * `GET /admin/characters/{handle}/agreements/transitions` の応答（`admin.rs` の `AgreementTransitionsView`）。
 *
 * **⚠ `total_transitions` は `transitions` の件数ではない**（`AttributeRevisionsView` と同じ）。
 * **`transitions_limit` を写さないこと。**
 */
export interface AgreementTransitionsView {
  character: string;
  /** **新しい順。** ここで並べ替えないこと。 */
  transitions: AgreementTransitionView[];
  total_transitions: number;
  transitions_limit: number;
}

/**
 * 会話1件の姿（`admin.rs` の `SessionSummaryView` / 作業単位 D-12c-1）。**選ぶための手がかり。**
 */
export interface SessionSummaryView {
  /** 主キー。**`aoi@<id>` の `<id>` そのもの**（選んで送ると、この会話の続きになる）。 */
  id: number;
  /** 人間向けの識別名。未設定は `null`（いまは付ける経路が無い）。 */
  label: string | null;
  /** RFC 3339。 */
  created_at: string;
  /** RFC 3339。**並びの権威**（中身のあるものの中で、新しい順）。 */
  updated_at: string;
  /** メッセージの件数（利用者と応答の両方）。 */
  message_count: number;
  /** **最初の利用者の発言の先頭**（サーバが上限で切る）。発言が無ければ `null`。 */
  first_user_message: string | null;
  /** **切ったかどうか。** 文字列の長さからは読めない。 */
  first_user_message_truncated: boolean;
}

/**
 * `GET /admin/characters/{handle}/sessions` の応答（`admin.rs` の `SessionListView`）。
 *
 * # ⚠ **`sessions[0]` が「素の `aoi` で送ったら続く会話」である**
 *
 * **並びはサーバが `Store::latest_session` と同じ `ORDER BY` で決める**
 * （空の会話は後ろへ回る）。**ここで並べ替えないこと** —— 並べ替えた瞬間、
 * 画面が「続ける会話」として見せるものと、本文が書き込まれる会話が分かれる。
 *
 * **`total_sessions` は `sessions` の件数ではない**（`AttributeRevisionsView` と同じ）。
 * **`sessions_limit` を写さないこと。**
 */
export interface SessionListView {
  character: string;
  sessions: SessionSummaryView[];
  /** **保存されている会話の総数**（上限で切られていない）。 */
  total_sessions: number;
  /** `sessions` に載る上限。 */
  sessions_limit: number;
}

/** 発話者の綴り（`admin.rs` の `role_to_wire`）。 */
export type MessageRole = "user" | "assistant";

/**
 * メッセージ1件（`admin.rs` の `MessageView` / 作業単位 D-12c-1）。
 *
 * # ⚠ **画面が見せる本文は `raw_content` から作る**（作業単位 D-12c-2）
 *
 * **`dialogue` / `narration` を読まないこと。** 実行中の返事は生出力を `dialogue.ts` の
 * `split` に通して見せている —— 保存された2列から組み直すと、
 * **同じ返事が、いま届いたときと読み直したときで違って見える**
 * （括弧の有無・強調の印・崩れた回の扱いが、保存の整形と表示の整形で違う）。
 */
export interface MessageView {
  id: number;
  role: MessageRole;
  dialogue: string | null;
  narration: string | null;
  /** **生出力そのもの。** 画面はこれを整える。 */
  raw_content: string;
  /** RFC 3339。 */
  created_at: string;
}

/**
 * `GET /admin/characters/{handle}/sessions/{session_id}/messages` の応答
 * （`admin.rs` の `MessagePageView` / 作業単位 D-12c-1）。
 *
 * **`messages` は古い順。** 次に遡るときは**先頭の `id`** を `before` に渡す。
 * **もっと前があるかは `has_older` で読む**（`total_messages` との差で決めない
 * ——読んでいる間に増えた分で狂う）。
 */
export interface MessagePageView {
  character: string;
  session_id: number;
  messages: MessageView[];
  has_older: boolean;
  /** **そのセッションのメッセージの総数**（上限で切られていない）。 */
  total_messages: number;
  /** このページに適用された上限（サーバの既定）。 */
  limit: number;
}

/**
 * **実測由来の定数と、いまの上流との突き合わせの内訳**
 * （`upstream.rs` の `MeasuredView` / 作業単位 D-9）。
 *
 * # ⚠ **`checked` は内訳の和である**
 *
 * **`checked` だけを見て「全部一致」と読まないこと。**
 * 画面もそう書く（`UpstreamPanel.svelte`）。
 *
 * # ⚠ **ずれの2種類を1つに潰さない**
 *
 * `drifted_other` は「測り直す先が決まっている」、
 * `drifted_absent` は「誰が答えるかまだ分からない」。**調査先がまるで違う。**
 * 足して1つの数にすると、**その違いを画面から復元できない。**
 */
export interface MeasuredView {
  /** 見た行の総数。**下の4つの和。** */
  checked: number;
  /** 測ったときの相手と、いま応答する相手が同じ。 */
  matched: number;
  /** 一覧には在るが、測ったときの相手ではない。 */
  drifted_other: number;
  /** 依頼する綴りが一覧に無い。 */
  drifted_absent: number;
  /** 聞けなかった（上流の停止・別実装・明示設定）。 */
  unanswered: number;
}

/**
 * **この実行ファイルが、その面の上流をどう扱っているか**
 * （`upstream.rs` の `LaunchView` / ADR-0042 §決定3 / 作業単位 F-4b-1）。
 *
 * # ⚠ `answered_by` と混同しないこと
 *
 * **あちらは「上流が何と答えたか」、こちらは「この実行ファイルが何をしたか」。**
 * **どちらも `null` / `external` になりうるが、意味が違う** ——
 * **`loading` ＋ `answered_by: null` は「いま載せている最中」**で、
 * **`external` ＋ `answered_by: null` は「外の上流が黙っている」。**
 * **畳むと、その違いを画面から復元できない。**
 *
 * # ⚠ 置き場所は載らない
 *
 * `log` は `upstream-logs/` の下の**ファイル名だけ**（サーバが絶対パスを落としている）。
 */
export interface LaunchView {
  /**
   * `external` / `loading` / `ready` / `failed` / `stopped`。
   *
   * **綴りを閉じた型にしていない**（`face` と同じ立場）——
   * サーバが6つ目の状態を足した日に、**画面からこの欄が消えるのがいちばん質の悪い壊れ方。**
   */
  state: string;
  /** **載せ始めた時刻**（`loading` のときだけ）。`null` はそれ以外の状態。 */
  since: string | null;
  /**
   * **上流が名乗った名前**（`ready` のときだけ）。**ファイル名だけ。**
   *
   * **`UpstreamView.answered_name` の写しではない** ——
   * **あちらは引き直しのたびに動き、こちらは「載ったと認めた時点の名前」で固定される。**
   */
  answered_name: string | null;
  /** **失敗の理由**（`failed` のときだけ）。 */
  reason: string | null;
  /** **出力の落とし先のファイル名**（`failed` のときだけ）。**置き場所は含まない。** */
  log: string | null;
}

/**
 * **上流1面ぶんの姿**（`upstream.rs` の `UpstreamView`）。
 *
 * # 面を必ず名乗る
 *
 * **鍵（`main` / `sub`）だけでは、1件を取り出して眺めたときに帰属が消える。**
 * サーバは**観測を引くのに使った面そのもの**から `face` を作るので、
 * **鍵と中身が食い違えば、そのまま応答に現れる**——画面はそれを出す。
 */
export interface UpstreamView {
  /**
   * どちらの面か（`main` / `sub`）。
   *
   * **綴りを閉じた型にしていない**（`DriftView.run_mode` と同じ立場）。
   * サーバが3つ目の面を足した日に、**画面からこの欄が消えるのがいちばん質の悪い壊れ方。**
   */
  face: string;
  /**
   * 設定された宛先。**認証情報（userinfo）はサーバ側で `***@` に伏せ、クエリとフラグメントは落としてある**
   * （`http://***@host/…` になることがある / 作業単位 B-71）。
   *
   * **⚠ 実際に叩く URL と1バイト一致するとは限らない。**
   */
  endpoint: string;
  /**
   * **頼んだ名前。** personacore が依頼するモデルの綴り。
   *
   * **上流が名乗った名前は隣の `answered_name`**（作業単位 B-45）。
   */
  model: string;
  /**
   * **答えた名前。** 上流が名乗った id の**ファイル名だけ**。
   *
   * **`null` は「上流は名乗っていない／名指しできない」。**
   *
   * # ⚠ **`model` と食い違うのは異常ではない**
   *
   * **`llama serve` は `model` の欄を無視して、いま載っているモデルで答える**
   * （`C-18` が実測 / 2026-09-17）。**食い違いこそが
   * 「別物が答えている」の唯一の見え方**なので、
   * **2つを1つの欄に畳まないこと。**
   *
   * **ディレクトリは載らない**（`llama-server` の `id` は gguf の絶対パスで、
   * サーバが継ぎ目でファイル名だけにしている）。
   */
  answered_name: string | null;
  /**
   * **上流が申告したコンテキスト長。** `null` は「読めなかった」。
   *
   * # ⚠ **いま使っている予算ではない**
   *
   * personacore が採用している値はここに載らない（写しを作らないため）。
   * **2つが食い違う状態は正常にありうる** ——
   * 引き直しが失敗した回は、予算は現在値を保ち、申告値は「答えなし」になる。
   * **その食い違いこそが読みたいもの。**
   */
  queried_context_tokens: number | null;
  /**
   * **どの読み手が答えたか。** `null` は「答えなし」＝**上流が黙っている。**
   *
   * 上流が停止しても**予算は現在値を保つ**ので、
   * **ここが `null` になることだけが、外から見える手掛かりになる。**
   * **`null` を値と同じ見え方にしないこと**（`upstream.ts` の `answeredByLabel`）。
   *
   * **綴りは閉じた型にしていない**（`face` と同じ理由）。
   */
  answered_by: string | null;
  /**
   * **最後のモデルの照会が、鍵を断られた（401 / 403）か**（ADR-0056 §決定4 / 作業単位 A-13b-1）。
   *
   * `answered_by: null` と組で読む。**画面は「いまの上流」の状態の印と異常の帯で出す**（作業単位 D-12b-2 /
   * `upstreamStatus.ts`。文言は作業単位 A-13b-3 で決めた）。
   */
  unauthorized: boolean;
  /**
   * **最後に照会した時刻**（RFC 3339 / 作業単位 B-66）。**`null` は「まだ照会していない」。**
   *
   * 画面は読み手の時間帯の `HH:MM:SS` で出す（`upstreamStatus.ts` の `checkedAtLabel`。経過は出さない）。
   */
  checked_at: string | null;
  /**
   * **最後の照会の結果**（作業単位 B-66 / Issue origin#94）。
   *
   * `not_checked`（まだ照会していない）/ `answered`（答えた）/ `no_answer`（繋がらない・黙っている）/
   * `no_model_list`（届いたが一覧が読めない —— レビュー是正で `no_answer` から割った）/
   * `refused`（鍵を断られた）/ `key_not_sent`（承認が無いので鍵を送らなかった）。
   * **`answered_by: null` だけでは「照会していない」と「照会したが答えない」が区別できなかった**ので足した。
   * **綴りは閉じた型にしていない**（`face` と同じ理由。写しは `upstreamStatus.ts` の `CHECK_RESULT`）。
   */
  check_result: string;
  /**
   * **鍵を平文の `http://` で別の機へ送る扱い**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
   * **警告が無い面では欄ごと省かれる**（`null` にはならない）。
   *
   * `local_network`（ローカルネットワークへ暗号化せずに送っている）/ `approved`（承認があるので外へ
   * 暗号化せずに送っている）/ `not_approved`（承認が無いので送っていない）。**画面は `not_approved` を異常の帯、
   * ほかの2つをカードの印で出す**（作業単位 A-13b-3 / `upstreamStatus.ts`）。
   * **綴りは閉じた型にしていない**（`face` と同じ理由。写しは `upstreamKey.ts` の `KEY_OVER_PLAIN_HTTP`）。
   */
  key_over_plain_http?: string;
  measured: MeasuredView;
  /**
   * **この実行ファイルが、この面の上流をどう扱っているか**（ADR-0042 §決定3）。
   *
   * **`--upstream` を渡していない起動では、どちらの面も `external`。**
   */
  launch: LaunchView;
}

/**
 * `GET /admin/upstream` の応答（`upstream.rs` の `UpstreamReport` / 作業単位 D-9）。
 *
 * # ⚠ **2つの上流は別物である**
 *
 * `PERSONACORE_SUB_LLM_ENDPOINT` を設定すれば**宛先まで別**になる。
 * **取り違えると、「抽出が死んでいるのに本文の数を見て正常と読む」形になる。**
 */
export interface UpstreamReport {
  /** 本文生成の上流。 */
  main: UpstreamView;
  /** 裏方（抽出・変容）の上流。 */
  sub: UpstreamView;
  /**
   * **この起動が上流プロファイルの見本を書いた先**（作業単位 F-4c / Issue origin#23）。
   *
   * **`null` は「この起動は見本を書いていない」**（既に在った・別の置き場所を指した・書けなかった）。
   * **画面は `null` のとき見本の話をしない**（`UpstreamPanel.svelte`）。
   *
   * **パスが載るのは、エラーではなく 127.0.0.1 の利用者本人への状態表示だから**
   * （`upstream.rs` の `UpstreamReport::profile_sample` の doc）。
   */
  profile_sample: string | null;
  /**
   * **いま選ばれている上流プロファイルの名前**（`upstream.rs` の `UpstreamReport::selected_profile` /
   * 作業単位 F-10b）。
   *
   * 起動時に選んだもの（`--upstream` か覚えた名前）か、最後に受け付けた切り替えの名前。
   * **`null` は「プロファイルを選んでいない」**（環境変数の既定で起きて、まだ切り替えていない）。
   *
   * **起こすボタンはこの名前を `POST /admin/upstream/profile` へ送る**（ADR-0055 §決定3 /
   * `upstreamProfiles.ts` の `profileToStart`）。
   */
  selected_profile: string | null;
}

/**
 * **1つの面の、利用者が `upstreams.json` に書いたままの値**
 * （`profile_editor.rs` の `WrittenFace` / 作業単位 F-4b-3）。
 *
 * # ⚠ **書いた値であって、観測した値ではない**（Issue origin#24 の閉じる条件）
 *
 * `gguf` は**書いたままの綴り**（`~` を含むなら `~` のまま）。
 * **`UpstreamView` の `answered_name` / `queried_context_tokens` は上流が名乗った（観測した）値**で、
 * そちらは絶対パスを落としてある。**2つを同じ表に混ぜないこと**（`UpstreamPanel.svelte`）。
 *
 * # 向きが2つある
 *
 * **Rust 側は応答と依頼で別の型**（`WrittenFace` / `WrittenFaceRequest` / 作業単位 P-2a）。
 * 応答では**9つの欄が必ず載る**（書いていない鍵は `null`、書いていない承認は `false`）。依頼の形は [`WrittenFaceRequest`]。
 *
 * **作業単位 A-13b-1 で、画面の側も2つに割った** —— 応答には鍵の値の代わりに
 * `api_key_set` / `api_key_hint` が載り、依頼にそれを送ると `deny_unknown_fields` で 400 になる。
 */
export interface WrittenFace {
  endpoint: string;
  model: string;
  /** **書いたままのパス。** `null` は「書いていない」（その面は起こさない）。 */
  gguf: string | null;
  /** `null` は「書いていない」。**空の配列は「書いた」**（別のこと）。 */
  args: string[] | null;
  /** `null` は「書いていない」（起こす面では既定の実行ファイル名を使う）。 */
  command: string | null;
  /**
   * **書いたコンテキスト長**（ADR-0054 §決定1 / 作業単位 A-13a-2）。`null` は「書いていない」（上流に聞く）。
   *
   * **書いた値であって、上流が答えた窓（`UpstreamView.queried_context_tokens`）ではない。**
   * 使えるか（応答の余白より大きいか）はサーバの保存の検証が決める。
   */
  context_tokens: number | null;
  /**
   * **鍵を書いてあるか**（ADR-0056 §決定3 / 作業単位 A-13b-1）。**鍵の値は応答に載らない。**
   *
   * 画面は一覧の「鍵あり」と、下書きの鍵の欄（作業単位 A-13b-3）で出す。
   */
  api_key_set: boolean;
  /** **鍵の末尾4文字。** 書いていない・8 文字より短い鍵は `null`。 */
  api_key_hint: string | null;
  /**
   * **書いた承認**（鍵を平文の `http://` でローカルネットワークの外へ送る / ADR-0056 §決定5 / 作業単位 A-13b-3）。
   * 書いていなければ `false`。**実際に送るかの判定ではない**（それは `UpstreamView.key_over_plain_http`）。
   */
  allow_key_over_plain_http: boolean;
}

/**
 * **1つの面の、保存の依頼の形**（`profile_editor.rs` の `WrittenFaceRequest` / 作業単位 A-13b-1 で分けた）。
 *
 * 依頼では `gguf` / `args` / `command` / `context_tokens` を省けるが、**`null` と省略は同じ意味**なので、
 * **画面は5つとも必ず送る**（`upstreamProfiles.ts` の `buildSaveRequest`）。
 *
 * **⚠ `api_key` だけは省略と `null` の意味が違う**（ADR-0056 §決定3）——
 * **書いていない＝いまの鍵を残す / `null`＝消す / 文字列＝置き換える。**
 * 画面は鍵の値を持たないので、鍵の欄に新しい鍵を書いたときと、消すときだけ送る（作業単位 A-13b-3）。
 *
 * **`allow_key_over_plain_http` は、送らない（か `null`）＝今の規則**（宛先が文字列として同じときだけ残す）、
 * **`true`＝承認する、`false`＝外す**（作業単位 A-13b-3）。画面は承認を取った・外したときだけ送る。
 */
export interface WrittenFaceRequest {
  endpoint: string;
  model: string;
  gguf?: string | null;
  args?: string[] | null;
  command?: string | null;
  context_tokens?: number | null;
  api_key?: string | null;
  allow_key_over_plain_http?: boolean | null;
}

/** **1つのプロファイルの、書いたままの値**（`profile_editor.rs` の `WrittenProfile`）。 */
export interface WrittenProfile {
  name: string;
  main: WrittenFace;
  /** **`null` は「書いていない」**——裏方は本文と同じ上流を使う。 */
  sub: WrittenFace | null;
}

/**
 * **何が駄目だったか**（`profile_editor.rs` の `ProblemView`）。
 *
 * **`message` は定数の文面**（画面にそのまま出してよい）。
 * `profile` だけは利用者がファイルに書いた綴り。
 */
export interface ProblemView {
  reason: string;
  message: string;
  profile: string | null;
  face: string | null;
  key: string | null;
}

/**
 * `GET /admin/upstream/profiles` の応答（`PUT` の成功も同じ形 / `profile_editor.rs` の `ProfilesView`）。
 *
 * `state` は `none` / `missing` / `unreadable` / `invalid` / `ok`。
 * **綴りを閉じた型にしていない**（`LaunchView.state` と同じ立場）。
 */
export interface ProfilesView {
  /** **読んでいるファイル。** 利用者の機のパスだが、127.0.0.1 の本人に言う状態表示なので載る。 */
  path: string | null;
  /** **画面から書けるか。** `false` は読むだけ（`PERSONACORE_UPSTREAMS` が指す先／置き場所が無い）。 */
  editable: boolean;
  state: string;
  /** **読んだときの内容の印。** 保存の依頼にそのまま添える（違えば 409）。 */
  revision: string | null;
  problem: ProblemView | null;
  /** **名前の順。** `state` が `ok` 以外なら空。 */
  profiles: WrittenProfile[];
}

/**
 * `PUT /admin/upstream/profiles/{name}` の本文（`profile_editor.rs` の `SaveRequest`）。
 *
 * **`revision` は `GET` の値そのもの**（ファイルが無かったなら `null`）。
 * **`sub` を `null` にすると、そのプロファイルから裏方の面が消える**（本文と同じ上流になる）。
 */
export interface SaveRequest {
  revision?: string | null;
  main: WrittenFaceRequest;
  sub?: WrittenFaceRequest | null;
}

/**
 * `POST /admin/upstream/profile` の本文（`switch.rs` の `SwitchRequest`）。
 *
 * **名前は `upstreams.json` に利用者が書いたもの**（一覧で選んだもの）。
 */
export interface SwitchRequest {
  name: string;
}

/**
 * サーバが返したエラー。
 *
 * **本文は OpenAI 互換の形**（`{"error": {"message", "type", "code"?}}`）だが、
 * **その形で返ってくるとは限らない**——UI のフォールバックに落ちた 404 は
 * 素の本文で返る（`src/server/web.rs`）。読めなければ `null` を持つ。
 *
 * # `details` —— **`error` の中の、`message` / `type` / `code` 以外の欄**（作業単位 F-4b-3）
 *
 * `PUT /admin/upstream/profiles/{name}` の 422 は、**どこが駄目だったか**を
 * `reason` / `reason_message` / `face` / `key` で返す（`error.rs` の `into_response_with`）。
 * **ここで落とすと、理由を欄に結びつけられない。**
 *
 * **載るのは文字列か `null` の欄だけ**（サーバは定数の文字列しか足さない）。
 * それ以外の形の欄は落とす——**読み手が型を1つで扱える**ように。
 */
export class ApiRequestError extends Error {
  readonly status: number;
  readonly kind: string | null;
  readonly code: string | null;
  readonly details: Readonly<Record<string, string | null>>;

  constructor(
    message: string,
    status: number,
    kind: string | null,
    code: string | null,
    details: Readonly<Record<string, string | null>> = {},
  ) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.kind = kind;
    this.code = code;
    this.details = details;
  }
}

/**
 * **エラー本文の、`message` / `type` / `code` 以外の欄**（[`ApiRequestError.details`]）。
 *
 * **読めなければ空。** 形が違う本文を「欄が無い」と読むだけで、失敗の扱いは変えない。
 */
export function errorDetailsOf(text: string): Record<string, string | null> {
  let parsed: unknown = null;
  try {
    parsed = JSON.parse(text) as unknown;
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null || !("error" in parsed)) {
    return {};
  }
  const detail: unknown = (parsed as { error: unknown }).error;
  if (typeof detail !== "object" || detail === null) {
    return {};
  }
  const found: Record<string, string | null> = {};
  for (const [key, value] of Object.entries(detail)) {
    if (key === "message" || key === "type" || key === "code") {
      continue;
    }
    if (typeof value === "string" || value === null) {
      found[key] = value;
    }
  }
  return found;
}

/** エラー本文（`error.rs` の `ErrorBody`）。**`code` は無いときキーごと落ちる。** */
interface ErrorBody {
  error: {
    message: string;
    type: string;
    code?: string;
  };
}

/**
 * エラー本文の中身。**読めたときだけ返る。**
 *
 * **`/v1/` 側（`chat.ts`）も同じ形を受ける**ので、ここから使う。
 * 判定を2つ持つと、片方だけが `code` の扱いを取りこぼす形で古びる。
 */
export interface ApiErrorDetail {
  message: string;
  kind: string;
  code: string | null;
}

/**
 * 本文の文字列を OpenAI 形のエラーとして読む。**読めなければ `null`。**
 *
 * **`null` を「エラーではない」と読まないこと。** 読めなかっただけで、
 * UI のフォールバックに落ちた 404 のように素の本文で返ることがある
 * （`src/server/web.rs`）。
 */
export function parseErrorBody(text: string): ApiErrorDetail | null {
  let parsed: unknown = null;
  try {
    parsed = JSON.parse(text) as unknown;
  } catch {
    return null;
  }
  if (!isErrorBody(parsed)) {
    return null;
  }
  return {
    message: parsed.error.message,
    kind: parsed.error.type,
    code: parsed.error.code ?? null,
  };
}

/**
 * 値が `ErrorBody` の形をしているか。**`as` で押し込まない。**
 *
 * **`message` だけでなく `type` も見る。** `type` を見ないと
 * `{"error":{"message":"x"}}` が通り、**`ApiErrorDetail.kind` が
 * 実行時 `undefined` なのに型は `string`** という、この関数が避けているはずの状態になる
 * （`code` は `#[serde(skip_serializing_if)]` でキーごと落ちるので任意のまま）。
 */
function isErrorBody(value: unknown): value is ErrorBody {
  if (typeof value !== "object" || value === null || !("error" in value)) {
    return false;
  }
  const detail: unknown = (value as { error: unknown }).error;
  return (
    typeof detail === "object" &&
    detail !== null &&
    "message" in detail &&
    typeof (detail as { message: unknown }).message === "string" &&
    "type" in detail &&
    typeof (detail as { type: unknown }).type === "string"
  );
}

/**
 * 繋がらなかったのか、**こちらが中止したのか。**
 *
 * **中止を「接続できませんでした」と言わない。** 自分で落とした結果を
 * サーバの不調として画面に出すと、原因から遠いところで嘘をつくことになる。
 */
function unreachableOrAborted(
  signal: AbortSignal | undefined,
): ApiRequestError {
  if (signal?.aborted === true) {
    return new ApiRequestError("中止しました", 0, null, null);
  }
  return new ApiRequestError(
    "サーバへ接続できませんでした（personacore は動いていますか）",
    0,
    null,
    null,
  );
}

/**
 * GET して JSON を読む。
 *
 * **返り値の形は検証していない**（上の ⚠）。ここが保証するのは
 * 「HTTP が成功し、本文が JSON として読めた」ところまでである。
 *
 * `signal` を渡すと中断できる。**渡した側は、`catch` で
 * `signal.aborted` を先に見ること**——中断は失敗ではないので、
 * 画面に「取得できませんでした」と出すのは誤り。
 */
async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  return sendJson<T>("GET", path, undefined, signal);
}

/**
 * JSON を送って JSON を読む。**`getJson` もここを通る。**
 *
 * **本文の有無で `content-type` を出し分ける。** 本文の無い `GET` に
 * `content-type: application/json` を付けると、中身が無いのに
 * 「JSON を送った」と名乗ることになる。
 *
 * **返り値の形は検証していない**（このファイル冒頭の ⚠）。
 */
async function sendJson<T>(
  method: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const { status, text } = await send(method, path, body, signal);
  try {
    return JSON.parse(text) as T;
  } catch {
    throw new ApiRequestError(
      "応答を JSON として読めませんでした",
      status,
      null,
      null,
    );
  }
}

/**
 * 送って、**本文を読まない**（作業単位 D-7）。
 *
 * # なぜ `sendJson` を使い回さないのか
 *
 * **成功が 204 の入口がある**（`DELETE /admin/characters/{handle}` /
 * `admin.rs` の `delete_character` —— 「返す状態がもう無い」）。
 * `sendJson` は成功した本文を必ず `JSON.parse` するので、
 * **空の本文を「応答を JSON として読めませんでした」という失敗に変えてしまう。**
 * **消えているのに、消せなかったように見える**——いちばん質の悪い嘘になる。
 *
 * **失敗の読み方は `send` に一本化してある**ので、
 * `code` の扱いがこちらだけ古びることは無い。
 */
async function sendNoContent(
  method: string,
  path: string,
  signal?: AbortSignal,
): Promise<void> {
  await send(method, path, undefined, signal);
}

/**
 * 実際に送って、**HTTP の成否までを見る。**
 *
 * 成功したときは本文を**文字列のまま**返す（読み方は呼び出し側が決める）。
 * 失敗したときは `ApiRequestError` を投げる——
 * **`code` を落とさない**（`parseErrorBody` を通す）。
 * 削除の拒否は `code` でしか見分けられない（`characterDelete.ts`）ので、
 * ここで潰すと、その区別が丸ごと消える。
 */
async function send(
  method: string,
  path: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<{ status: number; text: string }> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      headers:
        body === undefined
          ? { accept: "application/json" }
          : {
              accept: "application/json",
              "content-type": "application/json",
            },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      // `exactOptionalPropertyTypes` が効いているので `undefined` は渡せない。
      // `RequestInit.signal` は `null` を受ける（＝中断しない）。
      signal: signal ?? null,
    });
  } catch {
    throw unreachableOrAborted(signal);
  }

  let text: string;
  try {
    text = await response.text();
  } catch {
    // **本文の途中で切れた。** 中断でも回線断でもここへ来る。
    throw unreachableOrAborted(signal);
  }
  if (!response.ok) {
    const detail = parseErrorBody(text);
    if (detail !== null) {
      throw new ApiRequestError(
        detail.message,
        response.status,
        detail.kind,
        detail.code,
        errorDetailsOf(text),
      );
    }
    throw new ApiRequestError(
      `サーバが ${response.status} を返しました`,
      response.status,
      null,
      null,
    );
  }

  return { status: response.status, text };
}

/** `GET /admin/characters`。**読むだけ。** */
export function listCharacters(
  signal?: AbortSignal,
): Promise<CharacterListView> {
  return getJson<CharacterListView>("/admin/characters", signal);
}

/**
 * `GET /admin/upstream` — **上流について観測できたこと**（作業単位 D-9）。
 *
 * # **この口は読むだけ**
 *
 * **観測を書き換える口は無い**（1つの面を指すパスも生やしていない）。
 * 2026-09-07 の「選ばせもしない」は、**差し替えが安くなったので置き換わった**
 * （ADR-0042 §決定5）。**選ぶのは別の口** —— [`switchUpstreamProfile`] で、
 * **選べるのは利用者が `upstreams.json` に書いたプロファイルだけ**である。
 *
 * # 上流が停止していても失敗しない
 *
 * この入口は**「上流が生きているか」を読むため**に在るので、
 * 上流の状態でこちらが落ちない。停止しているときは `answered_by` が `null` になる。
 */
export function getUpstream(signal?: AbortSignal): Promise<UpstreamReport> {
  return getJson<UpstreamReport>("/admin/upstream", signal);
}

/**
 * `POST /admin/upstream/check` — **両方の面の上流に、いま照会する**（作業単位 B-66 / Issue origin#94）。
 *
 * **返るのは `GET /admin/upstream` と同じ形**（照会した後の観測）。本文は送らない。
 * **照会が走っている間に頼んでも、2本目は並ばない** —— サーバは走っている照会を待ち、その結果を返す
 * （409 にはしない / `context::CheckGate` の doc）。上流が黙っていれば、照会の上限（5秒）まで待つ。
 */
export function checkUpstream(signal?: AbortSignal): Promise<UpstreamReport> {
  return sendJson<UpstreamReport>("POST", "/admin/upstream/check", undefined, signal);
}

/**
 * `POST /admin/upstream/profile` — **書いたプロファイルへ、走ったまま差し替える**
 * （ADR-0042 §決定4 / 作業単位 F-4b-2）。
 *
 * **返るのは `GET /admin/upstream` と同じ形**（差し替えた直後の状態）。
 * **起こした面は `loading` で返る** —— 載るのはサーバが背景で待つので、
 * **画面は読み直して状態の移り変わりを見る**（`UpstreamPanel.svelte`）。
 *
 * **進行中に頼むと 409 `upstream_switch_in_progress`**（列に並べない）。
 * 読み分けはしない。**サーバの文面をそのまま出す。**
 */
export function switchUpstreamProfile(
  name: string,
  signal?: AbortSignal,
): Promise<UpstreamReport> {
  const body: SwitchRequest = { name };
  return sendJson<UpstreamReport>(
    "POST",
    "/admin/upstream/profile",
    body,
    signal,
  );
}

/**
 * `GET /admin/upstream/profiles` — **`upstreams.json` に書いたプロファイルの一覧**（作業単位 F-4b-3）。
 *
 * **失敗しない口**（無い・読めない・検証に通らないは `state` と `problem` が言う）。
 * **値は書いたまま**（`WrittenFace` の doc）。
 */
export function listUpstreamProfiles(
  signal?: AbortSignal,
): Promise<ProfilesView> {
  return getJson<ProfilesView>("/admin/upstream/profiles", signal);
}

/**
 * `PUT /admin/upstream/profiles/{name}` — **1つのプロファイルを足す・直す**（ADR-0042 §決定8）。
 *
 * **返るのは、保存した直後にサーバがファイルを読み直した一覧**（`GET` と同じ形）。
 * **画面はそれを一覧として出す** —— 送った内容ではなく、読み直した内容を見せる。
 *
 * **消す口は無い**（ADR-0042 §決定8）。**生やさないこと。**
 *
 * **名前は必ずエスケープする**（`getStats` と同じ理由。利用者が入力した綴りである）。
 * 失敗の読み分けは `upstreamProfiles.ts` の `describeSaveFailure`。
 */
export function saveUpstreamProfile(
  name: string,
  body: SaveRequest,
  signal?: AbortSignal,
): Promise<ProfilesView> {
  return sendJson<ProfilesView>(
    "PUT",
    `/admin/upstream/profiles/${encodeURIComponent(name)}`,
    body,
    signal,
  );
}

/**
 * `GET /admin/characters/{handle}/stats`。**読むだけ。**
 *
 * **`handle` は必ずエスケープする。** サーバから来た値をそのまま
 * パスへ差し込むと、`/` を含む値でパスが変わる。
 */
export function getStats(handle: string): Promise<StatsView> {
  return getJson<StatsView>(
    `/admin/characters/${encodeURIComponent(handle)}/stats`,
  );
}

/**
 * `PUT /admin/characters/{handle}/stats` — 性格ステータスの**丸ごと置き換え**。
 *
 * # ⚠ **要求に含まれない軸は消える**
 *
 * `store` の `replace_stat_allocs` の意味論そのまま（全消し→挿入）。
 * **空の配列を渡すと全軸が消える。**
 * **`PATCH` のような「変えない」は表現できない**ので、
 * **呼ぶ側は必ず全軸を組み立てること**（`statsEdit.ts` の `buildReplace`）。
 *
 * # 応答は「書いた後に読み直した姿」
 *
 * DB の状態と一致していることを、サーバが読み直して保証している
 * （`admin.rs` の `replace_stats`）。**画面はこれを次の下書きにできる。**
 *
 * # 失敗の読み分けはここでしない
 *
 * 投げるだけ。種別の判定は `statsEdit.ts` の `describeReplaceFailure`（純粋関数）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由。
 * 見張り番は `tests/api-paths.test.ts`）。
 */
export function replaceStats(
  handle: string,
  body: ReplaceStatsRequest,
): Promise<StatsView> {
  return sendJson<StatsView>(
    "PUT",
    `/admin/characters/${encodeURIComponent(handle)}/stats`,
    body,
  );
}

/**
 * `GET /admin/characters/{handle}/drift` — **変容の履歴**（ADR-0024 §9 / D-4a）。
 *
 * **読むだけ。** 票は追記のみで、書くのは本文処理だけである
 * （`admin.rs` の `get_drift` の doc —— **外から書き換えられる記録は記録ではない**）。
 * **書き込みの関数をこのファイルに生やさないこと。** サーバにも経路が無い。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function getDrift(handle: string): Promise<DriftView> {
  return getJson<DriftView>(
    `/admin/characters/${encodeURIComponent(handle)}/drift`,
  );
}

/**
 * `GET /admin/characters/{handle}/attributes` — キャラクター属性の取得（作業単位 D-12a）。
 *
 * **1件も無ければ空の配列。** **1行でも読めなければ 500**
 * （`Store::character_attributes` の方針をそのまま伝える / `admin.rs` の `get_attributes`）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由。
 * 見張り番は `tests/api-paths.test.ts`）。
 */
export function getAttributes(handle: string): Promise<AttributesView> {
  return getJson<AttributesView>(
    `/admin/characters/${encodeURIComponent(handle)}/attributes`,
  );
}

/**
 * `PUT /admin/characters/{handle}/attributes` — キャラクター属性の**丸ごと置き換え**。
 *
 * # ⚠ **要求に含まれない属性は消える**
 *
 * `store` の `replace_character_attributes` の意味論そのまま（全消し→挿入）。
 * **空の配列を渡すと全属性が消える。**
 * **`PATCH` のような「変えない」は表現できない**ので、
 * **呼ぶ側は必ず全件を組み立てること**（`attributesEdit.ts` の `buildReplaceAttributes`）。
 *
 * # 応答は「書いた後に読み直した姿」
 *
 * DB の状態と一致していることを、サーバが読み直して保証している
 * （`admin.rs` の `replace_attributes`）。**画面はこれを次の下書きにできる。**
 *
 * # 履歴は、この経路だけが増やす
 *
 * 変更の記録（`GET .../attributes/revisions`）を書くのは**この `PUT` ひとつ**
 * （ADR-0028 §決定5）。**同じ集合を送り直しても履歴は増えない**（差分ゼロ）。
 *
 * # 失敗の読み分けはここでしない
 *
 * 投げるだけ。種別の判定は `attributesEdit.ts` の
 * `describeReplaceAttributesFailure`（純粋関数）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function replaceAttributes(
  handle: string,
  body: ReplaceAttributesRequest,
): Promise<AttributesView> {
  return sendJson<AttributesView>(
    "PUT",
    `/admin/characters/${encodeURIComponent(handle)}/attributes`,
    body,
  );
}

/**
 * `GET /admin/characters/{handle}/attributes/revisions` — **属性の変更の記録**
 * （ADR-0028 §決定5 / 作業単位 A-3a）。
 *
 * **読むだけ。** 履歴は追記のみで、書くのは `PUT .../attributes` ひとつ。
 * **書き込みの関数をこのファイルに生やさないこと。** サーバにも経路が無い
 * （**1件を指すパス `.../revisions/{id}` すら生やしていない** ——
 * 生やせば `PUT` と `DELETE` が自然な形になり、記憶の側で塞いだ穴が開く）。
 *
 * **1行でも読めなければ 500。** **止まるのはこの一覧だけ**で、
 * 属性そのものの読みも置き換えも通る（`admin.rs` の `get_attribute_revisions`）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function listAttributeRevisions(
  handle: string,
): Promise<AttributeRevisionsView> {
  return getJson<AttributeRevisionsView>(
    `/admin/characters/${encodeURIComponent(handle)}/attributes/revisions`,
  );
}

/**
 * `GET /admin/characters/{handle}/agreements` — **取り決め（約束・決まり）の一覧**（作業単位 A-12a）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 *
 * `signal` は**会話の画面の問いかけ**が、画面を離れたときに読みを落とすために渡す（作業単位 A-12c-2）。
 */
export function listAgreements(
  handle: string,
  signal?: AbortSignal,
): Promise<AgreementsView> {
  return getJson<AgreementsView>(
    `/admin/characters/${encodeURIComponent(handle)}/agreements`,
    signal,
  );
}

/**
 * `POST /admin/characters/{handle}/agreements/{agreementId}/adoption` — **提案を採る**（提案中 → 有効 /
 * 作業単位 A-12c-2）。**採ったものは次のターンから本文に載る。**
 *
 * **有効なものが上限に達していたら 409 `agreement_limit_reached`**、提案中でなければ
 * 409 `agreement_not_proposed`。失敗の読み分けは `agreements.ts` の `describeProposalFailure`。
 *
 * `agreementId` は**サーバが返した `AgreementView.id`**。**`handle` は必ずエスケープする。**
 */
export function adoptAgreement(
  handle: string,
  agreementId: number,
  body: AdoptAgreementRequest,
  signal?: AbortSignal,
): Promise<AgreementView> {
  return sendJson<AgreementView>(
    "POST",
    `/admin/characters/${encodeURIComponent(handle)}/agreements/${agreementId}/adoption`,
    body,
    signal,
  );
}

/**
 * `POST /admin/characters/{handle}/agreements/{agreementId}/rejection` — **提案を捨てる**（提案中 → 却下 /
 * 作業単位 A-12c-2）。**本文は送らない**（サーバは読まない）。**行は消えない**（一覧と記録に残る）。
 *
 * `agreementId` は**サーバが返した `AgreementView.id`**。**`handle` は必ずエスケープする。**
 */
export function rejectAgreement(
  handle: string,
  agreementId: number,
  signal?: AbortSignal,
): Promise<AgreementRejectedView> {
  return sendJson<AgreementRejectedView>(
    "POST",
    `/admin/characters/${encodeURIComponent(handle)}/agreements/${agreementId}/rejection`,
    undefined,
    signal,
  );
}

/**
 * `POST /admin/characters/{handle}/agreements` — **取り決めを手で書く**（201 / 有効で始まる）。
 *
 * **上限に達していたら 409 `agreement_limit_reached`**（何も書かれない）。
 * 失敗の読み分けは `agreements.ts` の `describeAgreementWriteFailure`。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function createAgreement(
  handle: string,
  body: CreateAgreementRequest,
): Promise<AgreementView> {
  return sendJson<AgreementView>(
    "POST",
    `/admin/characters/${encodeURIComponent(handle)}/agreements`,
    body,
  );
}

/**
 * `POST /admin/characters/{handle}/agreements/{agreementId}/transitions` — **取り決めを手で終わらせる。**
 *
 * **行は消えない**（状態が変わり、履歴に1行足される）。**消す関数をこのファイルに生やさないこと**
 * —— サーバにも経路が無い。
 *
 * `agreementId` は**サーバが返した `AgreementView.id`**。**`handle` は必ずエスケープする。**
 */
export function endAgreement(
  handle: string,
  agreementId: number,
  body: EndAgreementRequest,
): Promise<AgreementEndedView> {
  return sendJson<AgreementEndedView>(
    "POST",
    `/admin/characters/${encodeURIComponent(handle)}/agreements/${agreementId}/transitions`,
    body,
  );
}

/**
 * `GET /admin/characters/{handle}/agreements/transitions` — **取り決めの履歴**（**読むだけ**）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function listAgreementTransitions(
  handle: string,
): Promise<AgreementTransitionsView> {
  return getJson<AgreementTransitionsView>(
    `/admin/characters/${encodeURIComponent(handle)}/agreements/transitions`,
  );
}

/**
 * `GET /admin/characters/{handle}/sessions` — **会話の一覧**（作業単位 D-12c-2）。**読むだけ。**
 *
 * **並びを変えないこと**（`SessionListView` の ⚠）。
 * **会話を書く・消す関数をこのファイルに生やさないこと** —— サーバにも経路が無い
 * （会話を書くのは `/v1/chat/completions` だけ）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 */
export function listSessions(
  handle: string,
  signal?: AbortSignal,
): Promise<SessionListView> {
  return getJson<SessionListView>(
    `/admin/characters/${encodeURIComponent(handle)}/sessions`,
    signal,
  );
}

/**
 * `GET /admin/characters/{handle}/sessions/{session_id}/messages` — **1つの会話の1ページ**
 * （作業単位 D-12c-2）。**読むだけ。**
 *
 * `before` を渡すと、その id より古いものを返す（**遡る**）。**件数は送らない**
 * （サーバの既定に任せる —— 画面に件数を写すと、サーバの上限との写しが1つ増える /
 * `tests/api-fields.test.ts` の「会話のページの `?` のキー」）。
 *
 * `sessionId` は**サーバが返した `SessionSummaryView.id`**。数値なのでパスの構造を変えようがないが、
 * **`handle` は必ずエスケープし、問い合わせは `URLSearchParams` に組ませる。**
 */
export function listSessionMessages(
  handle: string,
  sessionId: number,
  before?: number,
  signal?: AbortSignal,
): Promise<MessagePageView> {
  const query =
    before === undefined
      ? ""
      : `?${new URLSearchParams({ before: String(before) }).toString()}`;
  return getJson<MessagePageView>(
    `/admin/characters/${encodeURIComponent(handle)}/sessions/${sessionId}/messages${query}`,
    signal,
  );
}

/**
 * `POST /admin/characters` — キャラクターの**作成**。
 *
 * **作成できるのはここだけ。** 応答は 201 と、作った行を読み直した姿。
 */
export function createCharacter(
  body: CreateCharacterRequest,
): Promise<CharacterView> {
  return sendJson<CharacterView>("POST", "/admin/characters", body);
}

/**
 * `PATCH /admin/characters/{handle}` — 表示名・ペルソナ・形式指示・尺度の**部分更新**。
 *
 * **`PUT` は無い**（ADR-0016 §1 / サーバは 405 を返す）。**生やさないこと。**
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由 ——
 * **サーバから来た値をそのままパスへ差し込まない**）。
 *
 * **⚠ ここへ来る値は、いまは画面の入力欄から来ない。**
 * 編集の画面に識別子の欄は無く（不変 / ADR-0016 §1）、
 * 渡るのは **`CharacterView.handle`**（サーバ側で `Handle::parse` を通った値）だけ。
 * **`[a-z0-9_-]` しか通っていないので、実際には1文字も変わらない。**
 * **それでも外さないこと** —— **この関数は誰から呼ばれてもよい形**で、
 * 呼び出し側の性質を根拠にすると、呼び出し側が増えた日に黙って崩れる。
 */
export function updateCharacter(
  handle: string,
  body: UpdateCharacterRequest,
): Promise<UpdatedCharacterView> {
  return sendJson<UpdatedCharacterView>(
    "PATCH",
    `/admin/characters/${encodeURIComponent(handle)}`,
    body,
  );
}

/**
 * `DELETE /admin/characters/{handle}` — キャラクターの**削除**（作業単位 D-7）。
 *
 * # **しまったキャラクターは、中身ごと消える**（作業単位 A-10 / ADR-0049 §決定1）
 *
 * しまってあれば、会話があっても消える（会話・記憶・性格値・属性も全部。控えは残らない）。
 * **画面がこれを送るのは、しまった一覧の「完全に消す」で識別子を打ち込ませた後と、
 * 上の一覧の「削除」（会話の無いキャラクターだけが通る）の2箇所。**
 *
 * # **消せるかどうかは、押してみるまで分からない**
 *
 * **しまっておらず、会話が1件でもあれば拒否される**（ADR-0016 §2）。判定は `store` が削除と
 * 同じ文で不可分に行うので、**事前に数えて確かめる入口は無い**
 * （数えてから消すと、その間に始まった会話が黙って消える /
 * `admin.rs` の `delete_character` の doc）。
 * **「消せるか調べてから押させる」画面を作らないこと。** 原理的に作れない。
 *
 * # 失敗は `code` で見分ける
 *
 * ここは投げるだけで、**種別の判定はしない**。読み分けは
 * `characterDelete.ts` の `describeDeleteFailure`（純粋関数）が持つ。
 *
 * # 成功の判定は **2xx**。**204 を強制していない**
 *
 * **⚠ この関数は状態番号を見ていない。** `send` が 2xx を成功とするだけで、
 * **204 でなければ失敗、という検査はどこにも無い。**
 *
 * **それでよい。** **204 と「本文が空であること」を固定しているのはサーバ側**で、
 * **見張り番は `tests/server_admin.rs`**（`assert_eq!(status, StatusCode::NO_CONTENT, ...)` と
 * `assert!(body.is_empty(), "204 に本文を付けないこと: {body}")` の2行）。
 * **こちらでも数を見ると、その保証の写しが1つ増える** ——
 * **片方を直しても、もう片方が黙って古びる**（CLAUDE.md）。
 * ここは**上流の保証に寄りかかる**という判断であって、**守っているのではない。**
 *
 * # それでも `sendNoContent` を通す理由（**本文を読まない**）
 *
 * `sendJson` は成功した本文を必ず `JSON.parse` するので、
 * **空の本文を「応答を JSON として読めませんでした」という失敗に変えてしまう。**
 * **消えているのに、消せなかったように見える**——**これは実在する欠陥**で、
 * 状態番号の検査とは別の話（`sendNoContent` の doc）。
 *
 * **返り値は無い**——「消した行の姿」を返すと、消えたものを画面に残す誘惑ができる。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由。
 * 見張り番は `tests/api-paths.test.ts`）。
 */
export function deleteCharacter(handle: string): Promise<void> {
  return sendNoContent(
    "DELETE",
    `/admin/characters/${encodeURIComponent(handle)}`,
  );
}

/**
 * `GET /admin/characters/{handle}/memories` — **修復のための記憶一覧**（ADR-0022 §2）。
 *
 * # **これは想起ではない**
 *
 * 想起の入口は `Store::recall_memories` ひとつだけ（ADR-0003）。
 * ここが返すものは**プロンプトに一切載らない。**
 *
 * **`unreadable` を捨てないこと。** 捨てると、チャットと抽出を止めている行が
 * 画面から消える（`MemoryListView` の doc）。
 *
 * # `include` —— **上書きされた記憶も一緒に読む**（作業単位 D-3c）
 *
 * **省略すると、応答に `superseded` の節は来ない**（既定の応答は D-3 から
 * 1バイトも変わらない）。**`"superseded"` を渡したときだけ**、サーバは
 * 上書き済みの行・その総数・上限を足して返す。
 *
 * **綴りは写しなので `MemoryListInclude` を通すこと**（生の文字列を書かない）。
 *
 * **`handle` は必ずエスケープする**（`getStats` と同じ理由）。
 * **問い合わせも同じ** ——`URLSearchParams` に組ませ、手で `?` を繋がない。
 */
export function listMemories(
  handle: string,
  include?: MemoryListInclude,
  signal?: AbortSignal,
): Promise<MemoryListView> {
  const query =
    include === undefined
      ? ""
      : `?${new URLSearchParams({ include }).toString()}`;
  return getJson<MemoryListView>(
    `/admin/characters/${encodeURIComponent(handle)}/memories${query}`,
    signal,
  );
}

/**
 * `POST /admin/characters/{handle}/memories/{memory_id}/corrections`
 * — **訂正版を足して、古い行にそれを指させる**（ADR-0022）。
 *
 * ```text
 * 新しい行を INSERT   →   古い行の superseded_by に、その id を入れる
 * ```
 *
 * **古い行は消えない。** 想起から外れるだけで、本文も分類も作成日時も残る。
 *
 * **⚠ 記憶を消す・書き換える関数をこのファイルに生やさないこと**（絶対制約）。
 * サーバにもその経路は無い（`src/server/mod.rs` のルータの doc /
 * `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`）。
 * **`PUT` も `DELETE` も、1件の記憶を指すパスすら存在しない。**
 *
 * `memoryId` は**サーバが返した `MemoryView.id` / `UnreadableMemoryView.id`**。
 * 数値なのでパスの構造を変えようがないが、**`handle` は必ずエスケープする。**
 */
export function correctMemory(
  handle: string,
  memoryId: number,
  body: CorrectMemoryRequest,
): Promise<CorrectionView> {
  return sendJson<CorrectionView>(
    "POST",
    `/admin/characters/${encodeURIComponent(handle)}/memories/${memoryId}/corrections`,
    body,
  );
}
