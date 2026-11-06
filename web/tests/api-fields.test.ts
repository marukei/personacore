/**
 * **`api.ts` の欄名は `admin.rs` の写しである**（作業単位 B-18 / ADR-0031 §6）。
 *
 * # なぜ見張るのか —— **壊れ方を D-3c のレビューが実測している**
 *
 * ADR-0031 §6 は「**API の型は手で写す。生成しない**」と決め、
 * 「**写しなので Rust 側を変えると黙って古びる**」と自分で書いた。
 * **古びたときに何が起きるかは、推測ではなく実測がある**（D-3c / api.ts の `SupersededListView` の doc）:
 *
 * | サーバが変えた欄 | 画面 |
 * |---|---|
 * | `memories` / `unreadable` | **画面全体が「読み込み中…」で固まる**（`$derived` の中で `TypeError`）。**修復の入口ごと閉じる** |
 * | `total` | 「**全　件…すべて出しています**」——**空白のまま言い切る** |
 * | `superseded_by` | 「id **undefined** に置き換わりました」——**鎖が黙って嘘をつく** |
 *
 * **型検査は1つも捕まえない。** `svelte-check` が見るのは
 * 「TypeScript の中で辻褄が合っているか」だけで、
 * **実際に返ってくる JSON がこの形かどうかは、この検査を入れるまで誰も見ていなかった。**
 *
 * # 範囲 —— **`admin.rs` の `*View` / `*Request` のうち、`api.ts` が写している 31 個の全欄**
 *
 * > **⚠ 写し元の `.rs` は2つある**（作業単位 D-9 で足した）。
 * > **`GET /admin/upstream` の応答型は `src/server/upstream.rs`** にあり、
 * > **`admin.rs` の側にも「ここには無い」と明記されている**
 * > （観測を保持している側と同じファイルに置く、という判断）。
 * > **走査をそちらへも届かせた** —— 一覧は `UPSTREAM_MIRRORED`。
 * > **広げたのはその1ファイルだけで、`src/server/` を舐める形にはしていない**
 * > （`chat.rs` / `error.rs` は写し元が別で、別の見張り番がある）。
 * >
 * > **作業単位 F-4b-3 で `profile_editor.rs` と `switch.rs` にも届かせた**
 * > （書いたプロファイルの一覧・保存・差し替え）。一覧は `PROFILE_EDITOR_MIRRORED` /
 * > `PROFILE_EDITOR_REQUEST_ONLY` / `SWITCH_MIRRORED`。
 * > **⚠ `WrittenFace` はかつて応答と依頼の両方の向きを持っていた**（`PROFILE_EDITOR_BOTH_WAYS` /
 * > `rustFieldsAs` で応答の向きを名乗って読んでいた）。**作業単位 P-2a で依頼の型
 * > `WrittenFaceRequest` を分けた**ので、いまは両方とも1つの向きしか持たない。
 *
 * **欄を選んでいない。選んだのは構造体である。**
 * 「配列だけ」「値を出す欄だけ」と選ぶと、**選ばなかった欄が
 * 『守られていない場所に守ったと書く』の 8 回目になる**（CLAUDE.md）。
 * 1構造体あたり 1〜7 欄しかないので、**選別する理由が費用の側にも無い。**
 *
 * **⚠ 「ずれたら型検査が落ちるから対象外」という欄は無い**（3通り実測した）。
 *
 * | 仕込み | `pnpm run check` |
 * |---|---|
 * | **`admin.rs` の欄名を変える** | **通る**（TypeScript は Rust を読まない。**古びるのは常にこちら側が動いたとき**） |
 * | `api.ts` の `MemoryView.content` を改名 | 落ちる（**読んでいる場所がある**） |
 * | **`api.ts` の `CorrectionView.character` を改名** | **通る**（**その欄を読んでいる場所が1つも無い**） |
 *
 * **`api.ts` 側の改名ですら、読み手が居なければ黙って通る。**
 *
 * ## 対象外（**やっていないことを、やったと書かない**）
 *
 * | 何 | なぜ |
 * |---|---|
 * | ~~`AttributesView` / `AttributeView` / `ReplaceAttributesRequest` / `AttributeRequest` / `AttributeRevisionsView` / `AttributeRevisionView`~~ | ~~**`api.ts` が写していない**（属性の画面がまだ無い）~~ **→ 作業単位 D-12a で写した。**`MIRRORED` に居る |
 * | ~~**いまここに載っているものは1つも無い**~~ | ~~`admin.rs` の `serde` を通る構造体は、`MemoryListQuery`（問い合わせ / 下の `QUERY_ONLY`）を除いて**全部写してある**~~ **→ 作業単位 A-9a で下の行が入った** |
 * | `AxisCandidatesView` / `AxisCandidateView` / `AxisCandidateTransitionView` / `AxisTrialVotesView` | **`api.ts` が写していない**（軸の候補の画面は ADR-0046 の第2段）。`NOT_MIRRORED` に居る |
 * | `error.rs` の `ErrorBody` / `ErrorDetail` | **写し元が `admin.rs` ではない。** `api.ts` 側も無名の入れ子で、`code` の綴りには別の見張り番がある（`character-delete.test.ts` / `character-repair.test.ts`） |
 * | **欄の「型」**（`number` か `string` か） | **欄名と省略可否だけを見る。** 型の写しは ADR-0031 §6 の別件 |
 * | **並び順** | JSON のキーの順序に意味は無い。**集合として突き合わせる** |
 *
 * # `serde` の属性をどう扱ったか
 *
 * **線に乗る綴りを決めているのは `serde` なので、属性を読まずに欄名だけを写すと
 * 集計の道具が本番のパーサより狭くなる**（CLAUDE.md `16l` / D-3c の `includeSpellingsOf` の実績）。
 *
 * | 属性 | 扱い |
 * |---|---|
 * | `#[serde(rename = "…")]` | **そちらが線に乗る綴り**（`error.rs` の `kind` → `type` がこの形） |
 * | `#[serde(rename_all = "…")]` | 構造体全体に効く。**`snake_case` だけ実装し、他の値は落とす**（黙って素通しすると、属性が付いた日にこちらだけ緑になる） |
 * | **`#[serde(rename(serialize = "…"))]` / `#[serde(rename_all(deserialize = "…"))]`** | **向き別の形も読む。** **その構造体の向きに一致する綴りだけ**を採る（合わない向きの指定は線に乗る形を変えないので無視する） |
 * | `#[serde(flatten)]` | **その型の欄が親に混ざる。** 再帰して展開する（`SupersededMemoryView` / `UnreadableSupersededMemoryView`）。TypeScript 側の `extends` がこれに当たる |
 * | `#[serde(skip_serializing_if = "…")]` | **応答では省略可**（`MemoryListView::superseded`）。`?` として突き合わせる |
 * | `#[serde(skip)]` / `#[serde(skip_serializing)]` | **線に乗らない。** 欄ごと落とす |
 * | `#[serde(default)]` | **要求では省略可**（`UpdateCharacterRequest::output_format_instruction` のように `Option` でない型でも） |
 * | `#[serde(deny_unknown_fields)]` | 綴りの一覧には影響しない。**読み飛ばす** |
 *
 * **省略可否は向きで意味が変わる。向きは `#[derive(…)]` から読む**
 * （どちらも無い／両方ある構造体は落とす）。
 *
 * | 向き | 省略可の条件 |
 * |---|---|
 * | 応答（`Serialize`） | **`skip_serializing_if` だけ。** `Option<T>` は `null` として必ず載る（だから `MemoryView::category` は `category: string \| null` であって `category?` ではない） |
 * | 要求（`Deserialize`） | **`#[serde(default)]` があるか、型が `Option<…>` か、のどちらか** |
 *
 * > **⚠ 要求側の「どちらか」は、`default` の綴りだけを見ていた版の訂正である**
 * > （B-18 のレビューが `serde` だけの小さなクレートで実測）。
 * > **`serde` は `Option<T>` の欠落を `#[serde(default)]` 無しでも `None` として受ける。**
 * > 綴りだけを見ていると、**線の上の契約を1バイトも変えない変異が赤になり**、
 * > **その赤を「写しを直す」と読んだ人が、省略可の欄を必須と書き直す経路ができる。**
 *
 * # 走査する側が壊れていないこと（**`15b` の自己成就**）
 *
 * 実ファイルだけを見る対照は、**走査の綴りを壊す変異が、同じ綴りを
 * 走査対象のファイルに書き込む**と1件見つけて緑になる。
 * **だから対照は、テストがべた書きした「植えたソース」を同じ関数へ通す。**
 * **植えたソースを走査の入力から作らないこと**（作った瞬間に同じ穴に戻る）。
 *
 * > **⚠ Rust 側の走査本体は `./support/rust-wire` へ移した**（作業単位 B-20）。
 * > **写しの見張り番が2つになった**ので（`chat-wire.test.ts` が `/v1/` 側を見る）、
 * > **`serde` の解釈を2つ持つと片方だけが属性を取りこぼす形で古びる。**
 * > **中身は1文字も変えていない。植えたソースの対照もこのファイルに残っている**
 * > ——**動いたのは置き場所だけで、この検査の意味は変わっていない。**
 *
 * # 一覧そのものが痩せることを見張る
 *
 * `MIRRORED` が縮めば、その構造体の突き合わせは黙って消える。
 * **`admin.rs` の構造体を全部数えて `MIRRORED ∪ QUERY_ONLY ∪ NOT_MIRRORED` と
 * 一致することを見る**——**新しい `*View` が Rust 側に増えた日に、ここが赤くなる。**
 * `api.ts` 側も同じ形で数える（`MIRRORED ∪ FE_ONLY`）。
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  labelsOf,
  rustFieldsAs,
  rustFieldsOf,
  rustStructNamesOf,
  type WireField,
} from "./support/rust-wire";

/** `admin.rs` と `api.ts` の両方にあり、綴りも同じもの。**この 41 個を突き合わせる。** */
const MIRRORED = [
  "CharacterListView",
  "CharacterView",
  "UnreadableCharacterView",
  "UpdatedCharacterView",
  "CreateCharacterRequest",
  "UpdateCharacterRequest",
  "StatsView",
  "StatAllocView",
  "ReplaceStatsRequest",
  "StatAllocRequest",
  "DriftView",
  "DriftAxisView",
  "DriftVoteView",
  "MemoryListView",
  "MemoryView",
  "UnreadableMemoryView",
  "SupersededListView",
  "SupersededMemoryView",
  "UnreadableSupersededMemoryView",
  "CorrectMemoryRequest",
  "CorrectionView",
  // **属性の6つ**（作業単位 D-12a で写した。それまでは `NOT_MIRRORED` に居た）。
  "ReplaceAttributesRequest",
  "AttributeRequest",
  "AttributesView",
  "AttributeView",
  "AttributeRevisionsView",
  "AttributeRevisionView",
  // **会話履歴の4つ**（作業単位 D-12c-1 で足し、D-12c-2 で写した）。
  "SessionListView",
  "SessionSummaryView",
  "MessagePageView",
  "MessageView",
  // **約束・決まりの8つ**（作業単位 A-12a。サーバと画面を同じ単位で足した）。
  "CreateAgreementRequest",
  "EndAgreementRequest",
  "AgreementsView",
  "AgreementView",
  "AgreementEndedView",
  "AgreementTransitionsView",
  "AgreementTransitionView",
  "AgreementTurnView",
  // **提案を採る・捨てるの2つ**（作業単位 A-12c-1 で足し、A-12c-2 で写した。それまでは `NOT_MIRRORED` に居た）。
  "AdoptAgreementRequest",
  "AgreementRejectedView",
] as const;

