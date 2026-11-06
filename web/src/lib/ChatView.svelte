<script lang="ts">
  import {
    ApiRequestError,
    getUpstream,
    listCharacters,
    listSessionMessages,
    listSessions,
    switchUpstreamProfile,
    type CharacterView,
    type MessageView,
    type SessionListView,
    type SessionSummaryView,
    type UpstreamReport,
  } from "./api";
  import { bodyFaceStatus } from "./bodyFace";
  import {
    START_CAUTION,
    startButtonLabel,
    startFailureLabel,
    startingLabel,
  } from "./upstreamProfiles";
  import { keyFailureGuidance, keyFailureOf } from "./upstreamKey";
  import { activeCharacters, archivedCharacters } from "./characterArchive";
  import { browserZone } from "./timestamp";
  import {
    continueLatest,
    continueSession,
    isPersisted,
    isSessionOf,
    sessionIdOf,
    outcomeCode,
    outcomeNotice,
    sendMessage,
    startNew,
    type StreamOutcome,
  } from "./chat";
  import { reasonLabel, split } from "./dialogue";
  import { stripThinking } from "./thinking";
  // **綴りは1箇所**（`router.ts`）。空状態の案内が指す先も、ナビと同じ入口から作る。
  import { hrefFor, labelFor } from "./router";
  import ProposalPrompt from "./ProposalPrompt.svelte";
  import SessionPicker from "./SessionPicker.svelte";

  let {
    pollIntervalMs = 2000,
    idlePollIntervalMs = 15000,
    proposalRecheckMs = 30000,
    proposalRechecks = 3,
  }: {
    /**
     * **本文の面が載っていない（送信を止めている）間に、状態を読み直す間隔**（ミリ秒 / 作業単位 D-16）。
     *
     * **`#/upstream` の画面（`UpstreamPanel.svelte` の `pollIntervalMs`）と同じ値。**
     * **測定由来の値ではない**（速く読み直しても、載るのは速くならない）。
     * 外しても「送れるようになるのが少し遅れて見える」だけで、何も壊れない。テストは短くして回す。
     */
    pollIntervalMs?: number;
    /**
     * **止めていない間に、状態を読み直す間隔**（ミリ秒）。
     *
     * **止めていない間も読む**のは、この画面を開いたまま別のタブ・外のクライアントから
     * 差し替えられたとき（準備完了 → 載せている）に、送信を止めるため。
     * **長くしてある** —— 開いているだけで口を叩き続けないように。外しても「気付くのが遅れる」だけ
     * （その間に送った回は、今までどおり上流エラーの形で返る）。
     */
    idlePollIntervalMs?: number;
    /**
     * **約束・決まりの提案を、ターンの後に読み直す間隔**（ミリ秒 / 作業単位 A-12c-2）。
     *
     * 理由と回数の決め方は `ProposalPrompt.svelte` の doc。**測定由来の値ではない** ——
     * 外しても「問いかけが出るのが遅れる」か「少し多く読む」だけで、何も壊れない。テストは短くして回す。
     */
    proposalRecheckMs?: number;
    /** **ターンの後に読み直す回数**（同上）。 */
    proposalRechecks?: number;
  } = $props();

  /**
   * **本文の面の状態**（`GET /admin/upstream` / 作業単位 D-16 / ADR-0042 §決定3 の追記）。
   *
   * **会話の画面は、`#/upstream` の画面とは別に自分で読む**（画面をまたいで持ち越す持ち主が無い。
   * `App.svelte` は画面ごとに部品を作り直す）。**読み方は `UpstreamPanel.svelte` と同じ作法** ——
   * 生きている間の中断ハンドル（`lifetime`）で落とし、古い読みは番号（`upstreamTicket`）で捨てる。
   *
   * # 読み直しの上限と後片付け
   *
   * * **同時に飛ぶ読みは1本まで。** 次の1回は、前の読みが着いてから予約し直す（`upstreamReads`）。
   *   **間隔より短くは読まない**（止めている間 `pollIntervalMs`、それ以外 `idlePollIntervalMs`）。
   * * **回数の上限は付けない**（`UpstreamPanel.svelte` と同じ理由）—— 20 GB の gguf は載るまで1〜2分かかり、
   *   載らないまま黙る子もサーバは殺さない（ADR-0035）。**回数で読むのをやめると、
   *   載った後も古い「載せています」で送信を止め続ける**（外したときに壊れる側。ADR-0036）。
   * * **画面を離れたら止める**（予約を消し、飛んでいる読みを落とす）。
   */
  let upstreamReport = $state<UpstreamReport | null>(null);
  let upstreamError = $state<string | null>(null);
  let upstreamReadAt = $state<Date>(new Date());
  /** **着いた読みの数。** 次の1回を予約し直す合図（同じ値が着いても予約し直すため）。 */
  let upstreamReads = $state(0);
  /** いちばん新しい読みの番号（描画には使わない）。 */
  let upstreamTicket = 0;
  /**
   * **起こす依頼の応答を待っているプロファイルの名前**（作業単位 F-10b / ADR-0055 §決定3）。
   * `null` は待っていない。待っている間は、起こすボタンを塞ぐ。
   */
  let starting = $state<string | null>(null);
  /** 起こす依頼が断られた理由（409 等。次に押したら消す）。 */
  let startError = $state<string | null>(null);

  /**
   * **画面に出している、サーバに保存された過去のやり取り**（作業単位 D-12c-2 / Issue origin#4）。
   *
   * # 何を出すか
   *
   * | 送り先 | 出すもの |
   * |---|---|
   * | `aoi`（最新の続き） | **会話の一覧の先頭**（サーバが素の `aoi` の続け方と同じ順で返す） |
   * | `aoi@<id>`（一覧から選んだ） | **その会話** |
   * | `aoi@new` | **何も出さない**（新しい会話に過去は無い） |
   *
   * **いまの回（`turns`）とは別に持つ。** `turns` は「この画面で送った回」で、
   * 保存されていない回（中止・失敗）も含む。**ここは保存されたものだけ。**
   * 混ぜると「保存されていません」の注記がどちらの回の話か分からなくなる。
   *
   * # 本文の整え方は、実行中の返事と同じ
   *
   * 応答は **`raw_content` を `replyBody` に通す**（実行中の返事が `[DONE]` の後に通るのと同じ snippet /
   * `dialogue.ts` の `split`）。**保存された `dialogue` / `narration` の列は読まない** ——
   * 読むと、同じ返事がいま届いたときと読み直したときで違って見える（`api.ts` の `MessageView`）。
   * 見張り番は `tests/chat-view-history.test.ts` の「実行中の返事と、読み直した返事が同じに見える」。
   */
  interface History {
    /** どの会話のものか。 */
    sessionId: number;
    /** **古い順。** 遡ったら先頭へ足す。 */
    messages: MessageView[];
    /** もっと前があるか（サーバが言う）。 */
    hasOlder: boolean;
  }

  let history = $state<History | null>(null);
  let historyError = $state<string | null>(null);
  let historyLoading = $state(false);
  let olderLoading = $state(false);
  /** **過去の会話の一覧**（開いている間だけ持つ。開くたびに読み直す）。 */
  let picker = $state<SessionListView | null>(null);
  let pickerError = $state<string | null>(null);
  /** **一覧から選んだ会話**（選んでいなければ `null`）。画面が「この会話の続きになる」と言うのに使う。 */
  let chosen = $state<number | null>(null);
  /**
   * **このキャラクターに過去の会話が1つも無いと分かっている**（作業単位 D-17 のレビュー）。
   *
   * 「新しい会話を始める」の一言の後半（「前の会話は『過去の会話を選ぶ』から読めます」）を出すかに使う。
   * **分かっているときだけ `true`**（一覧が空だった）。読めていない・読めなかったときは `false` のまま
   * ——後半を出しても、一覧を開けば「まだ会話がありません」と言うだけで、嘘にはならない。
   * **着地したら `false` に戻す**（その時点で会話が1つできている）。
   * キャラクター単位の事実なので、`forgetHistory` では戻さない（`selectCharacter` で戻す）。
   */
  let noPastSessions = $state(false);
  /**
   * **返事が、表示していた会話とは別の会話に着地したことの報せ**（D-12c のレビュー / 司令塔の決定）。
   *
   * 素の `aoi` は「送った時点の最新」へ着地する。開いた後に**別のタブや外のクライアント**が
   * 別の会話に書くと、この画面が出していた過去（X）と、返事が保存された会話（Y）が分かれる。
   * **黙っていると、X の過去の下に Y の返事が並ぶ**（どの会話の続きか読めない）。
   * → 一言告げ、Y を読み直して差し替える（`reloadLanded`）。
   *
   * **居座らせない** —— 次に送った返事が、表示している会話に着地したら消す（`sessionWarning` と同じ理由）。
   * 見張り番は `tests/chat-view-history.test.ts` の「送った返事が、出している会話と別の会話に着地したとき」。
   */
  let historyNotice = $state<string | null>(null);
  /**
   * **いま有効な読み込みの番号。** キャラクターを変えた・新しい会話を始めた・別の会話を選んだら進める。
   *
   * 遅れて届いた前の読み込みの結果を捨てるため —— 捨てないと、
   * **別のキャラクターの過去が、いまのキャラクターの画面に出る。**
   * `$state` にしないのは、画面に出すものではないから。
   */
  let historyTicket = 0;

  /**
   * 画面に並ぶ1件。
   *
   * **失敗した回も消さない。** サーバは保存していない（`src/server/relay.rs`）が、
   * **画面から消すと「何が起きたか」が消える。** 残したうえで、
   * 保存されていないことを明示する。
   */
  type Turn =
    | { role: "user"; text: string }
    | {
        role: "assistant";
        /** **生出力そのもの。** 分解は表示のときにだけ行う。 */
        raw: string;
        /** `null` は受信中。 */
        outcome: StreamOutcome | null;
      };

  let characters = $state<CharacterView[] | null>(null);
  /** **しまったキャラクターの数**（作業単位 A-10）。相手が居ないときの文言を分けるためだけに持つ。 */
  let archivedCount = $state(0);
  let charactersError = $state<string | null>(null);

  let handle = $state<string | null>(null);
  /**
   * **着地したセッション**（`aoi@<session-id>`）。応答が返してきた値。
   *
   * ここが埋まったら以後はこれを送る（ADR-0004 の「セッションを固定する」）。
   *
   * # ⚠ **固定できるのは、本文が1件でも届いた回だけ**
   *
   * 埋まるのは `onModel` が呼ばれた回、つまり**サーバが 200 を送り、
   * チャンクが1件届いた後**に限る。**200 の前に断られた回**——
   * `StreamOutcome` で言う `rejected` と `unreachable`——**は固定できない。**
   * `pinned` は `null` のまま、`fresh` も `true` のまま残る。
   *
   * **そしてその回の空セッションは、もう出来てしまっている。**
   * `src/server/chat.rs` はセッションの作成（`resolve_session`）を
   * **上流を開くより前**に行うので、上流が開けなかった回でも行は残る。
   * つまり **`@new` を送り直すたびに空のセッションが増える。**
   *
   * **⚠ かつては、その空セッションが「最新」になっていた**（`INSERT` でも `updated_at` が
   * 押し上がるため / ADR-0004）。**素の `aoi` へ戻ると、それまでの会話ではなく、
   * 空セッションの続きに着地した。**
   *
   * **→ 2026-08-27 に直った**（作業単位 B-2b）。**「最新」は「中身のある最新」になり、
   * 空セッションは解決の順序の第1キーで降格する**（ADR-0004 の末尾の追記）。
   * **空セッションが出来ること自体は変わらない**——ADR-0019 §4 により、
   * **中止・打ち切りの回も1行も保存されない**ので、正常系でも残る。
   * **変わったのは、それが継続の対象から外れたこと。**
   *
   * **⚠ 「FE 側だけでは直せない」という判断はそのまま。** 200 を送る前のエラー応答は
   * `model` を返さない（形は `error.rs` の `ErrorBody` だけ）ので、
   * **着地したセッションを知る手段が無い。** だから `pinned` は `null` のままにする。
   *
   * **`rejected` の後に `fresh` を降ろす、という手は採らない。**
   * 利用者が明示した「新しい会話」を黙って捨てることになり、
   * ADR-0004 が避けようとしている「意図しない分断・結合が起き、
   * しかも気づきにくい」に真っ向から当たる。
   */
  let pinned = $state<string | null>(null);
  /** 次の送信で新しい会話を始める。 */
  let fresh = $state(false);
  /**
   * **応答が別のキャラクターのセッションを名乗った。**
   *
   * ADR-0004 がいちばん避けたい壊れ方なので、黙って固定せずに画面へ出す。
   *
   * # ⚠ **状態を名乗るのは送り先の表示。警告は「拒んだこと」だけを言う**（B-14 のレビュー）
   *
   * **「いま何を送ろうとしていて、それが何を意味するか」は `p.wire` が言っている**
   * （`outgoingModel` / `outgoingMeaning`）。**警告がそこへ踏み込むと、権威が2つになる。**
   *
   * **⚠ 実際に食い違う。** かつての文面は「セッションを**固定しませんでした**。」だったが、
   * **すでに固定した後で、別のキャラクターのセッションを名乗られた回**では、
   * 画面が**同時に**こう言う ——
   * 「セッションを固定しませんでした」／「送り先: `aoi@41`（**このセッションの続き**）」。
   * **その回について読めば真だが、状態として読めば偽。**
   * **読み分けを利用者に要求している時点で、画面の負けである。**
   *
   * **→ 文面は「そこへは切り替えませんでした。」。**
   * **固定済みでも未固定でも真**になり、**送り先の表示と競合しない。**
   *
   * **見張り番は `tests/chat-view-notices.test.ts` の
   * 「**固定した後に別キャラを名乗られても**、警告は送り先と食い違わない」**——
   * **同じ回に、警告の文面と送り先の表示の両方を見ている。**
   * **片方だけを見ると、食い違いはどちらの側からも観測できない。**
   *
   * # ⚠ それでも、居座らせない（作業単位 B-14 の決定）
   *
   * **異常が続いていないのに警告だけが残るのは、それ自体が古い情報**で、
   * **利用者は「まだ壊れている」と読む。**
   *
   * **→ `onModel` が固定に成功した枝で `null` に戻す。**
   * **それ以外では戻さない。** 異常が続いている間は出したままにする。
   *
   * **⚠ 逆の読み方（「起きたことを消さない」）も成り立つので、決めた理由を書く。**
   * 消さない側を採るなら、**それを載せる場所はここではない。**
   * ここは**1本しか出ない欄**であって、履歴ではない。
   * **起きた回そのものは `turns` に残っている**（届いた本文も一緒に）。
   *
   * # 異常が2回続いたら、**新しい文面に置き換える**（積み上げない）
   *
   * 名乗られたセッションの綴りは回ごとに違いうるが、**2本並べると
   * 「いま何が起きているのか」がかえって読み取れない。**
   * **いちばん新しい着地について1本だけ言う。**
   *
   * # ⚠ `null` に戻す場所は**3つある**
   *
   * `selectCharacter` / `beginNewSession` / **`onModel` の固定に成功した枝。**
   * **3つとも `tests/chat-view-notices.test.ts` が観測している。**
   * 1つでも観測から漏れると、その枝を落とす変異が緑を通る
   * （同じファイルで実際に起きた——`turn.raw === ""` の2箇所のうち片方だけを塞いだ）。
   */
  let sessionWarning = $state<string | null>(null);

  let turns = $state<Turn[]>([]);
  let draft = $state("");
  /**
   * **この画面で保存まで届いたターンの数**（作業単位 A-12c-2）。**約束・決まりの提案を取りに行く合図**
   * （`ProposalPrompt.svelte` の `turnsEnded`）。
   *
   * **数えるのは `[DONE]` まで届いた回だけ**（`isPersisted`）—— 届かなかった回は保存されず、
   * 裏方（提案を作る鎖）も走らないので、読みに行っても何も増えていない。
   * **キャラクターを変えても戻さない**（問いかけは相手ごとに作り直され、作り直した時点の数を「もう見た」とする）。
   */
  let persistedTurns = $state(0);

  /** 送信中の開始時刻（`null` なら送信していない）。 */
  let startedAt = $state<number | null>(null);
  let now = $state(Date.now());
  let controller: AbortController | null = null;
  /**
   * **この画面が生きている間**の中断ハンドル。破棄されたら落とす。
   *
   * 送信中の `controller` と分けてあるのは、**「中止」ボタンで
   * キャラクター一覧の取得まで落とさない**ため。
   */
  const lifetime = new AbortController();

  const inFlight = $derived(startedAt !== null);
  const elapsed = $derived(
    startedAt === null ? 0 : Math.max(0, Math.floor((now - startedAt) / 1000)),
  );

  /** **本文の面の一言と、送信を止めるか**（判定は `bodyFace.ts`）。 */
  // **読んだ時刻を「いま」として渡す**（Issue origin#40）—— 経過は読み直しのたびに進む。
  const readerZone = browserZone();
  const face = $derived(bodyFaceStatus(upstreamReport, upstreamError, upstreamReadAt, readerZone));

  /** 実際に送る `model`（ADR-0004）。 */
  const outgoingModel = $derived.by(() => {
    if (handle === null) {
      return null;
    }
    if (pinned !== null) {
      return pinned;
    }
    return fresh ? startNew(handle) : continueLatest(handle);
  });

  /** その `model` が何を意味するか。**画面に出す**（黙って決めない）。 */
  const outgoingMeaning = $derived.by(() => {
    if (pinned !== null) {
      return "このセッションの続き";
    }
    return fresh ? "新しい会話" : "最新の会話の続き";
  });

  // 経過秒。**「反応が無い」と思わせないため**に、最初の1文字が来るまでも数え続ける。
  $effect(() => {
    if (startedAt === null) {
      return;
    }
    const timer = setInterval(() => {
      now = Date.now();
    }, 1000);
    return () => clearInterval(timer);
  });

  /**
   * **画面を離れたら、走っている要求を落とす。**
   *
   * 落とさないと、`sendMessage` は `ChatView` が破棄された後も読み続ける。
   * **中止ボタンごと消えるので止められず、その回は保存されるのに画面のどこにも出ない。**
   * 戻ってすぐ送信すると**2本が同時に走る。**
   *
   * # ⚠ **中断すると、その回は1行も保存されない**（ADR-0019 第4節）
   *
   * つまり**利用者の1ターンを捨てている。** それでもこちらを選ぶ理由:
   *
   * **走らせ切る側は「保存されたのに画面に出ない」を作る。**
   * ~~**この画面には履歴を読み込む導線がまだ無い**ので、戻っても見えない。~~
   * そして次に送ると、**モデルは「利用者が見ていないやり取り」を踏まえて答える。**
   * **黙って食い違う方が悪い。**
   *
   * # 履歴を読む導線ができた後の見直し（作業単位 D-12c-2 / **中断のままにした**）
   *
   * **いまは戻ると過去のやり取りが出る**（`history`）。消えた理由は「戻っても見えない」の1つだけで、
   * **残りの2つは今も成り立つ:**
   *
   * * **保存されるのは `[DONE]` の後。** 戻った瞬間に読む履歴には、走っている回はまだ無い。
   *   **戻ってすぐ送ると2本が同時に走り**、しかも画面にはその回が出ていない。
   *   読み直しを「走り終わるまで待つ」形にするには、画面を離れた後も誰かが待ち続ける必要がある。
   * * **中止ボタンごと消えるので、止められない。**
   *
   * **→ 1ターンを捨てる代償は変わらないが、食い違いの窓を作らない方を採る。**
   * 変えるなら、画面を越えて生きる送信の持ち主（`App.svelte` 側）を先に作ること。
   */
  $effect(() => {
    return () => {
      controller?.abort();
      lifetime.abort();
    };
  });

  /**
   * **本文の面の状態を読み直す**（上の `upstreamReport` の doc）。
   *
   * **読めなかったら、前に読めた値を捨てる** —— 古い「載せています」で送信を止め続けない
   * （状態を知らないなら止める根拠が無い。`bodyFace.ts`）。
   */
  async function readUpstream(): Promise<void> {
    upstreamTicket += 1;
    const mine = upstreamTicket;
    try {
      const next = await getUpstream(lifetime.signal);
      if (lifetime.signal.aborted || mine !== upstreamTicket) {
        return;
      }
      upstreamReadAt = new Date();
      upstreamReport = next;
      upstreamError = null;
    } catch (error) {
      // **自分で落とした分・古い読みの分は書かない**（画面はもう無い／新しい読みが書く）。
      if (lifetime.signal.aborted || mine !== upstreamTicket) {
        return;
      }
      upstreamReport = null;
      upstreamError = messageOf(error);
    }
    upstreamReads += 1;
  }

  /**
   * **起こしていない本文の面を起こす**（作業単位 F-10b / ADR-0055 §決定3）。
   *
   * **いま選ばれているプロファイルと同じ名前で `POST /admin/upstream/profile` を送る**（名前は
   * `bodyFace.ts` が `selected_profile` から決める）。経路は差し替えと1つで、**応答（起こした面は
   * `loading`）を、いまの状態としてそのまま採る** —— 止める理由は「載せている最中」へ移り、
   * 以後は今までどおり読み直して載るのを待つ。
   *
   * **番号を進める**（`UpstreamPanel.svelte` の `adopt` と同じ）—— 押す前に出した読み直しが後から着いても、
   * 古い「起こしていない」へ巻き戻さない。**断られたら（409 等）サーバの文面をそのまま言う**
   * （`#/upstream` の差し替えの失敗と同じ形）。
   */
  async function startUpstream(name: string): Promise<void> {
    starting = name;
    startError = null;
    try {
      const next = await switchUpstreamProfile(name, lifetime.signal);
      if (lifetime.signal.aborted) {
        return;
      }
      upstreamTicket += 1;
      upstreamReadAt = new Date();
      upstreamReport = next;
      upstreamError = null;
    } catch (error) {
      if (lifetime.signal.aborted) {
        return;
      }
      startError = startFailureLabel(messageOf(error));
    } finally {
      starting = null;
    }
  }

  // **次の1回を予約する。** 読みが着くたびに予約し直す。止めている間だけ短い間隔。
  // **画面を離れたら、戻り値の後片付けで予約を消す**（飛んでいる読みは `lifetime` が落とす）。
  $effect(() => {
    void upstreamReads;
    const interval = face.hold !== null ? pollIntervalMs : idlePollIntervalMs;
    const timer = setTimeout(() => {
      void readUpstream();
    }, interval);
    return () => {
      clearTimeout(timer);
    };
  });

  function messageOf(error: unknown): string {
    if (error instanceof ApiRequestError) {
      return error.message;
    }
    return "予期しない失敗です";
  }

  async function loadCharacters(): Promise<void> {
    charactersError = null;
    try {
      const loaded = await listCharacters(lifetime.signal);
      // **しまったキャラクターは相手に出さない**（作業単位 A-10 / 出し分けは `characterArchive.ts`）。
      // 選べても、送れば 409 で断られる。**最初に開く相手も、しまっていない方から選ぶ。**
      const talkable = activeCharacters(loaded.characters);
      characters = talkable;
      // **しまったキャラクターしか居ないかを言い分けるため**に数だけ持つ（中身は出さない）。
      archivedCount = archivedCharacters(loaded.characters).length;
      if (handle === null) {
        handle = talkable[0]?.handle ?? null;
        // **開いたら、続ける会話の過去を出す**（作業単位 D-12c-2）。
        if (handle !== null) {
          void openLatest(handle);
        }
      }
    } catch (error) {
      // **自分で落とした分は失敗として書かない。**
      // 「取得できませんでした」と出すと嘘になる（しかも画面はもう無い）。
      if (lifetime.signal.aborted) {
        return;
      }
      characters = null;
      charactersError = messageOf(error);
    }
  }

  /** キャラクターを変えた。**会話は別物になる**ので画面も畳む。 */
  function selectCharacter(next: string): void {
    handle = next;
    pinned = null;
    fresh = false;
    sessionWarning = null;
    noPastSessions = false;
    turns = [];
    forgetHistory();
    void openLatest(next);
  }

  /**
   * 新しい会話を始める（次の送信が `aoi@new` になる / ADR-0004）。**過去は出さない。**
   *
   * **入口は2つ**: 見出しの釦と、過去の会話の一覧の釦（作業単位 D-17）。**一覧は畳む**（`forgetHistory`）。
   * 押したことは帯の下の一言が言う（`fresh` の間だけ）。
   */
  function beginNewSession(): void {
    pinned = null;
    fresh = true;
    sessionWarning = null;
    turns = [];
    forgetHistory();
  }

  /**
   * **出している過去を畳む。** 走っている読み込みの結果も捨てる（`historyTicket` を進める）。
   */
  function forgetHistory(): void {
    historyTicket += 1;
    history = null;
    historyError = null;
    historyLoading = false;
    olderLoading = false;
    picker = null;
    pickerError = null;
    chosen = null;
    historyNotice = null;
  }

  /**
   * **素の `aoi` が続ける会話**の過去を出す。
   *
   * **続ける会話は一覧の先頭**（サーバが `Store::latest_session` と同じ順で返す）。
   * **画面で選び直さない** —— 選び直すと、見せている会話と本文が書き込まれる会話が分かれる。
   */
  async function openLatest(character: string): Promise<void> {
    const ticket = ++historyTicket;
    historyLoading = true;
    historyError = null;
    try {
      const list = await listSessions(character, lifetime.signal);
      if (ticket !== historyTicket) {
        return;
      }
      noPastSessions = list.sessions.length === 0;
      const latest = list.sessions[0];
      if (latest === undefined) {
        // **会話がまだ1つも無い。** 出すものが無いだけで、失敗ではない。
        history = null;
        return;
      }
      await showSession(character, latest.id, ticket);
    } catch (error) {
      historyFailed(error, ticket);
    } finally {
      if (ticket === historyTicket) {
        historyLoading = false;
      }
    }
  }

  /** 一覧から選んだ会話を開く。**次の送信は `aoi@<id>`（その会話の続き）になる。** */
  async function openChosen(character: string, sessionId: number): Promise<void> {
    const ticket = ++historyTicket;
    historyLoading = true;
    historyError = null;
    history = null;
    try {
      await showSession(character, sessionId, ticket);
    } catch (error) {
      historyFailed(error, ticket);
    } finally {
      if (ticket === historyTicket) {
        historyLoading = false;
      }
    }
  }

  /** その会話の**最新のページ**を出す（古い順）。 */
  async function showSession(
    character: string,
    sessionId: number,
    ticket: number,
  ): Promise<void> {
    const page = await listSessionMessages(
      character,
      sessionId,
      undefined,
      lifetime.signal,
    );
    if (ticket !== historyTicket) {
      return;
    }
    history = {
      sessionId,
      messages: page.messages,
      hasOlder: page.has_older,
    };
  }

  /**
   * **もっと前を読む。** いま出している先頭の id をカーソルにして、その前の1ページを先頭へ足す。
   *
   * **重ねない・欠かさない**のはサーバの `before`（その id より**前**だけ）が持つ。
   * **ここで並べ直さない**（見張り番は `tests/chat-view-history.test.ts` の「遡り」）。
   */
  async function loadOlder(): Promise<void> {
    const current = history;
    const character = handle;
    const oldest = current?.messages[0];
    if (
      current === null ||
      character === null ||
      oldest === undefined ||
      olderLoading
    ) {
      return;
    }
    const ticket = historyTicket;
    olderLoading = true;
    historyError = null;
    try {
      const page = await listSessionMessages(
        character,
        current.sessionId,
        oldest.id,
        lifetime.signal,
      );
      if (ticket !== historyTicket) {
        return;
      }
      history = {
        sessionId: current.sessionId,
        messages: [...page.messages, ...current.messages],
        hasOlder: page.has_older,
      };
    } catch (error) {
      historyFailed(error, ticket);
    } finally {
      if (ticket === historyTicket) {
        olderLoading = false;
      }
    }
  }

  /** 読み込みの失敗。**自分で落とした分・古い読み込みの分は書かない。** */
  function historyFailed(error: unknown, ticket: number): void {
    if (lifetime.signal.aborted || ticket !== historyTicket) {
      return;
    }
    historyError = messageOf(error);
  }

  /** **過去の会話の一覧を開く・閉じる。** 開くたびに読み直す（送った会話は先頭へ動く）。 */
  async function togglePicker(): Promise<void> {
    if (picker !== null) {
      picker = null;
      return;
    }
    const character = handle;
    if (character === null) {
      return;
    }
    const ticket = historyTicket;
    pickerError = null;
    try {
      const list = await listSessions(character, lifetime.signal);
      if (ticket !== historyTicket) {
        return;
      }
      noPastSessions = list.sessions.length === 0;
      picker = list;
    } catch (error) {
      if (lifetime.signal.aborted || ticket !== historyTicket) {
        return;
      }
      pickerError = messageOf(error);
    }
  }

  /**
   * **一覧から選んだ。** 送り先をその会話に固定し（`aoi@<id>`）、その会話の過去を出す。
   *
   * **この画面で送った回（`turns`）は畳む** —— 別の会話の回が、選んだ会話の過去の下に並ぶと、
   * どの会話の続きなのかが読めなくなる。
   */
  function choose(session: SessionSummaryView): void {
    const character = handle;
    if (character === null || inFlight) {
      return;
    }
    pinned = continueSession(character, session.id);
    fresh = false;
    sessionWarning = null;
    turns = [];
    picker = null;
    pickerError = null;
    chosen = session.id;
    historyNotice = null;
    void openChosen(character, session.id);
  }

  /**
   * **返事が別の会話に着地した。** その会話の最新ページを読み直して差し替え、その旨を告げる。
   *
   * # 送った往復を2回出さない
   *
   * サーバは `[DONE]` を流す**前に**往復を保存する（`src/server/relay.rs`）。
   * だから `[DONE]` まで届いた回（`persisted`）は、**読み直したページの中に入っている** ——
   * この画面の回（`turns` の2件）を残すと、同じ発言と返事が2回並ぶ。**読み直せたら落とす。**
   * 届かなかった回は保存されていないので、「保存されていません」の注記ごと残す。
   *
   * **読み直しに失敗したら何も落とさない**（落とすと、送った回が画面から消えるだけになる）。
   */
  async function reloadLanded(
    character: string,
    sessionId: number,
    turnIndex: number,
    persisted: boolean,
  ): Promise<void> {
    const ticket = ++historyTicket;
    historyLoading = true;
    historyError = null;
    picker = null;
    try {
      await showSession(character, sessionId, ticket);
      if (ticket !== historyTicket) {
        return;
      }
      const model = continueSession(character, sessionId);
      historyNotice = persisted
        ? `返事は、表示していた会話とは別の会話（${model}）に保存されました。その会話を読み直しました。`
        : `送り先が、表示していた会話とは別の会話（${model}）になりました。その会話を読み直しました。`;
      if (
        persisted &&
        turns[turnIndex - 1]?.role === "user" &&
        turns[turnIndex]?.role === "assistant"
      ) {
        turns.splice(turnIndex - 1, 2);
      }
    } catch (error) {
      historyFailed(error, ticket);
    } finally {
      if (ticket === historyTicket) {
        historyLoading = false;
      }
    }
  }

  function abort(): void {
    controller?.abort();
  }

  async function send(): Promise<void> {
    const model = outgoingModel;
    const character = handle;
    const text = draft.trim();
    // **本文の面が載っていなければ送らない**（作業単位 D-16）。**釦を塞ぐだけでは足りない** ——
    // Ctrl+Enter（`onKeydown`）もここを通る。
    if (
      model === null ||
      character === null ||
      text === "" ||
      inFlight ||
      face.hold !== null
    ) {
      return;
    }

    draft = "";
    turns.push({ role: "user", text });
    const index = turns.push({ role: "assistant", raw: "", outcome: null }) - 1;

    controller = new AbortController();
    startedAt = Date.now();
    now = Date.now();
    /**
     * **表示している過去とは別の会話に着地した**ら、その id（`reloadLanded` の対象）。
     * 読み直すのは回が終わってから —— `[DONE]` の前に読むと、この回の往復がまだ保存されていない。
     */
    let landedElsewhere: number | null = null;

    const outcome = await sendMessage(
      model,
      text,
      {
        onDelta: (delta) => {
          const turn = turns[index];
          if (turn?.role === "assistant") {
            turn.raw += delta;
          }
        },
        onModel: (landed) => {
          if (isSessionOf(character, landed)) {
            pinned = landed;
            fresh = false;
            // **着地した＝会話が1つできた**（`noPastSessions` の doc）。
            noPastSessions = false;
            // **正しく着地した。** 前の回に拒んだ話はもう古い情報なので落とす
            // （`sessionWarning` の doc / 居座ると「まだ壊れている」と読まれる）。
            // **消してよいのはこの枝だけ。** `onModel` が呼ばれない回
            // （200 の前に断られた回など）は、着地できたかを何も言っていない。
            sessionWarning = null;
            // **表示している過去と突き合わせる**（`historyNotice` の doc）。
            // 過去を出していなければ（`@new`・会話ゼロ）、食い違う相手が無い。
            const landedId = sessionIdOf(character, landed);
            if (history !== null && landedId !== null) {
              if (landedId !== history.sessionId) {
                landedElsewhere = landedId;
              } else {
                historyNotice = null;
              }
            }
            return;
          }
          // **積み上げない。** いちばん新しい着地について1本だけ言う。
          // **⚠ 「いま固定しているか」は言わない**（doc / 状態の権威は `p.wire` の側）。
          sessionWarning =
            `応答が別のキャラクターのセッション（${landed}）を名乗ったため、` +
            "そこへは切り替えませんでした。";
        },
      },
      controller.signal,
    );

    const turn = turns[index];
    if (turn?.role === "assistant") {
      turn.outcome = outcome;
    }
    startedAt = null;
    controller = null;

    // **保存まで届いた回だけ、提案を取りに行く合図を進める**（`persistedTurns` の doc）。
    if (isPersisted(outcome)) {
      persistedTurns += 1;
    }

    // **完結しなかった回（利用者の中止を除く）の直後は、本文の面の状態をすぐ読み直す**
    // （D-16 のレビュー）。差し替えの窓で送ると、次の予約（止めていない間は `idlePollIntervalMs`）
    // まで「送れる」と言い続ける。**すぐ読めば、載せている最中なら止める側へ追いつく。**
    // 上流エラー（`upstream-error`）に限らず、200 の前に断られた回（上流が落ちていれば 502/503）も読む ——
    // どれも読むだけで、何も壊さない。
    if (outcome.kind !== "done" && outcome.kind !== "aborted") {
      void readUpstream();
    }

    if (landedElsewhere !== null) {
      void reloadLanded(character, landedElsewhere, index, isPersisted(outcome));
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      void send();
    }
  }

  void loadCharacters();
  void readUpstream();
