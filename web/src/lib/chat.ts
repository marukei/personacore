/**
 * **会話 API（`/v1/`）の型付き境界と、SSE の受け取り。**
 *
 * # ⚠ **この型は手で写したものである**（ADR-0031 §6）
 *
 * **生成していない。** 写し元は `src/server/chat.rs` の `ChatCompletionRequest` /
 * `IncomingMessage`、`src/server/relay.rs` の `Relay::relay`（チャンクの形）、
 * `src/server/error.rs` の `ApiError::as_event_data`（SSE の中のエラーの形）。
 * **写しの性質は ADR-0031 §6 が持っている。ここで書き足さない。**
 *
 * # **写しの綴りは、写し元を読んで突き合わせている**（作業単位 B-20）
 *
 * **見張り番は `tests/chat-wire.test.ts`。**
 * **範囲と「覆っていない範囲」はそのファイルの冒頭が権威。ここに写さない。**
 *
 * 見ているのは4つ ——
 * **送る本体の欄**（`ChatCompletionRequest` / `IncomingMessage` を1欄も選ばずに分類）／
 * **SSE のチャンクの鍵**（`Relay::relay` が載せる鍵を1つ残らず分類）／
 * **SSE の中のエラーの鍵**（`as_event_data`）／
 * **`model` の綴りの規則**（`selector.rs` の `NEW_SUFFIX` と区切り文字）。
 *
 * **⚠ 型検査はこの穴に1つも効かない**（B-20 が着手前に実測）。
 * **何をどれだけ壊してどうだったかは ADR-0031 §6 が持つ。ここに写さない。**
 *
 * # セッションの同定は `model` フィールド（ADR-0004）
 *
 * `aoi` = 最新の会話の継続 / `aoi@new` = 新規 / `aoi@<session-id>` = そのセッション。
 * **応答の `model` には「着地したセッション」が入る**ので、
 * **次の送信でそれをそのまま送れば、以後そのセッションに固定できる。**
 *
 * # `messages` は1件しか送らない
 *
 * **サーバは自前の DB を正とし、採用するのは最後の `user` だけ**
 * （`chat.rs` の `last_user_message`）。画面の履歴を全部送っても捨てられるので、
 * **送らない。** 送ると「送ったものが効いている」と読める形になる。
 *
 * # 失敗の伝わり方が2段ある
 *
 * **200 を送る前**（上流を開くまで）は HTTP のエラー応答（OpenAI 形の JSON）。
 * **送った後**は SSE の中に error オブジェクトが1件流れ、**`[DONE]` は来ない**
 * （`src/server/relay.rs`）。だから成否の手掛かりは2つある——
 * **error が流れたか / `[DONE]` が来たか。** ここは両方を見る。
 */

import { parseErrorBody, type ApiErrorDetail } from "./api";

/** `POST /v1/chat/completions` のパス。 */
const CHAT_PATH = "/v1/chat/completions";

/** 終端の番兵（`relay.rs` の `DONE_DATA`）。 */
const DONE_DATA = "[DONE]";

/** 新規セッションの予約語（ADR-0004）。 */
const NEW_SUFFIX = "@new";

/** そのキャラクターの**最新の会話を継続する** `model`。 */
export function continueLatest(handle: string): string {
  return handle;
}

/** そのキャラクターで**新しい会話を始める** `model`。 */
export function startNew(handle: string): string {
  return `${handle}${NEW_SUFFIX}`;
}

/**
 * そのキャラクターの**指定した会話を続ける** `model`（`aoi@<session-id>` / 作業単位 D-12c-2）。
 *
 * 過去の会話を一覧から選んだときに送る。**区切りは [`isSessionOf`] が見るものと同じ**
 * （食い違うと、選んだ直後の応答を「別の会話を名乗った」と読んで固定しない）。
 * 見張り番は `tests/chat-wire.test.ts`（`selector.rs` の区切りを実際に読む）。
 */