/**
 * `admin.rs` にあるが `api.ts` が写していないもの（理由はこのファイルの冒頭の表）。
 *
 * ~~**⚠ いまは空である**（作業単位 D-12a で属性の6つが `MIRRORED` へ移った）。~~
 * **作業単位 A-9a で軸の候補の4つが入った**（下の注記）。
 * **空になっても消さないこと** —— 下の「1つ残らず分類してある」が
 * **この一覧を必ず1回通る形**にしてあるので、
 * 次に「写さないと決めた」構造体ができたときの置き場所がここになる。
 */
const NOT_MIRRORED = [
  // **軸の候補の4つ**（ADR-0046 §決定6 / 作業単位 A-9a / `GET .../axis-candidates`）。
  // **画面は ADR-0046 の第2段**（提案の画面）で、いまは読み手が1人も居ない。
  // **読み手の居ない写しは、ずれても誰も気付けない**（`api.ts` の冒頭「最初の画面が使う分だけ」）ので、
  // 画面を作る作業単位で `MIRRORED` へ移す。
  "AxisCandidatesView",
  "AxisCandidateView",
  "AxisCandidateTransitionView",
  "AxisTrialVotesView",
] as const;

/**
 * **問い合わせパラメータ。** `api.ts` に対応する型は無く、
 * **キーの綴りは `listMemories` が `URLSearchParams` に組ませる短縮記法**にある。
 * だから突き合わせ方だけが違う（下の「`?` のキーの綴り」）。
 */