</script>

<!--
  **応答の本文の整え方**（作業単位 D-12c-2 で snippet に切り出した）。

  **実行中の返事（`[DONE]` の後）と、読み直した過去の返事が、同じここを通る。**
  写しを2つ持つと、片方だけが整え方を変えたときに、**同じ返事が2通りに見える**。
  **snippet にしてあるのは、スタイル（`.dialogue` / `.narration`）がこの部品に閉じているため**
  （別の部品へ出すと、この部品の CSS が届かない）。
  見張り番は `tests/chat-view-history.test.ts` の「実行中の返事と、読み直した返事が同じに見える」。

  **上流が漏らした思考（`<think>…</think>`）は、ここで最初に剥がす**（ADR-0053 / 作業単位 B-61 / `thinking.ts`）。
  **剥がして空になった回は「応答が空でした。」に入る**（思考だけの回・閉じない思考の回）。
  見張り番は `tests/chat-view-thinking.test.ts`。
-->
{#snippet replyBody(raw: string)}
  {@const body = stripThinking(raw).text}
  {#if body.trim() === ""}
    <!--
      **見える文字が1つも来ないまま `[DONE]` に至った。** 言わないと
      **見出しだけが並んで無言に見える**——
      「まだ受信中なのか」「壊れたのか」が画面から分からない。

      # ⚠ **`ParseOutcome::Empty` へ「寄せた」。一致はしていない**（作業単位 B-8）

      **Rust 側も `ParseOutcome::Empty` として区別して数えている**
      （`src/parse/mod.rs`。崩れではないが、調査材料としては別物）。
      その受理集合は「括弧の対応が取れていて、入れ子が無く、**台詞の外も中も
      `domain::text::TRIMMABLE`（8点）と `LINE_BREAKS`（7点）の文字しか無い**」入力
      （見張り番は `parse::tests::a_blank_response_yields_nothing`）。

      **こちらはその集合を再現していない。`trim()` で寄せただけ。**
      **合わせる案（2つの表を FE へ写す）は却下された**——
      ADR-0031 §6 の「黙って古びる写し」が2つ増え、しかもその写しには見張り番が無い。
      **画面の目的は分類の再現ではなく、「見えない1文字が並んで無言に見える」状態を避けること。**

      ## 残った差（**BMP 全域を走査して実測**）

      **測り方**: `U+0000..U+FFFF` からサロゲート域を除いた全符号位置に
      `String.prototype.trim()` を当て、`domain::text` の2表の和集合と突き合わせた
      （node v26 / この機）。**結果は3つの数で言える** ——
      **Rust の和集合 11 点 / JS が落とす 25 点 / 食い違い 16 点。**

      * **画面のほうが狭い①**: **空の `「」`**。`trim()` は括弧を落とさないので、
        Rust が `Empty` と数える `"「」"` / `"「 」"` を、画面は本文として出す
        （着色の話は `dialogue.ts` の doc が持っている）。
      * **画面のほうが狭い②**: **U+0085（NEL）だけ。** Rust の `LINE_BREAKS` には入っているが、
        **JS の `trim()` は落とさない。** **Rust だけが落とす点は、走査した限りこの1点。**
      * **画面のほうが広い（向きが逆）**: **15 点** ——
        **U+1680 / U+2000..U+200A（11 点） / U+202F / U+205F / U+FEFF**。
        **`trim()` は落とすが、Rust はどちらの表にも持っていない**ので `NarrationOnly` として保存する。
        （**U+200B は両方が落とさない**ので差にならない。1 + 15 = 食い違い 16 点。）

      **見張り番は `tests/chat-view-session.test.ts` の
      「空の応答（`trim()` へ寄せた。一致はしていない）」**——
      **寄せた側（空白だけ）と、残差の3件を、両向きとも1本ずつ観測している。**
      **列挙を増やしたら、あそこに1本足すこと。**

      **⚠ ターンの一覧の「受信中」側の `turn.raw === ""` は別の主張なので、触っていない。**
      あちらは「**1文字でも届いたか**」だけを見ており、`ParseOutcome` とは関係が無い。
      **`trim()` を入れると、空白しか届いていない受信中の回が
      「応答を待っています…」に戻り、届いていることが画面から消える。**
    -->
    <p class="note">応答が空でした。</p>
  {:else}
    {@const parsed = split(body)}
    {#if parsed.kind === "split"}
      <p class="body">{#each parsed.segments as segment, at (at)}<span
            class={segment.kind}>{segment.text}</span>{/each}</p>
      {#if parsed.nested}
        <!-- **崩れではない**（ADR-0014）。起きたことだけ言う。 -->
        <p class="note">入れ子の「」がありました（崩れではありません）。</p>
      {/if}
    {:else}
      <p class="body narration">{parsed.text}</p>
      <!--
        **画面側で拾い直さない**（ADR-0007）。サーバも同じ規則で
        生出力を丸ごと地の文として保存している。
      -->
      <p class="note">
        台詞と地の文に分解できなかったので、全部を地の文として出しています（{reasonLabel(
          parsed.reason,
        )}）。
      </p>
    {/if}
  {/if}
{/snippet}

<section>
  <header>
    <h1>会話</h1>
    {#if characters === null}
      <span class="muted" role="status">キャラクターを読み込み中…</span>
    {:else}
      <label class="partner">
        相手:
        <select
          value={handle}
          disabled={inFlight}
          onchange={(event) => selectCharacter(event.currentTarget.value)}
        >
          {#each characters as character (character.id)}
            <option value={character.handle}>
              {character.display_name}（{character.handle}）
            </option>
          {/each}
        </select>
      </label>
      <button type="button" onclick={beginNewSession} disabled={inFlight}>
        新しい会話を始める
      </button>
    {/if}
  </header>

  <!--
    **見出しのすぐ下の帯: 送り先と、本文の面の状態**（作業単位 D-16 / D-17）。

    **2つを1本の帯に並べる**（見出しの帯には「相手」と「新しい会話を始める」だけを置き、混ませない）。
    どちらも**送る前に知っておくこと**なので、やり取りの一覧より前に置く。
  -->
  <div class="strip">
    {#if outgoingModel !== null}
      <!--
        **送り先を隠さない**（ADR-0004）。`model` フィールドがセッションの同定そのものなので、
        何を送っているかが見えていないと、記憶がどこへ入ったかを利用者が追えない。

        **意味（「新しい会話」など）を注記の見た目に埋めない**（作業単位 D-17 / Issue origin#38）——
        「同じキャラクターと新しい会話を始められるのか」が分からなかったのは、
        `aoi@new`（新しい会話）が小さな灰色の注記に埋もれていたから。**新しい会話のときは印を付ける**（`.fresh`）。
      -->
      <p class="wire" class:fresh>
        送り先: <code>{outgoingModel}</code>
        <strong class="meaning">（{outgoingMeaning}）</strong>
      </p>
    {/if}
    <!--
      **本文の面の状態を、常に出す**（作業単位 D-16 / Issue origin#37）。
      **利用者が居るのはこの画面**なので、`#/upstream` だけが言っていても届かない（ADR-0042 §決定3 の追記）。
      **文面は `#/upstream` のカードの「起こした上流の状態」と同じ**（`launchLabel`）。止める理由は送信の釦の隣に出す。
    -->
    <p class="face" role="status" data-body-face="1">{face.line}</p>
  </div>
  {#if face.subNote !== null}
    <p class="note" data-sub-face="1">{face.subNote}</p>
  {/if}
  {#if fresh && outgoingModel !== null}
    <!--
      **「新しい会話を始める」を押したら、何が起きたかを言う**（作業単位 D-17 / Issue origin#38）。
      押しても画面が空になるだけでは、何が起きたか分からない。

      **`fresh` の間だけ出す** —— 送って新しい会話に着地したら（`onModel` の固定）、
      会話を選んだら、キャラクターを変えたら、この文は偽になる（どれも `fresh` を降ろす）。
      **200 の前に断られた回は `fresh` のまま**（`pinned` の doc）なので、この文も真のまま残る。
    -->
    <!--
      **後半は、過去の会話があるときだけ**（`noPastSessions` の doc）。無いのに「前の会話は…」と言うと、
      押した人は「前の会話があったのか」と探しに行く。
    -->
    <p class="notice" role="status" data-new-session-notice="1">
      次に送った発言から、新しい会話になります。{#if !noPastSessions}前の会話は「過去の会話を選ぶ」から読めます。{/if}
    </p>
  {/if}

  {#if charactersError !== null}
    <p class="error" role="alert">
      キャラクターを取得できませんでした: {charactersError}
      <button type="button" onclick={() => void loadCharacters()}>再試行</button>
    </p>
  {:else if characters !== null && characters.length === 0 && archivedCount > 0}
    <!--
      **しまったキャラクターしか居ない**（作業単位 A-10）。「まだ1件もありません」と言わない ——
      キャラクターの画面の「しまったキャラクター」の一覧に居て、「戻す」で話せるようになる。
      **名指す操作はその画面に実在する**（`tests/chat-view-archived.test.ts` が描いて引く）。
    -->
    <p data-only-archived="1">
      しまっていないキャラクターはありません（しまったキャラクターは<a
        href={hrefFor({ route: "characters" })}
        >{labelFor({ route: "characters" })}の画面</a
      >の「戻す」で話せるようになります）。
    </p>
  {:else if characters !== null && characters.length === 0}
    <!--
      **実在する導線だけを案内する**（作業単位 F-5）。

      **⚠ ここは利用者が最初に見る画面である**（既定の経路は `#/chat`）。
      **空の DB から起動すると、必ずこの状態から始まる。**

      **かつては `POST /admin/characters` を案内していた** ——
      **1クリック先に「新規作成」ボタンがあるのに、`curl` の話をしていた。**
      **「実在する操作を案内しない」の6例目**（`docs/06-roadmap.md` の F 節）。

      **名指しする操作は別の画面にある**ので、**そこへ行くリンクを一緒に置く。**
      見張り番は `tests/chat-view-empty.test.ts` ——
      **リンクの行き先と、その行き先の画面にそのボタンが実在することを、
      同じ1本の中で見ている**（`CharacterList` を実際に描いて引く）。
    -->
    <p data-no-characters="1">
      キャラクターがまだ1件もありません。<a
        href={hrefFor({ route: "characters" })}
        >{labelFor({ route: "characters" })}の画面</a
      >の「新規作成」から作れます。
    </p>
  {/if}

  {#if outgoingModel !== null}
    <!--
      **⚠ 上の帯の送り先は「いま使っているモデル」に読める**（作業単位 D-9）。

      綴りが `model` という欄に乗るので無理もないが、**中身はセッションの識別子**で、
      `GET /v1/models` が返すのも**キャラクター**である（ADR-0004）。
      **着手前、画面に「モデル」に見えるものはこれ1つしか無く、
      それは1つもモデルではなかった。**

      **文言だけでは足りない** —— **本物のモデルの綴りが画面のどこにも無い**限り、
      利用者には見比べる先が無い。**だから上流の画面への導線を一緒に置く。**

      **いまは上の帯が、本文の面の名乗りを出す**（作業単位 D-16 / 司令塔が認めた / 2026-09-26）。
      ~~**ここに綴りを写さないこと。** 写すと `/admin/upstream` の値の写しが2つになり、
      片方だけが黙って古びる。~~ **帯は写しではなく、この画面が自分で読み直した値**である
      （`readUpstream`）。**ただし読み直しの間隔ぶん古くなりうる**ので、注記でそう言う。
      **外の上流では帯に名前が出ない**（`launch` が無い＝personacore は名乗りを受けていない）ので、
      **依頼する綴り・窓・外の上流の名乗りは、今までどおり上流の画面で見る。**
      見張り番は `tests/chat-view-outgoing.test.ts` ——
      **リンクの行き先と、その行き先の画面にモデルの綴りが実在することを、
      同じファイルの中で見ている**（`UpstreamPanel` を実際に描いて引く）。
    -->
    <p class="note" data-wire-note="1">
      これはこの会話を指す名前で、モデルの名前ではありません。本文を書いているモデルは、上の「本文のモデル」が
      personacore の起こした上流の名乗りを出します（次に読み直すまでは古いことがあります）。外の上流のときや、依頼する綴り・窓は<a
        href={hrefFor({ route: "upstream" })}
        >{labelFor({ route: "upstream" })}の画面</a
      >で見られます。
    </p>
  {/if}

  {#if sessionWarning !== null}
    <p class="error" role="alert">{sessionWarning}</p>
  {/if}

  {#if handle !== null && characters !== null}
    <!--
      **過去の会話の一覧**（作業単位 D-12c-2）。**選ぶと、その会話を読み、送るとその続きになる。**
      一覧は開くたびに読み直す（送った会話は先頭へ動く）。
    -->
    <div class="history-actions">
      <button
        type="button"
        onclick={() => void togglePicker()}
        disabled={inFlight}
        aria-expanded={picker !== null}
      >
        {picker === null ? "過去の会話を選ぶ" : "過去の会話の一覧を閉じる"}
      </button>
    </div>
  {/if}
  {#if pickerError !== null}
    <p class="error" role="alert">
      過去の会話の一覧を取得できませんでした: {pickerError}
    </p>
  {/if}
  {#if picker !== null && handle !== null}
    <SessionPicker
      {handle}
      list={picker}
      current={history?.sessionId ?? null}
      busy={inFlight}
      onChoose={choose}
      onBeginNew={beginNewSession}
    />
  {/if}

  {#if historyLoading}
    <p class="muted" role="status">過去のやり取りを読み込み中…</p>
  {/if}
  {#if historyError !== null}
    <p class="error" role="alert">
      過去のやり取りを読み込めませんでした: {historyError}
    </p>
  {/if}
  {#if history !== null && handle !== null}
    <!--
      **サーバに保存された過去のやり取り**（作業単位 D-12c-2）。

      **告げる囲み（下の `aria-live`）の外に置く。** 読み込んだ過去を全部読み上げると、
      画面を開くたびに会話を丸ごと朗読することになる。
    -->
    {#if historyNotice !== null}
      <p class="notice" role="status">{historyNotice}</p>
    {/if}
    {#if chosen !== null}
      <p class="note" data-chosen-note="1">
        一覧から選んだ会話を出しています。送ると、この会話（<code
          >{continueSession(handle, chosen)}</code
        >）の続きになります。
      </p>
    {/if}
    {#if history.hasOlder}
      <button
        type="button"
        class="older"
        onclick={() => void loadOlder()}
        disabled={olderLoading}
      >
        もっと前を読む
      </button>
      {#if olderLoading}
        <span class="muted" role="status">前のやり取りを読み込み中…</span>
      {/if}
    {/if}
    <ol class="history">
      {#each history.messages as message (message.id)}
        <li class={message.role} data-message-id={message.id}>
          {#if message.role === "user"}
            <span class="who">あなた</span>
            <p class="body">{message.raw_content}</p>
          {:else}
            <span class="who">相手</span>
            {@render replyBody(message.raw_content)}
          {/if}
        </li>
      {/each}
    </ol>
  {/if}

  <!--
    **流れてくる本文を、読み上げる側にも届ける**（作業単位 D-11b）。

    **⚠ 受信中に生える `<p>` そのものには付けない。**
    **告げる領域は、中身が変わる「前から」在る必要がある** ——
    領域と中身が同時に挿入されると、**読み上げない閲覧器がある。**
    ここは画面が出た時点から在り、**本文はこの中で育つ**ので、
    **完結したときに `<p>` が別の要素へ差し替わっても、この囲みは動かない。**

    **⚠ 代償**: **利用者自身の発言も同じ囲みの中に在る**ので、一緒に告げられる。
    分けるには囲みをもう1つ要る＝マークアップを書く回（`D-12`）。

    **⚠ `polite` にしてある。** 本文は1文字ずつ増えるので、
    割り込む側（`assertive`）にすると、届くたびに読み上げが中断される。
    **`off` にすると、囲みは在るまま何も告げなくなる。**

    見張り番は `tests/live-region.test.ts` の `theLiveRegion()` ——
    **届く前からこの要素が在ることと、届いた本文がこの中に入ることと、
    この要素が `polite` を名乗っていることを、同じ1本で見ている**
    （`data-*` の印ではなく、属性そのものを引く。**`off` は「告げる要素」に数えない**）。
    **⚠ 値を見ない形では、`off` も `assertive` も 846 全緑を通り抜けた**（D-11b のレビューが実測）。
  -->
  <ol class="turns" aria-live="polite">
    {#each turns as turn, index (index)}
      <li class={turn.role}>
        {#if turn.role === "user"}
          <span class="who">あなた</span>
          <p class="body">{turn.text}</p>
        {:else}
          <span class="who">相手</span>
          {#if turn.outcome !== null && turn.outcome.kind === "done"}
            {@render replyBody(turn.raw)}
          {:else}
            <!--
              **受信中も思考は剥がす**（ADR-0053 §決定4 / 作業単位 B-61）。閉じないうちは
              開きタグから末尾までが思考とみなされるので、**思考が流れている間は本文が空のまま**で、
              閉じタグが届いた時点で後ろの本文が出始める。
              **⚠ 下の「応答を待っています…」の判定（`turn.raw === ""`）は生の本文のまま。**
              思考しか届いていなくても「受信中…」になる —— 届いていることを画面から消さない。
            -->
            <p class="body">{stripThinking(turn.raw).text}</p>
          {/if}

          {#if turn.outcome === null}
            <!--
              **毎秒書き換わるので、告げる側から外す**（作業単位 D-11b の是正）。

              この段落は上の `aria-live="polite"` の囲みの中に在り、`elapsed` が
              **1秒ごとに書き換わる。** 既定の `aria-relevant="additions text"` の下では
              **生成中に毎秒、読み上げの列へ積まれる**（30 秒なら約 30 回）——
              **本文の増分に割り込む。**

              **⚠ `off` は「名乗っているが告げない」。** 変わったことはこの囲みからは告げられなくなるが、
              **欄そのものは画面に在る**ので、目で見る側からは何も減らない。

              **⚠ 守れていないのは「実際に読み上げられないこと」。**jsdom は読み上げないので、
              見張り番が言えるのは**属性の在処まで**である（`tests/live-region.test.ts` の
              「経過秒を持つ段落は、告げる要素の中で `aria-live="off"` を名乗る」——
              **経過秒の文言を実際に持つ段落を引いて、その要素の属性を見る**）。
            -->
            <p class="note" aria-live="off">
              {#if turn.raw === ""}
                応答を待っています… {elapsed} 秒
              {:else}
                受信中… {elapsed} 秒
              {/if}
              {#if elapsed >= 5 && turn.raw === ""}
                <span class="muted">
                  （モデルの読み込みや、裏で走っている抽出の順番待ちで、
                  最初の一文字まで数十秒かかることがあります）
                </span>
              {/if}
            </p>
          {:else if !isPersisted(turn.outcome)}
            {@const notice = outcomeNotice(turn.outcome)}
            {@const keyFailure = keyFailureOf(outcomeCode(turn.outcome))}
            {#if notice !== null}
              <p class="error" role="alert">{notice}</p>
            {/if}
            <!--
              **鍵の失敗には、直す場所を足す**（作業単位 A-13b-3 / `upstreamKey.ts` の `keyFailureGuidance`）。
              断られた（`upstream_unauthorized`）と送らなかった（`upstream_key_not_approved`）は直す場所が違うので、
              別の文で言う。**ほかの失敗には足さない。** 導線の綴りは `router.ts` から（写さない）。
            -->
            {#if keyFailure !== null}
              {@const guidance = keyFailureGuidance(keyFailure)}
              <p class="note key-guidance">{guidance.before}<a href={hrefFor({ route: "upstream" })}
                  >{labelFor({ route: "upstream" })}の画面</a
                >{guidance.after}</p>
            {/if}
            <!--
              **`[DONE]` を受けていない回はサーバに1行も書かれていない**
              （利用者の発言も含めて / `src/server/relay.rs` / ADR-0019 第4節）。
              画面には残っているので、**言わないと画面と履歴が食い違ったままになる。**
            -->
            <p class="note">
              この回はサーバに保存されていません（あなたの発言も含めて履歴に残りません）。
            </p>
          {/if}
        {/if}
      </li>
    {/each}
  </ol>

  <form
    onsubmit={(event) => {
      event.preventDefault();
      void send();
    }}
  >
    <textarea
      bind:value={draft}
      onkeydown={onKeydown}
      rows="3"
      placeholder="発言を入力（Ctrl+Enter で送信）"
      disabled={outgoingModel === null}
    ></textarea>
    {#if face.hold !== null}
      <!--
        **送信を止めている理由を1行**（作業単位 D-16）。**載っていない（失敗・止めた）ときは、
        理由とログが見られる画面への導線を置く**（綴りは `router.ts` から。写さない）。
      -->
      {#if face.linkToUpstream}
        <p class="error" role="alert" data-send-hold="1">
          {face.hold}<a href={hrefFor({ route: "upstream" })}
            >{labelFor({ route: "upstream" })}の画面</a
          >で確かめられます。
        </p>
      {:else}
        <p class="note" role="status" data-send-hold="1">{face.hold}</p>
      {/if}
    {/if}
    {#if face.startProfile !== null}
      {@const name = face.startProfile}
      <!--
        **起こしていない本文の面を起こす**（作業単位 F-10b / ADR-0055 §決定3）。
        **`type="button"`** —— 送信の欄の中にあるので、既定（submit）のままだとフォームの送信になる。
        押した後は応答の状態（載せている）に乗り、この塊ごと消える。断られた理由はこの塊の中で言う。
      -->
      <div class="start-upstream">
        <button type="button" disabled={starting !== null} onclick={() => void startUpstream(name)}
          >{startButtonLabel(name)}</button
        >
        <span class="note">{START_CAUTION}</span>
        {#if starting !== null}
          <p class="muted" role="status">{startingLabel(starting)}</p>
        {/if}
        {#if startError !== null}
          <p class="error" role="alert">{startError}</p>
        {/if}
      </div>
    {/if}
    <div class="actions">
      <button
        type="submit"
        disabled={inFlight ||
          outgoingModel === null ||
          draft.trim() === "" ||
          face.hold !== null}
      >
        送信
      </button>
      <button type="button" onclick={abort} disabled={!inFlight}>中止</button>
    </div>
  </form>

  {#if handle !== null && characters !== null}
    <!--
      **約束・決まりの提案の問いかけ**（作業単位 A-12c-2 / 中身と時機は `ProposalPrompt.svelte` の doc）。

      **送信の欄の後ろに置く**（入力と送信を塞がない。測定が引く「最初の `<form>` / `<select>`」も変えない）。
      **相手を変えたら作り直す**（`{#key}`）—— 前の相手の提案・直しかけの欄・読み直しの予約を持ち越さない。
    -->
    {#key handle}
      <ProposalPrompt
        {handle}
        turnsEnded={persistedTurns}
        recheckIntervalMs={proposalRecheckMs}
        rechecks={proposalRechecks}
      />
    {/key}
  {/if}
</section>

<style>
  /* **枠・字・色は `src/app.css` にある**（作業単位 D-10）。 */

  /* **相手を選ぶ欄と「新しい会話」が、狭い窓で折り返せるように。** */
  header {
    flex-wrap: wrap;
  }

  /*
   * **「相手:」と選択は、いつも同じ行**（作業単位 B-67 / Issue origin#96）。
   *
   * 選択の幅は一番長い選択肢（`{display_name}（{handle}）`）で決まる。
   * ラベルが文字の行のままだと、幅が足りないときに**ラベルの中で**折り返し、
   * 「相手:」が上に残って選択だけが下の段に出た。
   * **横に並べて折り返さず、帯の残り幅まで縮める**（`min-width: 0`）——
   * 長い選択肢は選択の中で切れて見える。**狭い幅では帯ごと段を変える**（上の `flex-wrap`）。
   * 帯の `align-items: baseline`（土台）は、このラベルの最初の子（「相手:」）の基線で揃う。
   */
  .partner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    white-space: nowrap;
    min-width: 0;
  }

  /* **土台の `label select { width: 100% }` を外す** —— 残すと選択がラベルの幅を取り、文言を押し出す。 */
  .partner select {
    width: auto;
    min-width: 0;
  }

  /* **過去のやり取り（`.history`）も同じ見え方**（作業単位 D-12c-2）。 */
  .turns,
  .history {
    list-style: none;
    margin: 1rem 0;
    padding: 0;
  }

  /* **回の切れ目は横罫1本だけ**（縦罫は引かない / 土台の決め）。 */
  .turns > li,
  .history > li {
    border-top: 1px solid var(--color-rule);
    padding: 0.6rem 0;
  }

  .who {
    font-size: 0.8rem;
    color: var(--color-muted);
  }

  .body {
    margin: 0.2rem 0;
    /* **生出力の改行をそのまま見せる。** 詰めると段落構造が消える。 */
    white-space: pre-wrap;
  }

  /*
    **台詞と地の文の見分け**（ADR-0007）。

    **⚠ 分けているのは色ではなく字**（作業単位 D-10 / 「計器と声」）。
    **色は意味を持つときだけ使う**と決めたので、
    **台詞に accent を当てると「測っている」か「壊れている」に読める。**
    人格の側は明朝、それ以外はゴシックで分ける。
  */
  .dialogue {
    font-family: var(--font-mincho);
    font-weight: 600;
  }

  .narration {
    color: var(--color-ink-2);
  }

  .note {
    margin: 0.2rem 0;
  }

  /*
    **見出しの下の帯**（送り先と、本文の面の状態 / 作業単位 D-16・D-17）。
    **狭い窓では折り返す**（見出しの帯と同じ）。
  */
  .strip {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.25rem 1.5rem;
    margin: 0.5rem 0;
  }

  .strip > p {
    margin: 0;
  }

  /* **送り先**（`model` フィールド / ADR-0004）。**注記より大きく**（作業単位 D-17）。 */
  .wire {
    font-size: 1rem;
  }

  /*
    **新しい会話のときだけ、意味を枠で囲む**（作業単位 D-17）。
    **色は使わない**（accent は「測っている」か「壊れている」に読める / `src/app.css`）。
  */
  .wire.fresh .meaning {
    border: 1px solid var(--color-rule-2);
    padding: 0 0.4rem;
  }

  .face {
    font-size: 0.9rem;
    /* **長い gguf の名前で、狭い窓（375px）の横にはみ出さない**（D-16 のレビュー）。 */
    overflow-wrap: anywhere;
  }

  .actions {
    margin-top: 0.4rem;
  }

  /*
    **起こすボタンと、押す前に確かめること**（作業単位 F-10b）。**狭い窓では折り返す**（帯と同じ）。
    プロファイルの名前が長くても横にはみ出さない。
  */
  .start-upstream {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.25rem 0.75rem;
    margin: 0.4rem 0;
    overflow-wrap: anywhere;
  }

  .start-upstream > p {
    flex-basis: 100%;
    margin: 0;
  }
</style>