export function continueSession(handle: string, sessionId: number): string {
  return `${handle}@${sessionId}`;
}

/**
 * 応答が返した `model`（`aoi@<session-id>`）から、**着地した会話の id** を読む（作業単位 D-12c-2）。
 *
 * そのキャラクターの会話でない・id が10進の整数でない綴りは `null`（読めたことにしない）。
 * 画面はこれで「表示している会話」と「着地した会話」を突き合わせる。
 */
export function sessionIdOf(handle: string, model: string): number | null {
  if (!isSessionOf(handle, model)) {
    return null;
  }
  const suffix = model.slice(handle.length + 1);
  if (!/^\d+$/u.test(suffix)) {
    return null;
  }
  const id = Number(suffix);
  return Number.isSafeInteger(id) ? id : null;
}

/**
 * 応答が返した `model` が、こちらが指したキャラクターのものか。
 *
 * **食い違ったら固定しない。** セッションの取り違えは
 * 「別キャラクターの記憶が混ざる」という、ADR-0004 がいちばん避けたい壊れ方なので、
 * **黙って言われたとおりに固定するより、画面に出して止める。**
 */
export function isSessionOf(handle: string, model: string): boolean {
  return model.startsWith(`${handle}@`);
}

/** 送信中に分かること。 */
export interface StreamCallbacks {
  /** 増分が届いた。**空文字列のこともある**（役割だけの先頭チャンク）。 */
  onDelta: (text: string) => void;
  /** 着地したセッション（`aoi@<session-id>`）。**最初のチャンクで分かる。** */
  onModel: (model: string) => void;
}

/**
 * 1回の送信の結末。**握り潰さないための型**——
 * 呼び出し側は `switch` で全部に文言を割り当てる。
 */
export type StreamOutcome =
  /** `[DONE]` まで届いた。**この回だけがサーバに保存されている。** */
  | { kind: "done" }
  /** 中継の途中で失敗した（SSE の中の error オブジェクト）。 */
  | { kind: "upstream-error"; detail: ApiErrorDetail }
  /** 200 を送る前に断られた（HTTP のエラー応答）。 */
  | { kind: "rejected"; status: number; detail: ApiErrorDetail | null }
  /** そもそも繋がらなかった。 */
  | { kind: "unreachable" }
  /** **`[DONE]` もエラーも来ないまま終わった。** */
  | { kind: "truncated" }
  /** **読めない `data:` が来た。** 形が変わった可能性がある。 */
  | { kind: "unreadable" }
  /** 利用者が中止した。 */
  | { kind: "aborted" };

/**
 * **その回はサーバに保存されたか。**
 *
 * # ⚠ **保証があるのは片方向だけ**
 *
 * **`false`（＝ `done` 以外）⇒ 保存されていない。** ここは確か——
 * `src/server/relay.rs` は `[DONE]` を流す枝でしか保存へ進まないので、
 * 途中で壊れた回は**利用者の発言も含めて1行も書かれない**（ADR-0019 第4節）。
 *
 * **`true`（＝ `done`）⇒ 保存を「試みた」**、までしか言えない。
 * `src/server/turn.rs` の `persist` は `Store::append_turn` の失敗を
 * **`error!` に出して飲み込む**（`relay.rs` が「失敗しても中継の成否は変えられない」
 * と書いているとおり、そこはもう 200 を送り終えている）。
 * つまり **`[DONE]` が届いた回でも、書かれていないことがある。**
 *
 * **画面に出す文言は `false` の側にしか付けていない**ので、いまの実害はゼロ。
 * **だがこの向きで読むこと。** 「`done` なら保存されている」と読んで
 * 「保存しました」と書く文言を足すと、その瞬間に嘘になる。
 *
 * **画面には途中までの本文が残る**ので、言わないと画面と履歴が食い違ったままになる
 * （ADR-0019 が「未解決の緊張」と呼んでいるもの。**解いてはいないが、隠しもしない**）。
 */