const QUERY_ONLY = ["MemoryListQuery", "MessagePageQuery"] as const;

/**
 * **写し元が `src/server/upstream.rs` のもの**（作業単位 D-9 / `GET /admin/upstream`）。
 *
 * # なぜ `admin.rs` に無いのか
 *
 * **上流の観測はキャラクターにも `store` にも掛からない**ので、
 * サーバ側は**観測を保持している側と同じファイル**に応答の型を置いている
 * （`admin.rs` のモジュール doc が名指ししている）。
 * **写しの危険はまったく同じ**なので、走査の方をもう1ファイルへ届かせた。
 *
 * **⚠ 広げたのはこの1ファイルだけ。** `src/server/` を丸ごと舐める形にはしない
 * ——**`api.ts` が写していない構造体まで分類を要求される**ことになる
 * （`chat.rs` / `models.rs` / `error.rs` は写し元が別で、別の見張り番がある）。
 */
const UPSTREAM_MIRRORED = [
  "UpstreamReport",
  "UpstreamView",
  "MeasuredView",
  // **起こした上流の面ごとの状態**（ADR-0042 §決定3 / 作業単位 F-4b-1）。
  "LaunchView",
] as const;

/**
 * **写し元が `src/server/profile_editor.rs` のもの**（作業単位 F-4b-3 / `GET` / `PUT /admin/upstream/profiles…`）。
 *
 * **ずれると、書いたプロファイルの一覧が空に見える・保存が 400 で断られる**
 * （依頼の `SaveRequest` は `deny_unknown_fields`）。
 */
const PROFILE_EDITOR_MIRRORED = [
  "ProfilesView",
  "ProblemView",
  "WrittenProfile",
  "SaveRequest",
  // **応答の形**（作業単位 P-2a で依頼の型 `WrittenFaceRequest` と分けた —— いまは `Serialize` だけ）。
  "WrittenFace",
  // **依頼の形**（作業単位 A-13b-1 で画面の側も分けた —— 応答に `api_key_set` / `api_key_hint` が載り、
  // 依頼に送ると `deny_unknown_fields` で 400 になるため）。
  "WrittenFaceRequest",
] as const;

/**
 * **`profile_editor.rs` の依頼だけの型で、`api.ts` に同名の型が無いもの**（作業単位 P-2a）。
 *
 * **作業単位 A-13b-1 で空になった** —— `WrittenFaceRequest` は `api.ts` にも写した
 * （応答の形で依頼の契約を満たせなくなった。鍵の値の代わりに `api_key_set` / `api_key_hint` が載る）。
 * **⚠ かつて（`PROFILE_EDITOR_BOTH_WAYS` の頃）は「依頼の向きの省略可否は、ここでは見ていない」だった。**
 * いまは上の `it.each(PROFILE_EDITOR_MIRRORED)` が省略可否まで突き合わせる。
 */
const PROFILE_EDITOR_REQUEST_ONLY = [] as const;

/** **写し元が `src/server/switch.rs` のもの**（`POST /admin/upstream/profile`）。 */
const SWITCH_MIRRORED = ["SwitchRequest"] as const;

/** `api.ts` にあるが、写し元が `admin.rs` ではないもの。 */
const FE_ONLY = ["ErrorBody", "ApiErrorDetail"] as const;

const ADMIN_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/admin.rs", import.meta.url)),
  "utf8",
);

const UPSTREAM_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/upstream.rs", import.meta.url)),
  "utf8",
);

const PROFILE_EDITOR_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/profile_editor.rs", import.meta.url)),
  "utf8",
);

const SWITCH_RS = readFileSync(
  fileURLToPath(new URL("../../src/server/switch.rs", import.meta.url)),
  "utf8",
);

const API_TS = readFileSync(
  fileURLToPath(new URL("../src/lib/api.ts", import.meta.url)),
  "utf8",
);

describe("欄名は写し元と一致する", () => {
  /**
   * **構造体ごとに1本。** まとめて1本にすると、
   * どの構造体がずれたのかが失敗の名前から分からなくなる。
   */
  it.each(MIRRORED)("`%s` の欄が `admin.rs` と一致する", (name) => {
    expect(labelsOf(tsFieldsOf(API_TS, name))).toEqual(
      labelsOf(rustFieldsOf(ADMIN_RS, name)),
    );
  });

  /**
   * **一覧が痩せたら赤くなる。**
   *
   * `MIRRORED` から1つ落とすと、その構造体の突き合わせは**黙って消える。**
   * `admin.rs` に `*View` が増えたときも、ここで気付く
   * ——「写すか、写さないと決めるか」を必ず1回通る。
   */
  it("`admin.rs` の構造体を1つ残らず分類してある", () => {
    expect(new Set(rustStructNamesOf(ADMIN_RS))).toEqual(
      new Set<string>([...MIRRORED, ...QUERY_ONLY, ...NOT_MIRRORED]),
    );
  });

  it("`api.ts` の型を1つ残らず分類してある", () => {
    expect(new Set(tsInterfaceNamesOf(API_TS))).toEqual(
      new Set<string>([
        ...MIRRORED,
        ...UPSTREAM_MIRRORED,
        ...PROFILE_EDITOR_MIRRORED,
        ...SWITCH_MIRRORED,
        ...FE_ONLY,
      ]),
    );
  });

  /**
   * **対照: 同じ値を2回読んで緑、という形にしない。**
   *
   * 突き合わせの両側が同じ関数から出ていたら、この検査は何も見ていない。
   * **別々のソースを、別々のパーサで読んでいる**ことを、
   * 「Rust 側にしか無い欄」で確かめる。
   *
   * **⚠ 対照を `AttributeView` から `MemoryListQuery` へ移した**（作業単位 D-12a）——
   * **属性の6つは写したので、もう「写し先に無い」の例ではない。**
   * `MemoryListQuery` は**問い合わせパラメータ**なので、
   * `api.ts` に対応する型が原理的に無い（下の `QUERY_ONLY` の doc）。
   */
  it("対照: 写していない構造体は、写し先に存在しない", () => {
    expect(rustFieldsOf(ADMIN_RS, "MemoryListQuery").length).toBeGreaterThan(0);
    expect(() => tsFieldsOf(API_TS, "MemoryListQuery")).toThrow();
  });
});

