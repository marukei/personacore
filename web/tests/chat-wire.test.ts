/**
 * **`chat.ts` の写しは `/v1/` 側の Rust の写しである**（作業単位 B-20 / ADR-0031 §6）。
 *
 * # 着手前に測ってある。**数はここに写さない**
 *
 * 「`api.ts` と同じ性質のはず」は推測だったので、**着手前に変異で測った。**
 * **何をどれだけ壊して、どうだったかは [ADR-0031](../../docs/adr/0031-front-end-stack.md) §6 が持つ。**
 * ここに書き写すと、**測り直したときに片方だけが黙って古びる**（CLAUDE.md）。
 *
 * **測って分かったのは1つ** —— **`chat.ts` の写しは「構造体」だけではない**（下の表）。
 *
 * # 何が写しなのか —— **4種類ある**
 *
 * | 写し | 写し元 | ずれると |
 * |---|---|---|
 * | **要求の本体** | `chat.rs` の `ChatCompletionRequest` / `IncomingMessage` | 送った欄が**黙って捨てられる**（`deny_unknown_fields` を付けていない / `chat.rs` の doc）。`stream` が捨てられれば**非ストリーミングの JSON が返り、SSE として読めない** |
 * | **SSE のチャンクの枠** | `relay.rs` の `Relay::relay` | **本文が1文字も出ない。** `readFrame` が `unreadable` に落ちるか、増分が空文字列になる |
 * | **SSE の中のエラーの枠** | `error.rs` の `ApiError::as_event_data` | **理由が画面から消える**（`upstream-error` にならず `unreadable` になる） |
 * | **`model` の綴りの規則** | `selector.rs` の `NEW_SUFFIX` と `split_once('@')` | **「新しい会話を始める」が効かず、黙って前の会話の続きになる**（ADR-0004 がいちばん避けたい混線） |
 *
 * # 範囲の決め方 —— **B-18 の理由をそのまま引き継ぐ**
 *
 * **欄を選ばず、線に乗る単位を選んだ。**
 * 「FE が読む欄だけ」と選ぶと、**選ばなかった欄が
 * 『守られていない場所に守ったと書く』の次の一例になる**（CLAUDE.md）。
 *
 * だから **`Relay::relay` と `as_event_data` が載せる鍵は1つ残らず分類する**
 * ——**FE が読む値 / 通り道の入れ物 / FE が見ない欄**の3つに。
 * **`id` も `object` も `created` も `index` も `finish_reason` も、
 * 改名すれば分類がずれて赤くなる。**
 * 同じく **`ChatCompletionRequest` と `IncomingMessage` の欄も1つ残らず分類する**（送る／送らない）。
 *
 * ## 対象外（**やっていないことを、やったと書かない**）
 *
 * | 何 | なぜ |
 * |---|---|
 * | **非ストリーミングの応答**（`ChatCompletionView` / `ChoiceView` / `MessageView`） | **`chat.ts` は写していない。** `stream: true` しか送らないので、この枠は FE に一度も届かない |
 * | **`GET /v1/models` の応答**（`models.rs`） | 同上。**画面はキャラクターの一覧を `/admin/` から取る** |
 * | **欄の「型」**（`number` か `string` か） | **鍵の綴りと省略可否だけを見る**（B-18 と同じ線引き。ADR-0031 §6 の別件） |
 * | **並び順** | JSON のキーの順序に意味は無い。**集合として突き合わせる** |
 * | **`finish_reason` の値の綴り**（`stop` / `length`） | **`chat.ts` は `finish_reason` を1文字も読まない**（鍵の存在だけを分類する）。**読まない値に写しは無い** |
 * | **エラーの `code` の値の綴り**（`model_not_found` など） | **`chat.ts` は値で分岐しない**（`detail.message` をそのまま出す）。**`/admin/` 側の値には別の見張り番がある**（`character-delete.test.ts` / `character-repair.test.ts`） |
 * | **`Handle` が `@` を含めない約束**（`character.rs` の `raw.contains('@')`） | **写し元が `/v1/` ではない。** `isSessionOf` はこの約束に寄りかかっているが、**寄りかかり先を見張るのは別の作業単位** |
 * | **SSE の枠組みそのもの**（`data:` の綴り・空行の区切り） | **写し元は axum と SSE の仕様**で、personacore ではない。**振る舞いは `chat-outcomes.test.ts` が読み取りの切れ目まで指定して測っている** |
 * | **`accept` ヘッダ** | **サーバは `accept` を1行も読まない**（`src/server/` に参照が1件も無いことを確かめた）。**送っても送らなくても振る舞いが変わらない**ので写しではない |
 * | **省略可否のうち「条件でない条件」** | **走査は波括弧の深さで省略可否を決める。** `if !self.started {` を **`if true {`** にする変異は**ここでは緑**（実測）。**Rust 側に権威が実在する** —— `server::relay::tests::a_relayed_chunk_is_an_openai_compatible_chunk` が**厳密な JSON を固定していて赤くなる**（実測）。**走査を条件まで読む形には広げない** |
 *
 * > **⚠ `content-type` は対象外ではない。写しである**（B-20 のレビューが実測）。
 * > `chat.rs` は **`Json<ChatCompletionRequest>`** で本文を受けるので、
 * > **axum の抽出器が `content-type: application/json` を要求する**
 * > （外すと `JsonRejection` → `ApiError::invalid_body()` で**その回が丸ごと 400**）。
 * > **`accept` と同じ行に置いていたのが誤りだった** ——
 * > 「サーバは読まない」という理由は `accept` にしかかかっていない。
 *
 * # 突き合わせ方 —— **両側を別々の手段で見る**
 *
 * * **Rust 側はソースを読む**（`serde` の属性と `Map::insert` の鍵を解く）。
 * * **TypeScript 側はソースを読まない。本番の関数を実際に走らせる。**
 *   `sendMessage` に `fetch` の身代わりを噛ませ、**送った本体を覗き、
 *   Rust が置いた場所へ植えた目印が FE の手元に出てくるか**を見る。
 *
 * **だから「同じ値を2回読んで緑」にならない。**
 * 片側は静的な走査、もう片側は**本番の入口を通った実行結果**である。
 * TypeScript 側の綴りを変えれば目印が出てこなくなり、
 * Rust 側の綴りを変えれば植える場所が変わって、やはり出てこなくなる。
 *
 * **目印は「その道」そのもの**（`<choices[].delta.content>` のような文字列）にしてある。
 * **全部違う値になる**ので、**どこから読んでも緑**という形にならない
 * （CLAUDE.md の最頻の穴）。**違うことは検査で押さえてある。**
 *
 * # 走査する側が壊れていないこと（**`15b` の自己成就**）
 *
 * 実ファイルだけを見る対照は、**走査の綴りを壊す変異が、同じ綴りを
 * 走査対象のファイルに書き込む**と1件見つけて緑になる。
 * **だから対照は、テストがべた書きした「植えたソース」を同じ関数へ通す。**
 *
 * **jsdom は要らない**（DOM を触らない / 既定の `node` 環境で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  continueLatest,
  continueSession,
  isSessionOf,
  sessionIdOf,
  sendMessage,
  startNew,
  type StreamCallbacks,
  type StreamOutcome,
} from "../src/lib/chat";
import { rustFieldsOf } from "./support/rust-wire";

/** **リポジトリ根からの相対で読む**（`web/` だけ複製すると落ちる先例がある / `B-14`）。 */
function repoSource(relative: string): string {
  return readFileSync(
    fileURLToPath(new URL(`../../${relative}`, import.meta.url)),
    "utf8",
  );
}

const CHAT_RS = repoSource("src/server/chat.rs");
const RELAY_RS = repoSource("src/server/relay.rs");
const ERROR_RS = repoSource("src/server/error.rs");
const SELECTOR_RS = repoSource("src/domain/selector.rs");
const MOD_RS = repoSource("src/server/mod.rs");