export function isPersisted(outcome: StreamOutcome): boolean {
  return outcome.kind === "done";
}

/**
 * 結末を1文にする。**`done` 以外は必ず何か返る**——
 * `switch` の網羅は型検査が見ている（返り値の型が `undefined` を含まないので、
 * 枝を足し忘れるとコンパイルが通らない）。
 *
 * **サーバが言ったこと以上を推測しない。** `upstream-error` / `rejected` の文言は
 * サーバの定数そのままで、こちらは足さない（絶対制約: エラーに内部の詳細を載せない）。
 *
 * # ⚠ **空白だけの `message` は「無い」として扱う**（作業単位 B-14）
 *
 * `??` は `null` / `undefined` しか見ないので、**`message: ""` が来ると空文字列を採る。**
 * 画面側の `{#if notice !== null}` は空文字列を通すので、
 * **中身の空の `<p class="error">`** が出る——
 * **「エラーがあった」という枠だけを出して何も言わない**、いちばん悪い見え方になる。
 *
 * **判定は `trim()`。** 同じ日に `ChatView.svelte` が「空の応答」を
 * `turn.raw.trim() === ""` へ寄せたのと同じ寄せ方で、**空白だけも「無い」に含める。**
 * **文面そのものは書き換えない**（`trim()` は判定にだけ使い、返すのは元の文字列）。
 *
 * **⚠ いまの実害はゼロ。** サーバの `message` は `error.rs` の定数
 * （`&'static str`）で、空になる経路は無い。**それでも塞ぐ理由は2つ:**
 * **`error.rs` は文言を実際に改訂した記録を持っている**
 * （`character_already_exists` の「名前」→「識別子」）ことと、
 * **上流が personacore とは限らない**こと（ADR-0036 決定4）。
 *
 * **⚠ 塞いだのは `rejected` だけ。** `upstream-error` の `detail.message` は
 * **素通しのまま**で、そちらが空白だけなら同じ空の `<p class="error">` が出る
 * （作業単位 B-14 のスコープ外。**塞いだと読まないこと**）。
 *
 * # ⚠ **文言は3つある。`detail` の有無と `message` の中身は別の事実**（B-14 のレビュー）
 *
 * | 状態 | 何が起きたか | 出す文 |
 * |---|---|---|
 * | `detail === null` | **本文そのものが読めなかった** | 「本文を読めませんでした」 |
 * | `message` が空白だけ | **本文は読めた。中身が無かった** | 「理由は書かれていませんでした」 |
 * | `message` に見える文字がある | 理由が書かれていた | **その文をそのまま** |
 *
 * **⚠ 2つを同じ文に潰さないこと。** 空白だけの回に「本文を読めませんでした」と出すのは
 * **事実に反する**——本文は読めていて、**そこに理由が書かれていなかった。**
 * **このリポジトリの立場は「サーバが言ったこと以上を推測しない」**であり、
 * **起きていないことを画面が断言するのは、その反対側**（同じ日の D-7 と同じ形）。
 *
 * **⚠ 「サーバが理由を言わなかった」とも書かない。** こちらに分かるのは
 * **受け取った本文に理由が書かれていなかった**ことだけで、**上流が何を言ったかではない。**
 *
 * **見張り番は `tests/chat-view-notices.test.ts` の「断られた回の文言」**——
 * **3つが互いに違う文になること**（同じ状態番号で突き合わせる）と、
 * **「読めません」と名乗ってよいのは `detail === null` の回だけ**を押さえている。
 * **2つを同じ文にする変異と、3つを1つに潰す変異の両方**を当ててある。
 */