/**
 * **上流の観測の欄名も写しである**（作業単位 D-9 / `src/server/upstream.rs`）。
 *
 * # ずれると何が起きるか
 *
 * **画面は「上流が黙っているか」を読むために在る。**
 * `answered_by` の綴りがずれれば、**上流が停止していても
 * 画面は `undefined` を出すだけ**になり、**停止が読めなくなる**
 * ——予算は現在値を保つので、他に手掛かりが無い。
 *
 * **⚠ 走査は `admin.rs` の側と同じ関数**（`./support/rust-wire`）。
 * **`serde` の解釈を2つ持たない**（CLAUDE.md「同じ発見を2箇所に書かない」）。
 * **植えたソースの対照も、下の一群がそのまま両方を支えている。**
 */
describe("上流の欄名は写し元と一致する（`src/server/upstream.rs`）", () => {
  it.each(UPSTREAM_MIRRORED)("`%s` の欄が `upstream.rs` と一致する", (name) => {
    expect(labelsOf(tsFieldsOf(API_TS, name))).toEqual(
      labelsOf(rustFieldsOf(UPSTREAM_RS, name)),
    );
  });

  /**
   * **一覧が痩せたら赤くなる。**
   *
   * `upstream.rs` に `serde` を通る構造体が増えた日に、
   * 「写すか、写さないと決めるか」を必ず1回通る。
   */
  it("`upstream.rs` の `serde` を通る構造体を1つ残らず分類してある", () => {
    expect(new Set(rustStructNamesOf(UPSTREAM_RS))).toEqual(
      new Set<string>(UPSTREAM_MIRRORED),
    );
  });

  /**
   * **対照: 2つの `.rs` を、本当に別々に読んでいる。**
   *
   * 片方だけを読んで両方に使う実装だと、この検査は何も見ていない。
   */
  it("対照: `admin.rs` と `upstream.rs` は別のファイルとして読んでいる", () => {
    // **`admin.rs` にしか無い構造体**は、`upstream.rs` からは読めない。
    expect(rustFieldsOf(ADMIN_RS, "AttributeView").length).toBeGreaterThan(0);
    expect(() => rustFieldsOf(UPSTREAM_RS, "AttributeView")).toThrow();
    // **問い合わせの型でも同じ**（写し先を持たない側でも、ファイルは別である）。
    expect(rustFieldsOf(ADMIN_RS, "MemoryListQuery").length).toBeGreaterThan(0);
    expect(() => rustFieldsOf(UPSTREAM_RS, "MemoryListQuery")).toThrow();
    // **その逆も。**
    expect(rustFieldsOf(UPSTREAM_RS, "MeasuredView").length).toBeGreaterThan(0);
    expect(() => rustFieldsOf(ADMIN_RS, "MeasuredView")).toThrow();
  });
});

/**
 * **書いたプロファイルの一覧・保存・差し替えの欄名も写しである**
 * （作業単位 F-4b-3 / `src/server/profile_editor.rs` / `src/server/switch.rs`）。
 *
 * **走査は同じ関数**（`./support/rust-wire`）。広げたのはこの2ファイルだけ。
 */
describe("上流プロファイルの欄名は写し元と一致する（`profile_editor.rs` / `switch.rs`）", () => {
  it.each(PROFILE_EDITOR_MIRRORED)("`%s` の欄が `profile_editor.rs` と一致する", (name) => {
    expect(labelsOf(tsFieldsOf(API_TS, name))).toEqual(
      labelsOf(rustFieldsOf(PROFILE_EDITOR_RS, name)),
    );
  });

  /**
   * **応答の面と依頼の面は、鍵の欄だけが違う**（作業単位 A-13b-1）。
   *
   * 応答は鍵の値の代わりに `api_key_set` / `api_key_hint` を返し、依頼は `api_key` を受ける。
   * **それ以外の綴りは同じ**（画面は一覧の値を下書きにして、そのまま送り返す）。
   * `?` を外した綴りで突き合わせる（省略可否は向きで意味が違う）。
   */
  it("応答の `WrittenFace` と依頼の `WrittenFaceRequest` は、鍵の欄だけが違う", () => {
    const names = (fields: readonly WireField[]) =>
      fields.map((field) => field.name).sort();
    const response = names(tsFieldsOf(API_TS, "WrittenFace")).filter(
      (name) => name !== "api_key_set" && name !== "api_key_hint",
    );
    const request = names(tsFieldsOf(API_TS, "WrittenFaceRequest")).filter(
      (name) => name !== "api_key",
    );
    expect(request).toEqual(response);
    expect(names(rustFieldsOf(PROFILE_EDITOR_RS, "WrittenFaceRequest"))).toEqual(
      names(tsFieldsOf(API_TS, "WrittenFaceRequest")),
    );
  });

  it.each(SWITCH_MIRRORED)("`%s` の欄が `switch.rs` と一致する", (name) => {
    expect(labelsOf(tsFieldsOf(API_TS, name))).toEqual(
      labelsOf(rustFieldsOf(SWITCH_RS, name)),
    );
  });

  /** **一覧が痩せたら赤くなる**（2つのファイルとも）。 */
  it("`profile_editor.rs` の `serde` を通る構造体を1つ残らず分類してある", () => {
    expect(new Set(rustStructNamesOf(PROFILE_EDITOR_RS))).toEqual(
      new Set<string>([...PROFILE_EDITOR_MIRRORED, ...PROFILE_EDITOR_REQUEST_ONLY]),
    );
  });

  it("`switch.rs` の `serde` を通る構造体を1つ残らず分類してある", () => {
    expect(new Set(rustStructNamesOf(SWITCH_RS))).toEqual(
      new Set<string>(SWITCH_MIRRORED),
    );
  });

  /**
   * **対照: 応答の型と依頼の型を、別々の向きで読んでいる**（作業単位 P-2a で型を分けた）。
   *
   * **⚠ かつてはここが「両方の向きを持つ型は、向きを指定しないと読めない」を実ファイルの
   * `WrittenFace` で見ていた。** 型を分けたので実ファイルにその形はもう無い —— その主張は
   * 植えたソースの対照（下の `rustFieldsAs` の一群）が引き続き見ている。
   * **ここでは、同じ5つの鍵が向きで意味を変えることを実ファイルで見る。**
   */
  it("対照: 応答の型と依頼の型は、同じ鍵でも省略可否が違う", () => {
    // **応答は `Option` でも省略可ではない**（`null` として必ず載る）。
    // **鍵の値（`api_key`）は `#[serde(skip)]` で線に乗らない**（作業単位 A-13b-1）。
    // **承認（`allow_key_over_plain_http`）は作業単位 A-13b-3 で両方の向きに入った**（応答は必ず載る・依頼は省ける）。
    expect(labelsOf(rustFieldsOf(PROFILE_EDITOR_RS, "WrittenFace"))).toEqual([
      "allow_key_over_plain_http",
      "api_key_hint",
      "api_key_set",
      "args",
      "command",
      "context_tokens",
      "endpoint",
      "gguf",
      "model",
    ]);
    // **依頼なら、書かなくてよい鍵は省略可になる。**
    expect(labelsOf(rustFieldsOf(PROFILE_EDITOR_RS, "WrittenFaceRequest"))).toEqual([
      "allow_key_over_plain_http?",
      "api_key?",
      "args?",
      "command?",
      "context_tokens?",
      "endpoint",
      "gguf?",
      "model",
    ]);
    // **応答の型は依頼の向きを持たない**（黙って片方を選ばない）。
    expect(() => rustFieldsAs(PROFILE_EDITOR_RS, "WrittenFace", "deserialize")).toThrow();
  });
});