/**
 * 送り先の `model`。**識別子とも目印とも別の文字列**にしてある
 * （同値にすると「渡されたものをそのまま送っているか」が測れない）。
 */
const SENT_MODEL = "aoi@41";

/** 送る本文。**他のどの定数とも別の文字列。** */
const SENT_CONTENT = "ただいま";

/** 識別子。**`SENT_MODEL` の前半と一致させない**（`isSessionOf` の検査で使う）。 */
const HANDLE = "midori";

/**
 * **`chat.ts` が送らないと決めている欄。**
 *
 * `chat.rs` は**未知のフィールドを拒まない**（`deny_unknown_fields` を付けない / ADR-0001）ので、
 * **送らない欄があってよい。** ただし**省略可でなければ 400 になる**ので、そこは下で見る。
 *
 * **⚠ 「送らないと決めた」を書き残すのが要点。**
 * ここを書かずに「送っている欄が受理される」だけを見ると、
 * **Rust 側に必須の欄が増えた日に、誰も気付かない。**
 */
const NOT_SENT: readonly string[] = ["max_tokens", "temperature"];

/** `IncomingMessage` のうち送らない欄。**いまは無い**（役割も本文も送る）。 */
const NOT_SENT_IN_MESSAGE: readonly string[] = [];

/**
 * **`Json<T>` の抽出器が要求する媒体型。**
 *
 * **写し元は axum で、personacore ではない**（`JsonRejection::MissingJsonContentType`）。
 * だから綴りだけは**ここに置く**——**リポジトリのどこを読んでも出てこない。**
 *
 * **⚠ 「だから見張らなくてよい」ではない。**
 * **要求するかどうかを決めているのは `chat.rs` の側**（`Json<…>` で受けるか否か）なので、
 * **その判断のほうを読む**（[`jsonBodyTypeOf`]）。
 * 外すと `ApiError::invalid_body()` で**その回が丸ごと 400** になり、本文が1文字も出ない。
 */
const JSON_MEDIA_TYPE = "application/json";

/**
 * **`chat.rs` が本文を受け取る型。**
 *
 * ここを見るのは、**この検査が欄を突き合わせている構造体と、
 * ハンドラが実際に読む構造体が同じであること**を繋ぐため。
 * **繋がないと、ハンドラが別の型へ乗り換えた日に、欄の突き合わせだけが正しく緑になる。**
 */
const BODY_TYPE = "ChatCompletionRequest";

/**
 * **FE が値として読む欄。** 右辺は `Relay::relay` が組む JSON の中の道。
 *
 * **この道に植えた目印が FE の手元に出てくること**を下で実際に測る。
 */
const CHUNK_READ = {
  /** 着地したセッション（ADR-0004 の復旧経路）。 */
  model: "model",
  /** 本文の増分。 */
  content: "choices[].delta.content",
} as const;

/**
 * **通り道の入れ物。** FE は値としては読まないが、**辿れないと本文に届かない。**
 * 改名すると `readFrame` が `unreadable` を返す（＝画面に本文が1文字も出ない）。
 */
const CHUNK_CONTAINERS: readonly string[] = ["choices", "choices[].delta"];

/**
 * **FE が一切見ない欄。**
 *
 * **載っていること自体は OpenAI 互換の要件**（`relay.rs` の doc）で、
 * **この画面は見ない**というだけ。**分類から漏らさない**ために並べてある
 * ——ここを改名すると、下の「1つ残らず分類してある」が赤くなる。
 *
 * **`choices[].delta.role?` の `?` は「先頭チャンクにしか載らない」の意味**
 * （`relay.rs` の `if !self.started`）。**`insert` を `if` の外へ出す変異は赤くなる。**
 *
 * **⚠ ただし「無条件にする変異でも赤くなる」とは書けない**（B-20 のレビューが実測）——
 * **`if !self.started {` を `if true {` にする変異は、ここでは緑。**
 * **走査は波括弧の深さで省略可否を決めていて、条件が実際に条件かを見ていない。**
 * **権威は Rust 側に実在する** —— その形は
 * `server::relay::tests::a_relayed_chunk_is_an_openai_compatible_chunk` が赤にする（実測）。
 *
 * # ⚠ **この一覧は `relay.rs` 側の権威と重なる。重複ではない**
 *
 * あちらは**厳密な JSON を丸ごと固定する**ので、鍵を1つ改名すれば当然赤くなる。
 * **だが「FE がどれを読み、どれを読まないか」は1文字も言わない。**
 * こちらが言うのはそれだけで、**あちらが言うのは「Rust が何を出すか」だけ。**
 * **片方を「重複だから」と消すと、この分類は誰も守らなくなる。**
 */
const CHUNK_IGNORED: readonly string[] = [
  "id",
  "object",
  "created",
  "choices[].index",
  "choices[].delta.role?",
  "choices[].finish_reason",
];

/**
 * **FE が値として読むエラーの欄**（`api.ts` の `parseErrorBody` 越し）。
 *
 * **`code` は「無いときキーごと落ちる」**（`error.rs` の `if let Some(code)`）ので `?` 付き。
 * **落ちたときに `null` になること**も下で測る。
 */
const ERROR_READ = {
  message: "error.message",
  kind: "error.type",
  code: "error.code?",
} as const;

/** エラーの枠の通り道。 */
const ERROR_CONTAINERS: readonly string[] = ["error"];

/** FE が一切見ないエラーの欄。**いまは無い**（3欄とも読む）。 */
const ERROR_IGNORED: readonly string[] = [];

afterEach(() => {
  vi.unstubAllGlobals();
  seenCalls = [];
});