export function outcomeNotice(outcome: StreamOutcome): string | null {
  switch (outcome.kind) {
    case "done":
      return null;
    case "upstream-error":
      return outcome.detail.message;
    case "rejected": {
      const detail = outcome.detail;
      if (detail === null) {
        // **本文そのものが読めなかった。** 何が書いてあったかを知る手段が無い。
        return `サーバが ${outcome.status} を返しました（本文を読めませんでした）`;
      }
      if (detail.message.trim() === "") {
        // **本文は読めた。理由が書かれていなかった。**
        // ここで「読めませんでした」と言うと、起きていないことを断言することになる。
        return `サーバが ${outcome.status} を返しましたが、理由は書かれていませんでした`;
      }
      return detail.message;
    }
    case "unreachable":
      return "サーバへ接続できませんでした（personacore は動いていますか）";
    case "truncated":
      return "応答が完結する前に接続が終わりました";
    case "unreadable":
      return "サーバから読めない応答が届きました（API の形が変わった可能性があります）";
    case "aborted":
      return "中止しました";
  }
}

/**
 * **結末に載っていたエラーの `code`**（無ければ `null`）。
 *
 * **200 の前に断られた回**（`rejected`）と**流れの途中のエラー**（`upstream-error`）のどちらにも `code` は載りうる。
 * 会話の画面は、これで鍵の失敗（`upstream_unauthorized` / `upstream_key_not_approved`）を読み分けて案内を足す
 * （作業単位 A-13b-3 / `upstreamKey.ts` の `keyFailureOf`）。**文面（`outcomeNotice`）は変えない。**
 */
export function outcomeCode(outcome: StreamOutcome): string | null {
  switch (outcome.kind) {
    case "upstream-error":
      return outcome.detail.code;
    case "rejected":
      return outcome.detail?.code ?? null;
    default:
      return null;
  }
}

/** SSE のイベント1件から取り出したもの。 */
type Frame =
  | { kind: "done" }
  | { kind: "error"; detail: ApiErrorDetail }
  | { kind: "delta"; text: string; model: string | null }
  | { kind: "unreadable" };

/**
 * `data:` の中身を読む。**`as` で押し込まない。**
 *
 * `[DONE]` → エラー → チャンク の順に見る。**どれでもなければ `unreadable`**
 * （黙って捨てると、Rust 側が形を変えたときに「応答が空になる」という
 * 原因から遠い形で現れる）。
 */
function readFrame(data: string): Frame {
  if (data === DONE_DATA) {
    return { kind: "done" };
  }

  const detail = parseErrorBody(data);
  if (detail !== null) {
    return { kind: "error", detail };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(data) as unknown;
  } catch {
    return { kind: "unreadable" };
  }
  if (typeof parsed !== "object" || parsed === null) {
    return { kind: "unreadable" };
  }

  const record = parsed as Record<string, unknown>;
  const choices = record["choices"];
  if (!Array.isArray(choices)) {
    return { kind: "unreadable" };
  }

  const model = typeof record["model"] === "string" ? record["model"] : null;

  const first: unknown = choices[0];
  if (typeof first !== "object" || first === null) {
    // `choices: []` は本家の使用量チャンクの形。**中継はこれを下流へ出さない**
    // （`relay.rs` の `is_usage_only`）が、来ても本文としては何も足さない。
    return { kind: "delta", text: "", model };
  }
  const delta: unknown = (first as Record<string, unknown>)["delta"];
  if (typeof delta !== "object" || delta === null) {
    return { kind: "unreadable" };
  }
  const content: unknown = (delta as Record<string, unknown>)["content"];
  if (content === undefined || content === null) {
    return { kind: "delta", text: "", model };
  }
  if (typeof content !== "string") {
    return { kind: "unreadable" };
  }
  return { kind: "delta", text: content, model };
}