/**
 * **`?` のキーの綴りも写しである**（`admin.rs` の `MemoryListQuery`）。
 *
 * # ⚠ **`?include=` の値の見張り番では届かない**
 *
 * `memory-chain.test.ts` が読んでいるのは **`enum MemoryListInclude` の値**
 * （`superseded`）で、**キーの綴り `include` ではない。**
 * **`MemoryListQuery.include` を `includes` に改名する変異は 479 全緑だった**
 * （B-18 のレビューの実測）。
 *
 * # 壊れ方は「修復の入口ごと閉じる」より広い
 *
 * `MemoryListQuery` は **`#[serde(deny_unknown_fields)]`** なので、
 * ずれると `?include=superseded` が **400** になり、
 * **上書き済みの節どころか記憶の一覧そのものが出なくなる。**
 *
 * # 写し先は「型」ではなく**引数の名前**
 *
 * `listMemories` は `new URLSearchParams({ include })` の短縮記法で組む
 * ——**キーの綴りは引数名そのもの。** だから突き合わせ方だけが他と違う。
 */
describe("`?` のキーの綴りは写し元と一致する", () => {
  it("`listMemories` が組むキーが `MemoryListQuery` の欄と一致する", () => {
    expect(queryKeysOf(API_TS, "listMemories")).toEqual(
      labelsOf(rustFieldsOf(ADMIN_RS, "MemoryListQuery")).map((label) =>
        label.replace(/\?$/u, ""),
      ),
    );
  });

  it("対照: 植えたソースからは、その場のキーを読む", () => {
    const planted = [
      "export function listMemories(",
      "  handle: string,",
      "  plantedKey?: string,",
      "): Promise<void> {",
      "  const query = `?${new URLSearchParams({ plantedKey }).toString()}`;",
      "  return getJson(query);",
      "}",
    ].join("\n");
    expect(queryKeysOf(planted, "listMemories")).toEqual(["plantedKey"]);
  });

  it("短縮記法でない書き方も読む", () => {
    const planted = [
      "export function listMemories(): Promise<void> {",
      '  const query = new URLSearchParams({ first: a, second: b });',
      "  return getJson(query);",
      "}",
    ].join("\n");
    expect(queryKeysOf(planted, "listMemories")).toEqual(["first", "second"]);
  });

  it("対照: 探せていないことを緑にしない", () => {
    // その名前の関数が無い。
    expect(() => queryKeysOf(API_TS, "listNothing")).toThrow();
    // 関数はあるが `URLSearchParams` を組んでいない。
    expect(() =>
      queryKeysOf(
        "export function listMemories(): void {\n  return;\n}",
        "listMemories",
      ),
    ).toThrow();
    // キーが1つも無い。
    expect(() =>
      queryKeysOf(
        "export function listMemories(): void {\n  new URLSearchParams({});\n}",
        "listMemories",
      ),
    ).toThrow();
  });
});

/**
 * **会話のページの `?` のキーも写しである**（`admin.rs` の `MessagePageQuery` / 作業単位 D-12c-2）。
 *
 * # ⚠ **画面は `limit` を送らない**（**そう決めた**）
 *
 * 件数はサーバの既定（`MESSAGE_PAGE_DEFAULT`）に任せる —— **画面に件数を写すと、
 * サーバの既定と上限との写しが1つ増え、上限を下げた日に画面だけが 400 を受ける。**
 * **だから突き合わせは「画面が組むキーは、どれもサーバが知っている」の向きだけ。**
 * `MessagePageQuery` は `deny_unknown_fields` なので、**ずれると遡る要求が全部 400 になる。**
 *
 * **⚠ 逆の向き（サーバのキーを画面が全部組む）は見ていない。** 見ないと決めたのは `limit` 1つで、
 * それを名指しして固定してある（増えたら赤くなる）。
 */
describe("会話のページの `?` のキーは写し元と一致する", () => {
  it("`listSessionMessages` が組むキーは、どれも `MessagePageQuery` の欄である", () => {
    const rust = labelsOf(rustFieldsOf(ADMIN_RS, "MessagePageQuery")).map((label) =>
      label.replace(/\?$/u, ""),
    );
    const sent = queryKeysOf(API_TS, "listSessionMessages");
    expect(sent).toEqual(["before"]);
    for (const key of sent) {
      expect(rust).toContain(key);
    }
    // **送らないと決めたのは `limit` だけ。**
    expect(rust.filter((key) => !sent.includes(key))).toEqual(["limit"]);
  });
});