describe("送る本体は、サーバが受け取る形になっている", () => {
  it("送っている欄は、すべて `ChatCompletionRequest` にある", async () => {
    const sent = await sentBody();
    const known = rustFieldsOf(CHAT_RS, "ChatCompletionRequest").map(
      (field) => field.name,
    );

    // **黙って捨てられる欄を作らない。** `deny_unknown_fields` が無いので、
    // 綴りがずれた欄は 400 にならず**無言で無視される。**
    expect(Object.keys(sent).filter((key) => !known.includes(key))).toEqual([]);
  });

  it("サーバが必須にしている欄は、すべて送っている", async () => {
    const sent = await sentBody();
    const required = rustFieldsOf(CHAT_RS, "ChatCompletionRequest")
      .filter((field) => !field.optional)
      .map((field) => field.name);

    // ここが破れると、その回は 400 になって**本文が1文字も出ない。**
    expect(required.filter((name) => !(name in sent))).toEqual([]);
  });

  it("`ChatCompletionRequest` の欄を1つ残らず分類してある（送る／送らない）", async () => {
    const sent = await sentBody();
    const fields = rustFieldsOf(CHAT_RS, "ChatCompletionRequest");

    expect(
      fields
        .map((field) => field.name)
        .filter((name) => !(name in sent))
        .sort(),
    ).toEqual([...NOT_SENT].sort());

    // **送らない欄は、省略可でなければならない。**
    expect(
      fields
        .filter((field) => NOT_SENT.includes(field.name))
        .map((field) => `${field.name}:${field.optional ? "省略可" : "必須"}`)
        .sort(),
    ).toEqual([...NOT_SENT].map((name) => `${name}:省略可`).sort());
  });

  it("`IncomingMessage` の欄を1つ残らず分類してある", async () => {
    const message = firstMessageOf(await sentBody());
    const fields = rustFieldsOf(CHAT_RS, "IncomingMessage");
    const known = fields.map((field) => field.name);

    expect(Object.keys(message).filter((key) => !known.includes(key))).toEqual(
      [],
    );
    expect(
      fields
        .filter((field) => !field.optional)
        .map((field) => field.name)
        .filter((name) => !(name in message)),
    ).toEqual([]);
    expect(known.filter((name) => !(name in message)).sort()).toEqual(
      [...NOT_SENT_IN_MESSAGE].sort(),
    );
  });

  it("送る役割は `chat.rs` が採用する綴りと一致する", async () => {
    const message = firstMessageOf(await sentBody());

    // **完全一致でしか採用されない**（`chat.rs` の `USER_ROLE` の doc）。
    // ずれると `last_user_message` が1件も見つけられず、その回が丸ごと 400 になる。
    expect(message["role"]).toBe(rustConstOf(CHAT_RS, "USER_ROLE"));
  });

  it("送り先のパスとメソッドは `mod.rs` の登録と一致する", async () => {
    await sentBody();
    const registered = routeOf(MOD_RS, "chat::chat_completions");

    // **メソッドも写しである**（B-20 のレビューが実測: どちらを `get` にしても素通りしていた）。
    // ずれると axum は 405 を返し、その回は本文が1文字も出ない。
    expect(sentRequests()).toEqual([
      { path: registered.path, method: registered.method },
    ]);
  });

  it("本文の媒体型は、`chat.rs` が `Json<…>` で受けることの写しである", async () => {
    await sentBody();

    // **`Json<T>` の抽出器が `application/json` を要求する。**
    // 外すと `JsonRejection` → `ApiError::invalid_body()` で**その回が丸ごと 400**。
    expect(jsonBodyTypeOf(CHAT_RS, "chat_completions")).toBe(BODY_TYPE);
    expect(sentContentTypes()).toEqual([JSON_MEDIA_TYPE]);
  });

  it("`stream` に `true` を送る（この画面は SSE しか読めない）", async () => {
    // **`false` / 未指定に落ちると、返るのは1個の JSON** になり、
    // `readFrame` は `choices[0].delta` を見つけられずに `unreadable` になる。
    expect((await sentBody())["stream"]).toBe(true);
  });

  it("`model` と本文は、渡されたものがそのまま乗る", async () => {
    const sent = await sentBody();

    expect(sent["model"]).toBe(SENT_MODEL);
    expect(firstMessageOf(sent)["content"]).toBe(SENT_CONTENT);
  });
});

describe("SSE のチャンクの枠は `Relay::relay` の写しである", () => {
  it("`Relay::relay` が載せる鍵を1つ残らず分類してある", () => {
    expect(pathsOf(relayTree()).sort()).toEqual(
      [
        ...Object.values(CHUNK_READ),
        ...CHUNK_CONTAINERS,
        ...CHUNK_IGNORED,
      ].sort(),
    );
  });

  it("植えた目印は、道ごとに全部違う", () => {
    // **これが無いと「どこから読んでも緑」になる**（CLAUDE.md の最頻の穴）。
    const marks = [...plant(relayTree(), { includeOptional: true }).marks.values()];

    expect(new Set(marks).size).toBe(marks.length);
    expect(marks.length).toBeGreaterThan(1);
  });

  it("FE は、Rust が本文と `model` を置いた場所から取り出す", async () => {
    const planted = plant(relayTree(), { includeOptional: true });
    const seen = await run([
      sseEvent(JSON.stringify(planted.value)),
      sseEvent(rustConstOf(RELAY_RS, "DONE_DATA")),
    ]);

    expect(seen.outcome).toEqual({ kind: "done" });
    expect(seen.deltas).toEqual([markAt(planted, CHUNK_READ.content)]);
    expect(seen.models).toEqual([markAt(planted, CHUNK_READ.model)]);
  });

  it("対照: 本文を1段浅い場所に置くと、FE は取り出さない", async () => {
    const planted = plant(relayTree(), { includeOptional: true });
    const moved = moveMark(planted.value, CHUNK_READ.content, "choices[].text");
    const seen = await run([
      sseEvent(JSON.stringify(moved)),
      sseEvent(rustConstOf(RELAY_RS, "DONE_DATA")),
    ]);

    // 枠としては読めているので `unreadable` ではない。**本文だけが出ない。**
    expect(seen.outcome).toEqual({ kind: "done" });
    expect(seen.deltas).toEqual([]);
  });

  it("対照: `model` を1段深い場所に置くと、FE は取り出さない", async () => {
    const planted = plant(relayTree(), { includeOptional: true });
    const moved = moveMark(planted.value, CHUNK_READ.model, "choices[].model");
    const seen = await run([
      sseEvent(JSON.stringify(moved)),
      sseEvent(rustConstOf(RELAY_RS, "DONE_DATA")),
    ]);

    expect(seen.outcome).toEqual({ kind: "done" });
    expect(seen.models).toEqual([]);
  });

  /**
   * **入れ物は1つずつ動かす。**
   *
   * **先頭だけを動かすと、`choices` が読めない時点で `delta` まで届かず、
   * 深いほうの入れ物が一度も試されない**（`D-2` の「選ぶ対象を端に置かない」と同じ形）。
   */
  it.each(CHUNK_CONTAINERS)("対照: 入れ物 `%s` の鍵をずらすと、枠ごと読めなくなる", async (container) => {
    const planted = plant(relayTree(), { includeOptional: true });
    const moved = moveMark(planted.value, container, renamedTail(container));
    const seen = await run([sseEvent(JSON.stringify(moved))]);

    expect(seen.outcome).toEqual({ kind: "unreadable" });
  });

  it("終端の番兵は `relay.rs` の定数と一致する", async () => {
    const seen = await run([sseEvent(rustConstOf(RELAY_RS, "DONE_DATA"))]);

    expect(seen.outcome).toEqual({ kind: "done" });
  });

  it("対照: 別の綴りの番兵では終端しない", async () => {
    const seen = await run([sseEvent("[FINISHED]")]);

    // **JSON ですらないので `unreadable`。** 「何でも終端に見える」形になっていない。
    expect(seen.outcome).toEqual({ kind: "unreadable" });
  });
});

describe("SSE の中のエラーの枠は `as_event_data` の写しである", () => {
  it("`as_event_data` が載せる鍵を1つ残らず分類してある", () => {
    expect(pathsOf(errorTree()).sort()).toEqual(
      [
        ...Object.values(ERROR_READ),
        ...ERROR_CONTAINERS,
        ...ERROR_IGNORED,
      ].sort(),
    );
  });

  it("FE は、Rust が置いた場所から3つの欄を取り出す", async () => {
    const planted = plant(errorTree(), { includeOptional: true });
    const seen = await run([sseEvent(JSON.stringify(planted.value))]);

    expect(seen.outcome).toEqual({
      kind: "upstream-error",
      detail: {
        message: markAt(planted, ERROR_READ.message),
        kind: markAt(planted, ERROR_READ.kind),
        code: markAt(planted, ERROR_READ.code),
      },
    });
  });

  it("省略された `code` は `null` になる（キーごと落ちる約束）", async () => {
    const planted = plant(errorTree(), { includeOptional: false });
    const seen = await run([sseEvent(JSON.stringify(planted.value))]);

    expect(seen.outcome).toEqual({
      kind: "upstream-error",
      detail: {
        message: markAt(planted, ERROR_READ.message),
        kind: markAt(planted, ERROR_READ.kind),
        code: null,
      },
    });
  });

  it("対照: 外側の鍵をずらすと、エラーとして読めなくなる", async () => {
    const planted = plant(errorTree(), { includeOptional: true });
    const moved = moveMark(planted.value, ERROR_CONTAINERS[0] ?? "", "failure");
    const seen = await run([sseEvent(JSON.stringify(moved))]);

    expect(seen.outcome).toEqual({ kind: "unreadable" });
  });
});