/**
 * SSE の行を組み立てて `data:` を取り出す。
 *
 * **空行がイベントの区切り**、`:` で始まる行はコメント、`data:` の後ろの空白は1つだけ落とす
 * （[SSE の仕様](https://html.spec.whatwg.org/multipage/server-sent-events.html)）。
 * **複数行の `data:` は改行で繋ぐ**——いまのサーバは1行しか出さないが、
 * 仕様の側に合わせておく方が、増えたときに黙って壊れない。
 *
 * **末尾の空行が来ないまま終わったイベントは捨てる**（仕様どおり）。
 * 捨てたことは `[DONE]` が来ないことで外から分かる。
 */
class SseDecoder {
  private buffer = "";
  private data: string[] = [];

  /** 受け取った文字列を足して、**完成したイベントの `data:` だけ**返す。 */
  push(text: string): string[] {
    this.buffer += text;
    const lines = this.buffer.split("\n");
    // 最後の要素は改行で終わっていない = まだ途中。
    this.buffer = lines.pop() ?? "";

    const events: string[] = [];
    for (const raw of lines) {
      const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
      if (line === "") {
        if (this.data.length > 0) {
          events.push(this.data.join("\n"));
          this.data = [];
        }
        continue;
      }
      if (line.startsWith(":")) {
        continue;
      }
      if (line.startsWith("data:")) {
        const value = line.slice("data:".length);
        this.data.push(value.startsWith(" ") ? value.slice(1) : value);
      }
      // それ以外のフィールド（`event:` / `id:` / `retry:`）は使わない。
    }
    return events;
  }
}

/** 送信するもの。**`stream` は常に `true`**（この画面は逐次表示しかしない）。 */
interface WireRequest {
  model: string;
  messages: { role: "user"; content: string }[];
  stream: true;
}

/**
 * 送って、届いた分から `callbacks` に渡す。**投げない**——
 * 結末は必ず [`StreamOutcome`] で返る。
 *
 * **握り潰さないための約束**: `done` 以外はすべて「その回は保存されていない」。
 * サーバは `[DONE]` を流す枝でだけ保存する（`src/server/relay.rs`）。
 */
export async function sendMessage(
  model: string,
  content: string,
  callbacks: StreamCallbacks,
  signal: AbortSignal,
): Promise<StreamOutcome> {
  const body: WireRequest = {
    model,
    messages: [{ role: "user", content }],
    stream: true,
  };

  let response: Response;
  try {
    response = await fetch(CHAT_PATH, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        accept: "text/event-stream",
      },
      body: JSON.stringify(body),
      signal,
    });
  } catch {
    return signal.aborted ? { kind: "aborted" } : { kind: "unreachable" };
  }

  if (!response.ok) {
    const text = await response.text();
    return {
      kind: "rejected",
      status: response.status,
      detail: parseErrorBody(text),
    };
  }

  const stream = response.body;
  if (stream === null) {
    return { kind: "unreadable" };
  }

  const reader = stream.getReader();
  const decoder = new TextDecoder();
  const sse = new SseDecoder();

  try {
    for (;;) {
      const step = await reader.read();
      if (step.done) {
        // **`[DONE]` を受けずに終わった。** 完結していない。
        return { kind: "truncated" };
      }
      const events = sse.push(decoder.decode(step.value, { stream: true }));
      for (const data of events) {
        const frame = readFrame(data);
        switch (frame.kind) {
          case "done":
            return { kind: "done" };
          case "error":
            return { kind: "upstream-error", detail: frame.detail };
          case "unreadable":
            return { kind: "unreadable" };
          case "delta":
            if (frame.model !== null) {
              callbacks.onModel(frame.model);
            }
            if (frame.text !== "") {
              callbacks.onDelta(frame.text);
            }
            break;
        }
      }
    }
  } catch {
    return signal.aborted ? { kind: "aborted" } : { kind: "truncated" };
  } finally {
    // **読むのをやめたら上流も落ちる**（axum が本文のストリームを落とし、
    // その先の `ChatStream` の drop が LM Studio への接続を閉じる / ADR-0019）。
    await reader.cancel().catch(() => undefined);
  }
}