describe("Rust 側の走査（植えたソースを通す）", () => {
  it("欄と省略可否を読む（応答は `skip_serializing_if` だけが省略可）", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "pub(super) struct Planted {",
      "    /// doc コメントは読み飛ばす。",
      "    plain: String,",
      "    /// `Option` でも `null` として載るので省略可ではない。",
      "    nullable: Option<String>,",
      '    #[serde(skip_serializing_if = "Option::is_none")]',
      "    omitted: Option<u32>,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "nullable",
      "omitted?",
      "plain",
    ]);
  });

  /**
   * **要求は「`#[serde(default)]` があるか、型が `Option<…>` か」。**
   *
   * **`serde` は `Option<T>` の欠落を `default` 無しでも `None` として受ける**
   * （B-18 のレビューが実測）。**綴りだけを見ると、線の上の契約を
   * 1バイトも変えない変異が赤になる。**
   */
  it("要求は `default` か `Option<…>` のどちらかで省略可", () => {
    const planted = [
      "#[derive(Debug, Deserialize)]",
      "#[serde(deny_unknown_fields)]",
      "pub(super) struct Planted {",
      "    required: String,",
      "    #[serde(default)]",
      "    optional: Option<String>,",
      "    /// **`default` が無くても省略できる**（型が `Option`）。",
      "    bare_option: Option<u32>,",
      '    #[serde(default, deserialize_with = "somewhere")]',
      "    also_optional: FieldUpdate<String>,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "also_optional?",
      "bare_option?",
      "optional?",
      "required",
    ]);
  });

  /**
   * **応答側は `Option<…>` でも省略可ではない。**
   *
   * ここを要求側と同じ規則にすると、**`MemoryView::category` が `category?` になり、
   * 「未設定は `null` でキーごと落とさない」という Rust 側の約束が写しから消える。**
   */
  it("応答の `Option<…>` は省略可ではない（`null` として必ず載る）", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      "    nullable: Option<String>,",
      '    #[serde(skip_serializing_if = "Option::is_none")]',
      "    omitted: Option<String>,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "nullable",
      "omitted?",
    ]);
  });

  it("`rename` は、その欄1つにだけ効く", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      '    #[serde(rename = "type")]',
      "    kind: &'static str,",
      "    message: &'static str,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "message",
      "type",
    ]);
  });

  it("`rename_all = snake_case` は構造体全体に効く", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      '#[serde(rename_all = "snake_case")]',
      "struct Planted {",
      "    alreadySnake: String,",
      "    twoWords: String,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "already_snake",
      "two_words",
    ]);
  });

  it("知らない `rename_all` は落とす（黙って素通ししない）", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      '#[serde(rename_all = "camelCase")]',
      "struct Planted {",
      "    two_words: String,",
      "}",
    ].join("\n");

    expect(() => rustFieldsOf(planted, "Planted")).toThrow();
  });

  /**
   * **向き別の形も読む**（B-18 のレビューが実測した素通し）。
   *
   * `key = "…"` の形しか見ないと、**`rename_all(serialize = "camelCase")` は
   * `null` になって throw の分岐へ入らない** ——
   * **`stat_scale` が `statScale` として線に乗るのに、写しはずれたまま緑。**
   */
  it("向き別の `rename_all` も、自分の向きなら落とす", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      '#[serde(rename_all(serialize = "camelCase"))]',
      "struct Planted {",
      "    two_words: String,",
      "}",
    ].join("\n");

    expect(() => rustFieldsOf(planted, "Planted")).toThrow();
  });

  it("向きが合わない `rename_all` は、線に乗る形を変えないので効かない", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      '#[serde(rename_all(deserialize = "camelCase"))]',
      "struct Planted {",
      "    two_words: String,",
      "}",
    ].join("\n");

    // 応答なので `deserialize = …` は無視する（throw もしない）。
    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual(["two_words"]);
  });

  it("向き別の `rename` は、自分の向きの綴りが線に乗る", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      '    #[serde(rename(serialize = "onTheWire", deserialize = "ignored"))]',
      "    original: String,",
      '    #[serde(rename(deserialize = "not_this_one"))]',
      "    untouched: String,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "onTheWire",
      "untouched",
    ]);
  });

  it("向き別の `rename` は、要求なら `deserialize` の側が線に乗る", () => {
    const planted = [
      "#[derive(Debug, Deserialize)]",
      "struct Planted {",
      '    #[serde(rename(serialize = "ignored", deserialize = "on_the_wire"))]',
      "    original: String,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual(["on_the_wire"]);
  });

  it("`flatten` は、その型の欄を親に混ぜる", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Inner {",
      "    inner_one: i64,",
      "    inner_two: String,",
      "}",
      "",
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      "    #[serde(flatten)]",
      "    wrapped: Inner,",
      "    own_field: i64,",
      "}",
    ].join("\n");

    // **`wrapped` は線に乗らない。** 中身が親の欄になる。
    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "inner_one",
      "inner_two",
      "own_field",
    ]);
  });

  it("`skip` された欄は線に乗らない", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      "    shown: i64,",
      "    #[serde(skip_serializing)]",
      "    hidden: i64,",
      "    #[serde(skip)]",
      "    also_hidden: i64,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual(["shown"]);
  });

  it("doc コメントの中の `{` `}` で本文を切らない", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "struct Planted {",
      "    /// FE の `{#each xs as x (x.id)}` に通すと鍵が重複する。",
      "    first: i64,",
      '    /// 空なら `{"a": [], "b": 0}` が来る。',
      "    second: i64,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsOf(planted, "Planted"))).toEqual([
      "first",
      "second",
    ]);
  });

  it("対照: 探せていないことを緑にしない", () => {
    const body = "#[derive(Debug, Serialize)]\nstruct Planted {\n    a: i64,\n}";
    // **その名前の構造体が無い。**
    expect(() => rustFieldsOf(body, "Missing")).toThrow();
    // **欄が1つも無い。**
    expect(() =>
      rustFieldsOf("#[derive(Debug, Serialize)]\nstruct Empty {\n}", "Empty"),
    ).toThrow();
    // **向きが分からない**（`Serialize` も `Deserialize` も無い）。
    expect(() =>
      rustFieldsOf("#[derive(Debug)]\nstruct Planted {\n    a: i64,\n}", "Planted"),
    ).toThrow();
    // **両方ある**（省略可否の意味が1つに決まらない）。
    expect(() =>
      rustFieldsOf(
        "#[derive(Serialize, Deserialize)]\nstruct Planted {\n    a: i64,\n}",
        "Planted",
      ),
    ).toThrow();
    // **読めない行を黙って飛ばさない。**
    expect(() =>
      rustFieldsOf(
        "#[derive(Debug, Serialize)]\nstruct Planted {\n    fn nested() {}\n}",
        "Planted",
      ),
    ).toThrow();
    // **名前の前方一致で拾わない**（`Planted` を探して `PlantedExtra` を読まない）。
    expect(() =>
      rustFieldsOf(
        "#[derive(Debug, Serialize)]\nstruct PlantedExtra {\n    a: i64,\n}",
        "Planted",
      ),
    ).toThrow();
    // **同じ名前が2回**（`#[cfg(test)] mod tests` の中に同名のヘルパが入る形）。
    // 「最初の1つを読む」は選択ではなく偶然なので、黙って選ばない。
    expect(() =>
      rustFieldsOf(
        [
          "#[derive(Debug, Serialize)]",
          "struct Planted {",
          "    a: i64,",
          "}",
          "#[cfg(test)]",
          "mod tests {",
          "    #[derive(Debug, Serialize)]",
          "    struct Planted {",
          "        b: i64,",
          "    }",
          "}",
        ].join("\n"),
        "Planted",
      ),
    ).toThrow();
  });

  /**
   * **向きを指定して読む**（作業単位 F-4b-3 / `WrittenFace` のように両方の向きを持つ型）。
   *
   * **指定した向きの `derive` が無ければ落とす**（応答しか無い型を依頼として読まない）。
   */
  it("向きを指定すると、両方の向きを持つ型をその向きで読む", () => {
    const planted = [
      "#[derive(Debug, Serialize, Deserialize)]",
      "struct Planted {",
      "    plain: String,",
      "    #[serde(default)]",
      "    nullable: Option<String>,",
      "}",
    ].join("\n");

    expect(labelsOf(rustFieldsAs(planted, "Planted", "serialize"))).toEqual([
      "nullable",
      "plain",
    ]);
    expect(labelsOf(rustFieldsAs(planted, "Planted", "deserialize"))).toEqual([
      "nullable?",
      "plain",
    ]);
    const onlyOut = "#[derive(Debug, Serialize)]\nstruct Out {\n    a: Option<i64>,\n}";
    expect(labelsOf(rustFieldsAs(onlyOut, "Out", "serialize"))).toEqual(["a"]);
    expect(() => rustFieldsAs(onlyOut, "Out", "deserialize")).toThrow();
    const onlyIn = "#[derive(Debug, Deserialize)]\nstruct In {\n    a: Option<i64>,\n}";
    expect(() => rustFieldsAs(onlyIn, "In", "serialize")).toThrow();
  });

  it("対照: 構造体の一覧は、植えたソースからその場の名前を読む", () => {
    const planted = [
      "#[derive(Debug, Serialize)]",
      "pub(super) struct PlantedView {",
      "    a: i64,",
      "}",
      "",
      "#[derive(Debug, Deserialize)]",
      "struct PlantedRequest {",
      "    b: i64,",
      "}",
      "",
      "/// `serde` を通らない構造体は、線に乗らないので数えない。",
      "#[derive(Debug, Clone)]",
      "struct NotOnTheWire {",
      "    c: i64,",
      "}",
    ].join("\n");

    expect(rustStructNamesOf(planted)).toEqual([
      "PlantedView",
      "PlantedRequest",
    ]);
    expect(() => rustStructNamesOf("// 何も無い")).toThrow();
    // **重複は `Set` に入れると消える**ので、一覧の側で落とす。
    expect(() =>
      rustStructNamesOf(
        [
          "#[derive(Debug, Serialize)]",
          "struct Twice {",
          "    a: i64,",
          "}",
          "#[derive(Debug, Serialize)]",
          "struct Twice {",
          "    b: i64,",
          "}",
        ].join("\n"),
      ),
    ).toThrow();
  });
});