describe("`model` の綴りの規則は `selector.rs` の写しである（ADR-0004）", () => {
  it("新規セッションは、区切りと予約語をそのまま繋いだ形", () => {
    // **`ModelSelector::parse` は最初の区切りで1回だけ分け、接尾辞を完全一致で見る。**
    // ずれると `SessionRef::New` にならず、**黙って前の会話の続き**になる。
    expect(startNew(HANDLE)).toBe(
      `${HANDLE}${separatorOf(SELECTOR_RS)}${rustConstOf(SELECTOR_RS, "NEW_SUFFIX")}`,
    );
  });

  /**
   * **過去の会話を選んで送る形**（作業単位 D-12c-2 / ADR-0004 の `aoi@<id>`）。
   *
   * **ずれると `SessionRef::Existing` にならない** —— 区切りが違えば識別子の検証で 400、
   * 数でなければ `model_invalid_suffix`。**画面は「この会話の続き」と言っているのに届かない。**
   * **選んだ id がそのまま載ること**も見る（別の会話へ書かない）。
   */
  /**
   * **着地した会話の id を読む**（作業単位 D-12c-2）。画面はこれで「表示している会話」と突き合わせる。
   * **読めない綴りを読めたことにしない**（別のキャラクター・数でない・空）。
   */
  it("着地した会話の id は、区切りの後ろの整数だけを読む", () => {
    const separator = separatorOf(SELECTOR_RS);
    expect(sessionIdOf(HANDLE, `${HANDLE}${separator}12`)).toBe(12);
    expect(sessionIdOf(HANDLE, continueSession(HANDLE, 907))).toBe(907);
    for (const model of [
      HANDLE,
      `${HANDLE}${separator}`,
      `${HANDLE}${separator}new`,
      `${HANDLE}${separator}1x`,
      `${HANDLE}${separator}-3`,
      `${HANDLE}${separator}1.5`,
      `x${HANDLE}${separator}12`,
      `${HANDLE}-2${separator}12`,
    ]) {
      expect(sessionIdOf(HANDLE, model), model).toBeNull();
    }
  });

  it("既存の会話の継続は、区切りと id を繋いだ形", () => {
    const separator = separatorOf(SELECTOR_RS);
    expect(continueSession(HANDLE, 12)).toBe(`${HANDLE}${separator}12`);
    // **画面が固定を確かめる関数と、同じ形を作っている**（食い違うと選んだ直後に警告が出る）。
    expect(isSessionOf(HANDLE, continueSession(HANDLE, 12))).toBe(true);
  });

  it("最新の継続は、接尾辞を1文字も付けない", () => {
    expect(continueLatest(HANDLE)).toBe(HANDLE);
    expect(continueLatest(HANDLE)).not.toContain(separatorOf(SELECTOR_RS));
  });

  it("セッションかどうかは、`selector.rs` の区切りで見分ける", () => {
    const separator = separatorOf(SELECTOR_RS);

    expect(isSessionOf(HANDLE, `${HANDLE}${separator}41`)).toBe(true);
    // 接尾辞が無い＝まだ着地していない。
    expect(isSessionOf(HANDLE, HANDLE)).toBe(false);
    // **別のキャラクターを前方一致で通さない。**
    expect(isSessionOf(HANDLE, `${HANDLE}x${separator}41`)).toBe(false);
    expect(isSessionOf(HANDLE, `x${HANDLE}${separator}41`)).toBe(false);
  });
});

// ===========================================================================
// 走査本体（Rust 側）と、その対照
// ===========================================================================

describe("Rust 側の走査: `Map` を組む関数（植えたソースを通す）", () => {
  it("入れ子と配列を、線の上の道として読む", () => {
    const planted = [
      "    fn planted(&self) -> String {",
      "        let mut inner = Map::new();",
      '        inner.insert("leaf".to_owned(), Value::from(1));',
      "",
      "        let mut outer = Map::new();",
      '        outer.insert("own".to_owned(), Value::from(2));',
      '        outer.insert("wrapped".to_owned(), Value::Object(inner));',
      "",
      "        let mut top = Map::new();",
      '        top.insert("列".to_owned(), Value::Array(vec![Value::Object(outer)]));',
      "",
      "        Value::Object(top).to_string()",
      "    }",
    ].join("\n");

    expect(pathsOf(jsonTreeOf(planted, "fn planted("))).toEqual([
      "列",
      "列[].own",
      "列[].wrapped",
      "列[].wrapped.leaf",
    ]);
  });

  it("枝の中で載せる欄は省略可（`?` が付く）", () => {
    const planted = [
      "    fn planted(&self) -> String {",
      "        let mut top = Map::new();",
      '        top.insert("always".to_owned(), Value::from(1));',
      "        if self.first {",
      '            top.insert("sometimes".to_owned(), Value::from(2));',
      "        }",
      "        Value::Object(top).to_string()",
      "    }",
    ].join("\n");

    expect(pathsOf(jsonTreeOf(planted, "fn planted("))).toEqual([
      "always",
      "sometimes?",
    ]);
  });

  it("コメントと文字列の中の `{` `}` で本文を切らない", () => {
    const planted = [
      "    fn planted(&self) -> String {",
      "        // 閉じ括弧 } を含むコメント。",
      '        /* `{"a": 1}` のような形が来る。 */',
      "        let mut top = Map::new();",
      '        top.insert("brace".to_owned(), Value::from("}{"));',
      "        Value::Object(top).to_string()",
      "    }",
    ].join("\n");

    expect(pathsOf(jsonTreeOf(planted, "fn planted("))).toEqual(["brace"]);
  });

  it("対照: 探せていないことを緑にしない", () => {
    const body = [
      "    fn planted(&self) -> String {",
      "        let mut top = Map::new();",
      '        top.insert("a".to_owned(), Value::from(1));',
      "        Value::Object(top).to_string()",
      "    }",
    ].join("\n");

    // **その名前の関数が無い。**
    expect(() => jsonTreeOf(body, "fn missing(")).toThrow();
    // **同じ目印が2回**（どちらを読んだのか決まらない）。
    expect(() => jsonTreeOf(`${body}\n${body}`, "fn planted(")).toThrow();
    // **`Map` を1つも組んでいない。**
    expect(() =>
      jsonTreeOf(
        '    fn planted(&self) -> String {\n        "x".to_owned()\n    }',
        "fn planted(",
      ),
    ).toThrow();
    // **根が2つある**（どちらが線に乗るのか決まらない）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut one = Map::new();",
          '        one.insert("a".to_owned(), Value::from(1));',
          "        let mut two = Map::new();",
          '        two.insert("b".to_owned(), Value::from(2));',
          "        Value::Object(one).to_string();",
          "        Value::Object(two).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
    // **根から辿り着けない `Map` がある**（黙って落とすと、その欄ごと見えなくなる）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut orphan = Map::new();",
          '        orphan.insert("lost".to_owned(), Value::from(1));',
          "        let mut top = Map::new();",
          '        top.insert("a".to_owned(), Value::from(2));',
          "        Value::Object(top).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
    // **知らない形で `Map` を混ぜている**（黙って葉として数えない）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut inner = Map::new();",
          '        inner.insert("a".to_owned(), Value::from(1));',
          "        let mut top = Map::new();",
          '        top.insert("b".to_owned(), Value::Array(vec![Value::Object(inner), Value::Null]));',
          "        Value::Object(top).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
    // **鍵が文字列リテラルではない**（読めない形を黙って飛ばさない）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut top = Map::new();",
          "        top.insert(key.to_owned(), Value::from(1));",
          "        Value::Object(top).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
    // **同じ鍵を2回載せている**（後勝ちなのか、条件つきなのかが決まらない）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut top = Map::new();",
          '        top.insert("a".to_owned(), Value::from(1));',
          '        top.insert("a".to_owned(), Value::from(2));',
          "        Value::Object(top).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
    // **生文字列は読めない**（黙って読み違えるより落ちる）。
    expect(() =>
      jsonTreeOf(
        [
          "    fn planted(&self) -> String {",
          "        let mut top = Map::new();",
          '        top.insert("a".to_owned(), Value::from(r"x"));',
          "        Value::Object(top).to_string()",
          "    }",
        ].join("\n"),
        "fn planted(",
      ),
    ).toThrow();
  });
});