describe("TypeScript 側の走査（植えたソースを通す）", () => {
  it("欄と省略可否を読む", () => {
    const planted = [
      "export interface Planted {",
      "  /** doc コメントは読み飛ばす。 */",
      "  plain: string;",
      "  nullable: string | null;",
      "  omitted?: number;",
      "}",
    ].join("\n");

    expect(labelsOf(tsFieldsOf(planted, "Planted"))).toEqual([
      "nullable",
      "omitted?",
      "plain",
    ]);
  });

  it("`extends` は、親の欄を混ぜる", () => {
    const planted = [
      "export interface Inner {",
      "  inner_one: number;",
      "  inner_two: string;",
      "}",
      "",
      "export interface Planted extends Inner {",
      "  own_field: number;",
      "}",
    ].join("\n");

    expect(labelsOf(tsFieldsOf(planted, "Planted"))).toEqual([
      "inner_one",
      "inner_two",
      "own_field",
    ]);
  });

  it("コメントの中の `{` `}` `;` で本文を切らない", () => {
    const planted = [
      "export interface Planted {",
      "  /** `{ a: 1 }` のような形が来る。`;` も書ける。 */",
      "  first: number;",
      "  // 行コメントの中の } も同じ。",
      "  second: number;",
      "}",
    ].join("\n");

    expect(labelsOf(tsFieldsOf(planted, "Planted"))).toEqual([
      "first",
      "second",
    ]);
  });

  it("対照: 探せていないことを緑にしない", () => {
    expect(() =>
      tsFieldsOf("export interface Planted {\n  a: number;\n}", "Missing"),
    ).toThrow();
    expect(() =>
      tsFieldsOf("export interface Empty {\n}", "Empty"),
    ).toThrow();
    // **読めない行を黙って飛ばさない。**
    expect(() =>
      tsFieldsOf("export interface Planted {\n  readonly a: number;\n}", "Planted"),
    ).toThrow();
    // **名前の前方一致で拾わない。**
    expect(() =>
      tsFieldsOf("export interface PlantedExtra {\n  a: number;\n}", "Planted"),
    ).toThrow();
    // **同じ名前が2回**（どちらを読んだのか決まらない）。
    expect(() =>
      tsFieldsOf(
        "interface Planted {\n  a: number;\n}\nexport interface Planted {\n  b: number;\n}",
        "Planted",
      ),
    ).toThrow();
  });

  it("対照: 型の一覧は、植えたソースからその場の名前を読む", () => {
    const planted = [
      "export interface PlantedOne {",
      "  a: number;",
      "}",
      "",
      "interface PlantedTwo {",
      "  b: number;",
      "}",
      "",
      "export type NotAnInterface = string;",
    ].join("\n");

    expect(tsInterfaceNamesOf(planted)).toEqual(["PlantedOne", "PlantedTwo"]);
    expect(() => tsInterfaceNamesOf("// 何も無い")).toThrow();
  });
});

/**
 * `api.ts` の型1つを、**線に乗る欄名**で読む。
 *
 * `extends` は `#[serde(flatten)]` に当たるので展開する。
 * **読めない行は throw する**（Rust 側と同じ理由）。
 */