describe("Rust 側の走査: 定数・パス・区切り（植えたソースを通す）", () => {
  it("`&str` の定数を読む", () => {
    const planted = [
      '/// 説明の中の PLANTED: &str = "囮"; は拾わない。',
      'const PLANTED: &str = "本物";',
      'pub(super) const EXPORTED: &str = "外";',
      "const NUMBER: u32 = 3;",
    ].join("\n");

    expect(rustConstOf(planted, "PLANTED")).toBe("本物");
    expect(rustConstOf(planted, "EXPORTED")).toBe("外");
    // **その名前が無い。**
    expect(() => rustConstOf(planted, "MISSING")).toThrow();
    // **`&str` ではない。**
    expect(() => rustConstOf(planted, "NUMBER")).toThrow();
    // **同じ名前が2回。**
    expect(() => rustConstOf(`${planted}\n${planted}`, "PLANTED")).toThrow();
  });

  it("`.route(…)` の登録からパスとメソッドを読む", () => {
    const planted = [
      "    Router::new()",
      '        .route("/v1/planted", post(planted::handler))',
      '        .route("/v1/other", get(other::handler))',
      '        .route("/v1/both", get(both::read).patch(both::write))',
    ].join("\n");

    expect(routeOf(planted, "planted::handler")).toEqual({
      path: "/v1/planted",
      method: "POST",
    });
    // **同じ1本に2つ載っていても、その処理を包んでいる呼び出しを取る。**
    expect(routeOf(planted, "both::read")).toEqual({
      path: "/v1/both",
      method: "GET",
    });
    expect(routeOf(planted, "both::write")).toEqual({
      path: "/v1/both",
      method: "PATCH",
    });
    // **その処理が登録されていない。**
    expect(() => routeOf(planted, "missing::handler")).toThrow();
    // **前方一致で拾わない。**
    expect(() => routeOf(planted, "both::rea")).toThrow();
    // **2箇所に登録されている**（どちらが送り先か決まらない）。
    expect(() =>
      routeOf(`${planted}\n${planted}`, "planted::handler"),
    ).toThrow();
    // **知らないメソッドは黙って通さない。**
    expect(() =>
      routeOf('.route("/v1/x", verb(x::handler))', "x::handler"),
    ).toThrow();
  });

  it("ハンドラが `Json<…>` で受けている型を読む", () => {
    const planted = [
      "pub(super) async fn planted(",
      "    State(state): State<AppState>,",
      "    // **`Json<T>` を直接受けない。** ← コメントの中は数えない",
      "    body: Result<Json<PlantedRequest>, JsonRejection>,",
      ") -> Result<Response, ApiError> {",
      "    let _ = (state, body);",
      "}",
      "",
      "pub(super) async fn raw(body: Bytes) -> Result<Response, ApiError> {",
      "    let _ = body;",
      "}",
      "",
      "pub(super) async fn twice(",
      "    a: Json<One>,",
      "    b: Json<Two>,",
      ") -> Result<Response, ApiError> {",
      "    let _ = (a, b);",
      "}",
    ].join("\n");

    expect(jsonBodyTypeOf(planted, "planted")).toBe("PlantedRequest");
    // **その名前の関数が無い。**
    expect(() => jsonBodyTypeOf(planted, "missing")).toThrow();
    // **`Json<…>` で受けていない**（媒体型を要求しない ＝ 写しの前提が消えている）。
    expect(() => jsonBodyTypeOf(planted, "raw")).toThrow();
    // **2つある**（どちらが本文なのか決まらない）。
    expect(() => jsonBodyTypeOf(planted, "twice")).toThrow();
  });

  it("`split_once` の区切り文字を読む", () => {
    const planted = [
      "        // `split_once('!')` はコメントなので拾わない。",
      "        let (head, tail) = match raw.split_once('%') {",
      "            Some(pair) => pair,",
      "        };",
    ].join("\n");

    expect(separatorOf(planted)).toBe("%");
    // **1つも無い。**
    expect(() => separatorOf("let x = 1;")).toThrow();
    // **2つある**（どちらが `model` の区切りか決まらない）。
    expect(() => separatorOf(`${planted}\n${planted}`)).toThrow();
  });
});

describe("植える側の対照（目印の置き換えが本当に効くか）", () => {
  it("`plant` は道ごとに目印を持つ", () => {
    const tree: WireNode = {
      kind: "object",
      fields: [
        { name: "a", optional: false, node: { kind: "leaf" } },
        {
          name: "b",
          optional: true,
          node: {
            kind: "array",
            of: {
              kind: "object",
              fields: [{ name: "c", optional: false, node: { kind: "leaf" } }],
            },
          },
        },
      ],
    };

    const withOptional = plant(tree, { includeOptional: true });
    expect(withOptional.value).toEqual({ a: "<a>", b: [{ c: "<b[].c>" }] });
    expect(markAt(withOptional, "a")).toBe("<a>");
    expect(markAt(withOptional, "b[].c")).toBe("<b[].c>");
    expect(() => markAt(withOptional, "missing")).toThrow();

    const without = plant(tree, { includeOptional: false });
    expect(without.value).toEqual({ a: "<a>" });
  });

  it("`moveMark` は、その道の値を別の道へ移す", () => {
    const before = { outer: { inner: "x" }, keep: 1 };

    expect(moveMark(before, "outer.inner", "outer.moved")).toEqual({
      outer: { moved: "x" },
      keep: 1,
    });
    expect(moveMark(before, "outer", "renamed")).toEqual({
      renamed: { inner: "x" },
      keep: 1,
    });
    // **元の値は壊さない**（突き合わせの相手を作り替えていない）。
    expect(before).toEqual({ outer: { inner: "x" }, keep: 1 });
    // **無い道は動かせない。**
    expect(() => moveMark(before, "missing", "x")).toThrow();
  });

  it("`renamedTail` は末尾の1歩だけを別名にする", () => {
    // **手前の段を書き換えないこと。** 書き換えると、狙った入れ物ではなく
    // その親を動かすことになり、**どの入れ物を試したのか分からなくなる。**
    expect(renamedTail("choices")).toBe("choicesX");
    expect(renamedTail("choices[].delta")).toBe("choices[].deltaX");
    expect(() => renamedTail("")).toThrow();
  });
});

// ===========================================================================
// 道具
// ===========================================================================

/** 線に乗る枠の1節点。 */
type WireNode =
  | { kind: "object"; fields: WireBranch[] }
  | { kind: "array"; of: WireNode }
  | { kind: "leaf" };

/** 入れ物の中の1欄。 */
interface WireBranch {
  name: string;
  /** 枝の中で載せているか（`if` の内側なら省略されうる）。 */
  optional: boolean;
  node: WireNode;
}

/** `Relay::relay` が組む枠。 */
function relayTree(): WireNode {
  return jsonTreeOf(RELAY_RS, "fn relay(&mut self, chunk: &ChatChunk)");
}

/** `ApiError::as_event_data` が組む枠。 */
function errorTree(): WireNode {
  return jsonTreeOf(ERROR_RS, "fn as_event_data(self) -> String");
}