function tsFieldsOf(
  source: string,
  interfaceName: string,
  seen: ReadonlySet<string> = new Set(),
): WireField[] {
  if (seen.has(interfaceName)) {
    throw new Error(`interface ${interfaceName} が自分自身を extends しています`);
  }
  const at = declarationOf(source, interfaceName);
  const openAt = source.indexOf("{", at);
  if (openAt < 0) {
    throw new Error(`interface ${interfaceName} に本文がありません`);
  }

  const heading = source.slice(at + `interface ${interfaceName}`.length, openAt);
  const parent = /^\s+extends\s+([A-Za-z0-9_]+)\s*$/u.exec(heading);
  if (parent === null && heading.trim() !== "") {
    throw new Error(`interface ${interfaceName} の読めない見出し: ${heading}`);
  }

  const fields: WireField[] =
    parent?.[1] === undefined
      ? []
      : tsFieldsOf(source, parent[1], new Set([...seen, interfaceName]));

  for (const member of membersOf(source, openAt)) {
    const found = /^([A-Za-z_][A-Za-z0-9_]*)(\?)?\s*:/u.exec(member);
    if (found?.[1] === undefined) {
      throw new Error(`interface ${interfaceName} の読めない行: ${member}`);
    }
    fields.push({ name: found[1], optional: found[2] === "?" });
  }

  if (fields.length === 0) {
    throw new Error(`interface ${interfaceName} に欄がありません`);
  }
  return fields;
}

/**
 * その関数が `new URLSearchParams({ … })` に渡すキーを、**現れる順に**。
 *
 * **短縮記法（`{ include }`）も `{ first: a }` の形も読む。**
 * 見つからなければ throw（「そもそも探せていないから緑」を作らない）。
 */
function queryKeysOf(source: string, functionName: string): string[] {
  const at = source.indexOf(`function ${functionName}(`);
  if (at < 0) {
    throw new Error(`function ${functionName} が見つかりません`);
  }
  const marker = "new URLSearchParams({";
  const from = source.indexOf(marker, at);
  if (from < 0) {
    throw new Error(`${functionName} が URLSearchParams を組んでいません`);
  }
  const openAt = from + marker.length - 1;
  const close = source.indexOf("}", openAt);
  if (close < 0) {
    throw new Error("閉じ括弧が見つかりません");
  }

  const keys = source
    .slice(openAt + 1, close)
    .split(",")
    .map((entry) => entry.split(":")[0]?.trim() ?? "")
    .filter((entry) => entry !== "");
  for (const key of keys) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/u.test(key)) {
      throw new Error(`読めないキーです: ${key}`);
    }
  }
  if (keys.length === 0) {
    throw new Error(`${functionName} が組むキーが1つもありません`);
  }
  return keys;
}

/** `api.ts` の `interface` の名前を、**現れる順に**全部。 */
function tsInterfaceNamesOf(source: string): string[] {
  const names: string[] = [];
  for (const raw of source.split("\n")) {
    const found = /^(?:export )?interface ([A-Za-z0-9_]+)/u.exec(
      raw.replace(/\r$/u, ""),
    );
    if (found?.[1] !== undefined) {
      names.push(found[1]);
    }
  }
  if (names.length === 0) {
    throw new Error("interface が1つも見つかりません");
  }
  return names;
}

/** `interface <名前>` の宣言の位置。**2つあれば落とす**（どちらを読んだか決まらない）。 */
function declarationOf(source: string, interfaceName: string): number {
  // **行頭であることを見る。** 先頭行も行頭なので、改行を1つ足してから探す
  // （足さないと、1行目の宣言だけが「無い」ことになる）。
  const padded = `\n${source}`;
  const found: number[] = [];
  for (const prefix of ["\nexport interface ", "\ninterface "]) {
    let from = 0;
    for (;;) {
      const at = padded.indexOf(`${prefix}${interfaceName}`, from);
      if (at < 0) {
        break;
      }
      const after = padded.charAt(at + prefix.length + interfaceName.length);
      if (after === " " || after === "{") {
        found.push(at + prefix.length - "interface ".length - 1);
      }
      from = at + 1;
    }
  }
  if (found.length !== 1 || found[0] === undefined) {
    throw new Error(
      `interface ${interfaceName} の宣言が ${found.length} 個あります`,
    );
  }
  return found[0];
}

/**
 * `{` から対応する `}` までを、**深さ 0 の `;` で切った並び**にする。
 *
 * コメントと文字列リテラルの中は数えない（そこの `{` `}` `;` で切ると、
 * doc コメントに `{"a": []}` と書いた日に本文が化ける）。
 */
function membersOf(source: string, openAt: number): string[] {
  const members: string[] = [];
  let current = "";
  let depth = 0;
  let index = openAt;

  const flush = (): void => {
    const member = current.trim();
    if (member !== "") {
      members.push(member);
    }
    current = "";
  };

  while (index < source.length) {
    const pair = source.slice(index, index + 2);
    if (pair === "/*") {
      const end = source.indexOf("*/", index + 2);
      if (end < 0) {
        throw new Error("閉じていないコメントがあります");
      }
      index = end + 2;
      continue;
    }
    if (pair === "//") {
      const end = source.indexOf("\n", index + 2);
      index = end < 0 ? source.length : end + 1;
      continue;
    }
    const char = source.charAt(index);
    if (char === '"' || char === "'" || char === "`") {
      const end = endOfString(source, index);
      current += source.slice(index, end);
      index = end;
      continue;
    }
    if (char === "{") {
      depth += 1;
      if (depth > 1) {
        current += char;
      }
      index += 1;
      continue;
    }
    if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        flush();
        return members;
      }
      current += char;
      index += 1;
      continue;
    }
    if (char === ";" && depth === 1) {
      flush();
      index += 1;
      continue;
    }
    current += char;
    index += 1;
  }
  throw new Error("閉じ括弧が見つかりません");
}

/** 文字列リテラルの終わり（閉じ引用符の次）。 */
function endOfString(source: string, openAt: number): number {
  const quote = source.charAt(openAt);
  let index = openAt + 1;
  while (index < source.length) {
    const char = source.charAt(index);
    if (char === "\\") {
      index += 2;
      continue;
    }
    if (char === quote) {
      return index + 1;
    }
    index += 1;
  }
  throw new Error("閉じていない文字列リテラルがあります");
}