/**
 * **`serde_json::Map` を手で組む関数を読む。**
 *
 * `<変数>.insert("<鍵>".to_owned(), <式>)` を集め、
 * `Value::Object(<変数>)` / `Value::Array(vec![Value::Object(<変数>)])` を辺として繋ぐ。
 * 根は `Value::Object(<変数>).to_string()`。
 *
 * **読めない形はすべて throw する。** 黙って飛ばすと、飛ばした欄が
 * 「両側に無い」ことになって緑を通る（`api-fields.test.ts` と同じ理由）。
 *
 * **枝の深さがそのまま省略可否になる** —— `if` の内側で載せる欄は
 * 載らない回がある（`role` と `code` が実際にそれ）。
 */
function jsonTreeOf(source: string, marker: string): WireNode {
  const body = rustBodyOf(source, marker);
  const kinds = classifyRust(body);

  const roots = matchesOfCode(
    body,
    kinds,
    /Value::Object\(([A-Za-z_][A-Za-z0-9_]*)\)\.to_string\(\)/gu,
  ).map((found) => found[1] ?? "");
  const root = roots[0];
  if (roots.length !== 1 || root === undefined) {
    throw new Error(`根が ${roots.length} 個あります`);
  }

  const inserts = insertsOf(body, kinds);
  const owners = new Map<string, RustInsert[]>();
  for (const insert of inserts) {
    const branches = owners.get(insert.owner) ?? [];
    if (branches.some((branch) => branch.key === insert.key)) {
      throw new Error(`同じ鍵を2回載せています: ${insert.owner}.${insert.key}`);
    }
    // **知らない入れ子の形をここで落とす**（黙って葉として数えない）。
    shapeOf(insert.expr);
    branches.push(insert);
    owners.set(insert.owner, branches);
  }
  if (owners.size === 0) {
    throw new Error("Map を組んでいる場所が1つもありません");
  }

  const seen = new Set<string>();
  const built = expand(root);

  // **根から辿り着けない `Map` を残さない。** 残すと、その欄ごと黙って消える。
  for (const owner of owners.keys()) {
    if (!seen.has(owner)) {
      throw new Error(`根から辿り着けない Map があります: ${owner}`);
    }
  }
  return built;

  /** その変数名の `Map` を展開する。 */
  function expand(owner: string): WireNode {
    if (seen.has(owner)) {
      throw new Error(`Map が自分自身を含んでいます: ${owner}`);
    }
    seen.add(owner);
    const branches = owners.get(owner);
    if (branches === undefined || branches.length === 0) {
      throw new Error(`Map ${owner} に鍵がありません`);
    }
    return {
      kind: "object",
      fields: branches.map((branch) => {
        const shape = shapeOf(branch.expr);
        const node: WireNode =
          shape === "leaf" || branch.child === null
            ? { kind: "leaf" }
            : shape === "array"
              ? { kind: "array", of: expand(branch.child) }
              : expand(branch.child);
        return { name: branch.key, optional: branch.depth > 0, node };
      }),
    };
  }
}

/** 式が指す形。**知らない形は throw。** */
function shapeOf(expr: string): "leaf" | "object" | "array" {
  if (/^Value::Object\(\s*[A-Za-z_][A-Za-z0-9_]*\s*\)$/u.test(expr)) {
    return "object";
  }
  if (
    /^Value::Array\(\s*vec!\[\s*Value::Object\(\s*[A-Za-z_][A-Za-z0-9_]*\s*\)\s*\]\s*\)$/u.test(
      expr,
    )
  ) {
    return "array";
  }
  if (expr.includes("Value::Object(") || expr.includes("Value::Array(")) {
    throw new Error(`知らない入れ子の形です: ${expr}`);
  }
  return "leaf";
}

/** `<変数>.insert("<鍵>".to_owned(), <式>)` 1件。 */
interface RustInsert {
  owner: string;
  key: string;
  expr: string;
  /** 入れ子の子の変数名（葉なら `null`）。 */
  child: string | null;
  /** 関数本体の中の枝の深さ。**0 なら必ず載る。** */
  depth: number;
}

/** 関数本体から `insert` を全部、**現れる順に**。 */
function insertsOf(body: string, kinds: Uint8Array): RustInsert[] {
  const found: RustInsert[] = [];
  for (const match of matchesOfCode(
    body,
    kinds,
    /([A-Za-z_][A-Za-z0-9_]*)\.insert\(/gu,
  )) {
    const openAt = (match.index ?? 0) + match[0].length - 1;
    const closeAt = matchingAt(body, kinds, openAt, "(", ")");
    const args = splitTopLevel(
      body.slice(openAt + 1, closeAt),
      kinds,
      openAt + 1,
    );
    const key = /^\s*"([^"]*)"\.to_owned\(\)\s*$/u.exec(args[0] ?? "");
    if (key?.[1] === undefined) {
      throw new Error(`鍵が文字列リテラルではありません: ${args[0] ?? ""}`);
    }
    const expr = (args[1] ?? "").trim();
    if (expr === "" || args.length !== 2) {
      throw new Error(`insert の引数が2つではありません: ${args.join(" / ")}`);
    }
    const inner =
      /Value::(?:Object|Array)\(\s*(?:vec!\[\s*Value::Object\(\s*)?([A-Za-z_][A-Za-z0-9_]*)/u.exec(
        expr,
      );
    found.push({
      owner: match[1] ?? "",
      key: key[1],
      expr,
      child: inner?.[1] ?? null,
      depth: depthAt(body, kinds, openAt),
    });
  }
  return found;
}

/** `const NAME: &str = "…";` の値。**1つでなければ throw。** */
function rustConstOf(source: string, name: string): string {
  const pattern = new RegExp(
    `^\\s*(?:pub(?:\\([^)]*\\))?\\s+)?const ${name}: &(?:'static )?str = "([^"]*)";$`,
    "u",
  );
  const found = source
    .split("\n")
    .map((line) => pattern.exec(line.replace(/\r$/u, "")))
    .filter((match): match is RegExpExecArray => match !== null);
  const first = found[0];
  if (found.length !== 1 || first?.[1] === undefined) {
    throw new Error(`const ${name}: &str の宣言が ${found.length} 個あります`);
  }
  return first[1];
}

/** axum の method router。**知らない綴りは throw する**（黙って素通ししない）。 */
const HTTP_METHODS: readonly string[] = [
  "get",
  "post",
  "put",
  "patch",
  "delete",
  "head",
  "options",
  "trace",
];

/**
 * `.route("<パス>", <メソッド>(<処理>)…)` の**パスとメソッド**。**1つでなければ throw。**
 *
 * **区間で切って探す。** 1つの正規表現で「パス … 処理」を跨がせると、
 * **手前の `.route(` のパスを拾ってしまう**（`/v1/models` の登録から
 * `chat::chat_completions` まで一続きに読めてしまう）。
 *
 * **⚠ メソッドを読み飛ばさないこと**（B-20 のレビューが実測）。
 * `.route("…", get(…))` と `.route("…", post(…))` は**別の登録**で、
 * 取り違えると axum は 405 を返す。**パスだけを見る形にしていたのが誤りだった。**
 * 1本の `.route` に `get(…).post(…)` と2つ載る形があるので、
 * **その処理を包んでいる呼び出しの名前**を取る。
 */
function routeOf(
  source: string,
  handler: string,
): { path: string; method: string } {
  const kinds = classifyRust(source);
  const starts = matchesOfCode(source, kinds, /\.route\(\s*"([^"]*)"\s*,/gu);
  const escaped = handler.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const wanted = new RegExp(
    `(?<![A-Za-z0-9_])([A-Za-z_][A-Za-z0-9_]*)\\(\\s*${escaped}\\s*\\)`,
    "u",
  );
  const found: { path: string; method: string }[] = [];
  for (const [index, match] of starts.entries()) {
    const from = (match.index ?? 0) + match[0].length;
    const to = starts[index + 1]?.index ?? source.length;
    const hit = wanted.exec(source.slice(from, to));
    if (hit?.[1] !== undefined) {
      found.push({ path: match[1] ?? "", method: hit[1] });
    }
  }
  const first = found[0];
  if (found.length !== 1 || first === undefined) {
    throw new Error(`${handler} の登録が ${found.length} 個あります`);
  }
  if (!HTTP_METHODS.includes(first.method)) {
    throw new Error(`知らないメソッドです: ${first.method}`);
  }
  return { path: first.path, method: first.method.toUpperCase() };
}

/**
 * そのハンドラが `Json<T>` で本文を受けているときの `T`。**1つでなければ throw。**
 *
 * **`Json<T>` で受ける = axum の抽出器が [`JSON_MEDIA_TYPE`] を要求する**、が写しの中身。
 * `Bytes` や `String` へ乗り換えたらここが落ちる ——
 * **要求しなくなったのに FE が送り続けている状態を、緑にしない。**
 */
function jsonBodyTypeOf(source: string, functionName: string): string {
  const at = source.indexOf(`fn ${functionName}(`);
  if (at < 0) {
    throw new Error(`fn ${functionName} が見つかりません`);
  }
  const kinds = classifyRust(source);
  const openAt = source.indexOf("(", at);
  const closeAt = matchingAt(source, kinds, openAt, "(", ")");
  // **コメントの中の `Json<T>` を数えない。** `chat.rs` の署名の直上には
  // 「**`Json<T>` を直接受けない**」という注記があり、素朴に数えると 2 個になる。
  const found = matchesOfCode(
    source,
    kinds,
    /\bJson<([A-Za-z_][A-Za-z0-9_]*)>/gu,
  )
    .filter((match) => {
      const index = match.index ?? 0;
      return index > openAt && index < closeAt;
    })
    .map((match) => match[1] ?? "");
  const first = found[0];
  if (found.length !== 1 || first === undefined) {
    throw new Error(
      `fn ${functionName} が Json<…> で受けている箇所が ${found.length} 個あります`,
    );
  }
  return first;
}

/** `a.b[].c` → `a.b[].cX`。**末尾の1歩だけを別名にする。** */
function renamedTail(path: string): string {
  const steps = path.split(".");
  const last = steps.pop();
  if (last === undefined || last === "") {
    throw new Error(`道の終わりが読めません: ${path}`);
  }
  return [...steps, `${last}X`].join(".");
}

/** `split_once('<文字>')` の区切り文字。**1つでなければ throw。** */
function separatorOf(source: string): string {
  const kinds = classifyRust(source);
  const found = matchesOfCode(source, kinds, /\.split_once\('(.)'\)/gu).map(
    (match) => match[1] ?? "",
  );
  const first = found[0];
  if (found.length !== 1 || first === undefined) {
    throw new Error(`split_once('…') が ${found.length} 個あります`);
  }
  return first;
}

/**
 * 関数本体（外側の `{` と `}` の間）。**目印は1つでなければ throw。**
 *
 * **生文字列（`r"…"`）があれば落とす。** 読み違えるより落ちる方がよい。
 */
function rustBodyOf(source: string, marker: string): string {
  const hits = source.split(marker).length - 1;
  if (hits !== 1) {
    throw new Error(`目印「${marker}」が ${hits} 個あります`);
  }
  const at = source.indexOf(marker);
  const openAt = source.indexOf("{", at);
  if (openAt < 0) {
    throw new Error(`「${marker}」の後に本文がありません`);
  }
  const whole = source.slice(openAt);
  const closeAt = matchingAt(whole, classifyRust(whole), 0, "{", "}");
  const body = whole.slice(1, closeAt);
  if (/\br#*"/u.test(body)) {
    throw new Error('生文字列（r"…"）は読めません');
  }
  return body;
}

/** 文字ごとの種類。**0=コード / 1=文字列や文字リテラル / 2=コメント。** */
function classifyRust(source: string): Uint8Array {
  const kinds = new Uint8Array(source.length);
  let index = 0;
  while (index < source.length) {
    const pair = source.slice(index, index + 2);
    if (pair === "//") {
      const end = source.indexOf("\n", index);
      const stop = end < 0 ? source.length : end;
      kinds.fill(2, index, stop);
      index = stop;
      continue;
    }
    if (pair === "/*") {
      let depth = 0;
      let cursor = index;
      while (cursor < source.length) {
        const here = source.slice(cursor, cursor + 2);
        if (here === "/*") {
          depth += 1;
          cursor += 2;
          continue;
        }
        if (here === "*/") {
          depth -= 1;
          cursor += 2;
          if (depth === 0) {
            break;
          }
          continue;
        }
        cursor += 1;
      }
      kinds.fill(2, index, cursor);
      index = cursor;
      continue;
    }
    const char = source.charAt(index);
    if (char === '"') {
      let cursor = index + 1;
      while (cursor < source.length) {
        if (source.charAt(cursor) === "\\") {
          cursor += 2;
          continue;
        }
        if (source.charAt(cursor) === '"') {
          cursor += 1;
          break;
        }
        cursor += 1;
      }
      kinds.fill(1, index, cursor);
      index = cursor;
      continue;
    }
    // **文字リテラルだけを飛ばす。** ライフタイム（`&'static`）は飛ばさない。
    const literal = /^'(?:\\.|[^\\'])'/u.exec(source.slice(index, index + 4));
    if (char === "'" && literal !== null) {
      kinds.fill(1, index, index + literal[0].length);
      index += literal[0].length;
      continue;
    }
    index += 1;
  }
  return kinds;
}

/** コード部分だけで正規表現を当てる。 */
function matchesOfCode(
  source: string,
  kinds: Uint8Array,
  pattern: RegExp,
): RegExpMatchArray[] {
  return [...source.matchAll(pattern)].filter(
    (match) => kinds[match.index ?? 0] === 0,
  );
}

/** 対応する閉じ括弧の位置。**コード部分だけを数える。** */
function matchingAt(
  source: string,
  kinds: Uint8Array,
  openAt: number,
  open: string,
  close: string,
): number {
  let depth = 0;
  for (let index = openAt; index < source.length; index += 1) {
    if (kinds[index] !== 0) {
      continue;
    }
    const char = source.charAt(index);
    if (char === open) {
      depth += 1;
    } else if (char === close) {
      depth -= 1;
      if (depth === 0) {
        return index;
      }
    }
  }
  throw new Error(`閉じ「${close}」が見つかりません`);
}

/** その位置の `{` の深さ（関数本体の直下なら 0）。 */
function depthAt(source: string, kinds: Uint8Array, at: number): number {
  let depth = 0;
  for (let index = 0; index < at; index += 1) {
    if (kinds[index] !== 0) {
      continue;
    }
    const char = source.charAt(index);
    if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      depth -= 1;
    }
  }
  return depth;
}

/** 括弧の外側の `,` で切る。**空白だけの区切りは落とす**（末尾のコンマ）。 */
function splitTopLevel(
  slice: string,
  kinds: Uint8Array,
  offset: number,
): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < slice.length; index += 1) {
    if (kinds[offset + index] !== 0) {
      continue;
    }
    const char = slice.charAt(index);
    if (char === "(" || char === "[" || char === "{") {
      depth += 1;
    } else if (char === ")" || char === "]" || char === "}") {
      depth -= 1;
    } else if (char === "," && depth === 0) {
      parts.push(slice.slice(start, index));
      start = index + 1;
    }
  }
  parts.push(slice.slice(start));
  return parts.filter((part) => part.trim() !== "");
}

/** 枠の中の道を、**現れる順に**全部（省略可なら `?` 付き）。 */
function pathsOf(node: WireNode, prefix = ""): string[] {
  if (node.kind === "leaf") {
    return [];
  }
  if (node.kind === "array") {
    return pathsOf(node.of, `${prefix}[]`);
  }
  return node.fields.flatMap((branch) => {
    const path = prefix === "" ? branch.name : `${prefix}.${branch.name}`;
    return [branch.optional ? `${path}?` : path, ...pathsOf(branch.node, path)];
  });
}

/** 目印を植えた枠と、道から目印を引くための対応。 */
interface Planted {
  value: unknown;
  marks: Map<string, string>;
}

/**
 * 枠の**葉に、その道そのものを書いた目印**を植える。
 *
 * **目印が道と1対1**なので、**どこから読んでも緑**という形にならない。
 * `includeOptional` が `false` なら、**省略されうる欄を落とす。**
 */
function plant(node: WireNode, options: { includeOptional: boolean }): Planted {
  const marks = new Map<string, string>();

  function build(current: WireNode, prefix: string): unknown {
    if (current.kind === "leaf") {
      const mark = `<${prefix}>`;
      marks.set(prefix, mark);
      return mark;
    }
    if (current.kind === "array") {
      return [build(current.of, `${prefix}[]`)];
    }
    const out: Record<string, unknown> = {};
    for (const branch of current.fields) {
      if (branch.optional && !options.includeOptional) {
        continue;
      }
      const path = prefix === "" ? branch.name : `${prefix}.${branch.name}`;
      out[branch.name] = build(branch.node, path);
    }
    return out;
  }

  return { value: build(node, ""), marks };
}

/** その道に植えた目印。**無ければ throw**（道がずれたことを緑にしない）。 */
function markAt(planted: Planted, path: string): string {
  const mark = planted.marks.get(path.replace(/\?$/u, ""));
  if (mark === undefined) {
    throw new Error(
      `道「${path}」に目印がありません（あるのは ${[...planted.marks.keys()].join(" / ")}）`,
    );
  }
  return mark;
}

/**
 * ある道の値を、別の道へ移した写しを作る。**元は壊さない。**
 *
 * 「FE は Rust が置いた場所から読んでいる」の対照に使う
 * ——**置き場所を変えたら読めなくなる**ことを見る。
 */
function moveMark(value: unknown, from: string, to: string): unknown {
  const copy: unknown = JSON.parse(JSON.stringify(value)) as unknown;
  const source = locate(copy, from.replace(/\?$/u, ""), false);
  if (!(source.key in source.holder)) {
    throw new Error(`道が見つかりません: ${from}`);
  }
  const taken: unknown = source.holder[source.key];
  delete source.holder[source.key];

  const target = locate(copy, to, true);
  target.holder[target.key] = taken;
  return copy;
}

/**
 * 道を辿って、**最後の1歩の手前の入れ物と、最後の鍵**を返す。
 *
 * `name[]` は「1件だけの配列の中へ入る」。**最後の1歩を配列にはできない。**
 */
function locate(
  value: unknown,
  path: string,
  create: boolean,
): { holder: Record<string, unknown>; key: string } {
  const steps = path.split(".");
  const last = steps.pop();
  if (last === undefined || last === "" || last.endsWith("[]")) {
    throw new Error(`道の終わりが読めません: ${path}`);
  }
  let current: unknown = value;
  for (const step of steps) {
    const array = /^(.+)\[\]$/u.exec(step);
    const key = array?.[1] ?? step;
    const holder = asRecord(current);
    if (create && holder[key] === undefined) {
      holder[key] = array === null ? {} : [{}];
    }
    const next: unknown = holder[key];
    if (array === null) {
      current = next;
      continue;
    }
    if (!Array.isArray(next) || next.length !== 1) {
      throw new Error(`1件だけの配列ではありません: ${step}`);
    }
    current = next[0];
  }
  return { holder: asRecord(current), key: last };
}

/** 入れ物として見る。**違えば throw。** */
function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("入れ物ではありません");
  }
  return value as Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// `sendMessage` を実際に走らせる側
// ---------------------------------------------------------------------------

/** `data:` 1件ぶん。 */
function sseEvent(payload: string): string {
  return `data: ${payload}\n\n`;
}

/** 中断しない合図。 */
function signal(): AbortSignal {
  return new AbortController().signal;
}

/** 受け取った増分と `model` を控える。 */
function recorder(): StreamCallbacks & { deltas: string[]; models: string[] } {
  const deltas: string[] = [];
  const models: string[] = [];
  return {
    deltas,
    models,
    onDelta: (text) => void deltas.push(text),
    onModel: (model) => void models.push(model),
  };
}

/** `fetch` の身代わりが控えた呼び出し。 */
let seenCalls: {
  url: string;
  method: string;
  headers: Record<string, string>;
  body: string;
}[] = [];

/** `fetch` の第2引数のうち、控えるもの。 */
interface StubInit {
  method?: unknown;
  headers?: unknown;
  body?: unknown;
}

/** 流すものを持った `fetch` の身代わり。**送り先・メソッド・見出し・本体を控える。** */
function stubFetch(
  chunks: string[],
): (input: unknown, init: unknown) => Promise<Response> {
  return (input, init) => {
    const options = init as StubInit | undefined;
    const headers: Record<string, string> = {};
    for (const [key, value] of Object.entries(
      (options?.headers ?? {}) as Record<string, unknown>,
    )) {
      // **見出しの名前は大小を区別しない**ので、突き合わせる前に揃える。
      headers[key.toLowerCase()] = String(value);
    }
    seenCalls.push({
      url: String(input),
      method: String(options?.method ?? ""),
      headers,
      body: String(options?.body ?? ""),
    });
    return Promise.resolve(new Response(streamOf(chunks)));
  };
}

/** 控えた送り先とメソッド。**まとめて1つの主張として突き合わせる。** */
function sentRequests(): { path: string; method: string }[] {
  return seenCalls.map((call) => ({
    path: call.url,
    method: call.method.toUpperCase(),
  }));
}

/** 控えた本文の媒体型。**`;charset=…` が付いていても本体だけを見る。** */
function sentContentTypes(): string[] {
  return seenCalls.map(
    (call) => (call.headers["content-type"] ?? "").split(";")[0]?.trim() ?? "",
  );
}

/** 1要素が「1回の読み取り」。 */
function streamOf(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let index = 0;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      const next = chunks[index];
      index += 1;
      if (next === undefined) {
        controller.close();
        return;
      }
      controller.enqueue(encoder.encode(next));
    },
  });
}

/** その読み取り列を流したときの結末と、届いた分。 */
async function run(chunks: string[]): Promise<{
  outcome: StreamOutcome;
  deltas: string[];
  models: string[];
}> {
  vi.stubGlobal("fetch", stubFetch(chunks));
  const seen = recorder();
  const outcome = await sendMessage(SENT_MODEL, SENT_CONTENT, seen, signal());
  return { outcome, deltas: seen.deltas, models: seen.models };
}

/** **本番の `sendMessage` が実際に送った本体。** 写しではない。 */
async function sentBody(): Promise<Record<string, unknown>> {
  vi.stubGlobal("fetch", stubFetch([]));
  await sendMessage(SENT_MODEL, SENT_CONTENT, recorder(), signal());

  const call = seenCalls[0];
  if (seenCalls.length !== 1 || call === undefined) {
    throw new Error(`送信が ${seenCalls.length} 回です`);
  }
  return asRecord(JSON.parse(call.body) as unknown);
}

/** 送った `messages` の1件目。**1件しか送らない**（`chat.ts` の doc）。 */
function firstMessageOf(body: Record<string, unknown>): Record<string, unknown> {
  const messages = body["messages"];
  if (!Array.isArray(messages) || messages.length !== 1) {
    throw new Error("messages が1件ではありません");
  }
  return asRecord(messages[0]);
}
