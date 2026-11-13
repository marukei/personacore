//! 応答の台詞 / 地の文への分解。
//!
//! **I/O を持たない純粋関数のみ**を置く（CLAUDE.md 3節）。`async` も HTTP も DB も無い。
//! 入力は LLM の生出力（`&str`）、出力は `dialogue` / `narration` と**分解の結果**。
//!
//! # 形式（ADR-0007）
//!
//! **`「` と `」` で囲まれた部分が台詞、それ以外が地の文。**
//!
//! ```text
//! 　ふと顔を上げて、彼女は目を細めた。「おかえりなさい」
//! 　声には少しだけ安堵が混じっている。
//! ```
//!
//! → `dialogue` = `おかえりなさい` / `narration` = 残り
//!
//! # 「崩れた」の判定基準
//!
//! **崩れたら生出力を丸ごと地の文にする。部分的に拾わない**（ADR-0007）。
//! したがって「どこからが崩れか」がこのモジュールの設計の中心になる。
//! 崩れと判定するのは**次の2つだけ**で、`FallbackReason` がそのまま対応する。
//!
//! | 判定 | 例 | 理由 |
//! |---|---|---|
//! | [`FallbackReason::Unclosed`] | `「おかえり` | 台詞の終端が決まらない。どこまでを台詞とするかを推測すると「拾えた分だけ拾う」になる |
//! | [`FallbackReason::StrayClose`] | `」あ「い」` | 対応する `「` の無い `」`。開き括弧の脱落であり、区間の左端が決まらない |
//!
//! **逆に、次は崩れではない**（正常系として分解する）。
//!
//! * 台詞が複数ある（`「あ」…「い」`）— 慣習どおりの正しい出力
//! * **入れ子（`「外「内」外」`）— [`ParseOutcome::StructuredNested`]**（下記）
//! * 台詞が無い（地の文だけ）— [`ParseOutcome::NarrationOnly`]
//! * 地の文が無い（応答全体が台詞1つ）— `narration` が `None` になるだけ
//! * 台詞が空（`「」`）— 対応は取れているので崩れではない。中身が無いので台詞としては数えず、
//!   `「」` の2文字は地の文にも残さない（「`「」` の外側が地の文」の定義どおり）
//!
//! **判定は開き括弧と閉じ括弧の対応だけを見る。** 台詞の長さや中身では判定しない。
//! 「長すぎる台詞は崩れ」のような閾値を入れると、正当な長台詞が理由不明で
//! 丸ごと地の文になり、しかも `ParseOutcome` を見ても区別がつかなくなる。
//!
//! # 入れ子の `「」`（ADR-0014）
//!
//! **1段目の `「` から、対応する `」` までを台詞とする。**
//! 内側の `「」` は**台詞本文にそのまま残す**（除去も置換もしない）。
//!
//! ```text
//! 「この前ね、『誰かが「生ケーキ」って言い出して大騒ぎ』だったんだよ」
//! ```
//!
//! → `dialogue` = `この前ね、『誰かが「生ケーキ」って言い出して大騒ぎ』だったんだよ`
//!
//! **実測で、モデルは日本語として正しく書いていた**（内側は `『』` を使い、1箇所だけ滑る）。
//! 崩れとして丸ごと地の文へ落とすと、**1段目すら捨てることになる。**
//!
//! ただし**「許可」ではなく「捕捉して記録」にする。** 入れ子が起きた回は
//! [`ParseOutcome::StructuredNested`]（[`ParseOutcome::fell_back`] は `false`）になり、
//! 独自の集計キーを持つので**発生率を追える。**
//!
//! **緩めるのは入れ子だけで、不均衡な入力の壊れ方は変わらない。**
//! `「あ「い」` は深さが 1 のまま終わるので [`FallbackReason::Unclosed`]、
//! `」あ「い」` は [`FallbackReason::StrayClose`]。
//!
//! # `「` `」` 以外の引用符
//!
//! **台詞として扱わない。** ADR-0007 が台詞と決めたのは `「」` だけであり、
//! `『』` / `""` / `“”` / `〈〉` などは地の文の中の引用（作品名・強調・引用）として
//! そのまま残る。台詞の中に現れた場合も、ただの文字として台詞本文に残る。
//!
//! 特に **`『』` を台詞に含めない**（ADR-0007。ADR-0014 でも変えない）。
//! 慣習では `「彼は『行く』と言った」` のように `「」` の中で `『』` を使うので、
//! 加えると**慣習どおりの正しい出力が軒並み入れ子として記録され**、
//! 「モデルが `「」` を重ねた回」を数えるという [`ParseOutcome::StructuredNested`] の
//! 目的が成立しなくなる。地の文の中の `『作品名』` を台詞として拾ってしまう問題もそのまま残る。
//!
//! # 複数の台詞をどう1列に入れるか
//!
//! `message.dialogue` は TEXT 1列なので、複数の台詞は**改行で連結する**。
//! そのために**台詞1つの中の改行は半角スペースへ畳む**ので、
//! **「`dialogue` の1行 = 台詞1つ」が不変条件**になる（`dialogue_lines` で取り出せる）。
//!
//! 区切り文字を決める以上どれかは犠牲になるが、
//!
//! * `「` `」` を付け直して連結する案は、台詞1つのときに ADR-0007 の例
//!   （`dialogue` = `おかえりなさい`）と食い違う。
//! * 空白区切りは台詞の境界が本文中の空白と区別できない。
//! * 改行区切りは、**畳んだ改行が生出力に残っている**（`message.raw_content`、
//!   CLAUDE.md 絶対制約）ので復元可能であり、境界が機械的に取り出せる。
//!
//! 台詞の中の改行が意味を持つことは稀で、失われても `raw_content` から復元できる。
//! 逆に「何個の台詞だったか」は連結してしまうと復元が難しい。**取り出せる方を残す。**
//!
//! # 整形の範囲
//!
//! 分解にあたって行うのは次だけ。**内容は書き換えない。**
//!
//! * 行区切り（CRLF / NEL / U+2028 / U+2029 等）を `\n` に揃える。
//!   `char::is_control` では U+2028 / U+2029 を捕まえられない（`prompt::safe` と同じ事情）。
//! * 台詞: 行区切りを半角スペースへ畳み、前後の空白（[`TRIMMABLE`]）を落とす。
//! * 地の文: **各行の行末**の空白を落とし、先頭と末尾の空行を落とす。
//!   **行頭の空白は保持する。** 字下げ（`　`）は地の文の様式であり、
//!   落とすと1行目だけ字下げが消えて不揃いになる。
//! * 地の文で、**台詞を抜いた結果として空になった行は落とす**（跡地を残さない）。
//!   **元から空だった行は段落の区切りとして残す**（ADR-0007）。
//!   台詞を単独行に置くのは日本語小説で最も一般的な体裁なので、
//!   跡地を残すと**台詞1つにつき空行が1つ湧く。** 逆に全部畳むと段落構造が消える。
//! * どちらも、残りが空なら `None`。
//!
//! # Markdown の強調の印（作業単位 B-46）
//!
//! **割る前に、行ごとに強調の印を落とす**（[`strip_body_emphasis`]）。
//! 実機の観測（`C-18` / 公式 Qwen3.6-35B-A3B）で、**モデルは本文に `**朝**` と書く。**
//! 抽出の側は前から落としていた（`extraction::strip_emphasis`）が、**本文の側には無かった。**
//!
//! **⚠ ここが落とす先は、保存する2列だけである**（`message.dialogue` / `message.narration`）。
//! **画面はこの関数の出力を読んでいない** —— 非ストリーミングの応答は上流の生本文、
//! SSE は中継、履歴に出るのは `raw_content`（応答は漏れた思考を剥がし、暴走を畳んでから / ADR-0053・ADR-0062）。
//! **この2列の値を使う本番コードは、いま1つも無い**——`store` は列を `Message` に載せるが、
//! **`prompt` が履歴へ戻すのは `raw_content`**（`prompt::mod` の「本文は `raw_content` を使う」。
//! 応答からは漏れた思考を剥がし、暴走を畳む）。
//! つまりここで直しているのは**保存する2列の正しさ**で、**いまその読み手は居ない**
//! ——調査（`raw_content` と並べて読む）と、将来の読み手のために正しくしておく。
//!
//! **画面からアスタリスクを落としているのは `web/src/lib/emphasis.ts` のほう**で、
//! **この関数ではない。**「サーバが落とすから画面にも出ない」と読まないこと。
//!
//! **規則は `extraction::strip_emphasis` と同じ1つを共有する**（写しを作らない）。
//! 対になった印と、行の端で宙に浮いた印だけを落とし、
//! **対になっていない印（`**閉じない`）は触らない**——
//! 落とすと、崩れた出力の本文を**こちらの推測で書き換える**ことになる。
//!
//! **3つだけ、ここで決めていることがある。**
//!
//! * **行ごとに当てる。** 対は行をまたがない（`**朝` と `**夜` は別々の行の印）。
//!   **行区切りは1文字も変えずに戻す。**
//! * **崩れた回は落とさない**（[`fallback`] は生出力そのまま / ADR-0007「整形もしない」）。
//! * **落とした対の数**を [`ParsedResponse::emphasis_pairs_stripped`] に載せる。
//!   呼び出し側（`server::turn`）がログへ出す。**画面には出さない。**
//!
//! **`message.raw_content` は生出力のまま**（CLAUDE.md 絶対制約）。落ちるのは
//! `dialogue` / `narration` の側だけで、**非ストリーミングの応答も SSE も、
//! 上流から受け取ったままクライアントへ返る**（`server::chat` / `server::relay`）。
//! **画面の側は `web/src/lib/emphasis.ts` が同じ規則を手で写して持っている**
//! ——両方を同じ1つの表で固定している（`tests/fixtures/emphasis_corpus.tsv`）。
//!
//! # 観測可能性（パース成功率）
//!
//! **`parse` の本番コードはログを出さない**
//! （**確かめ方: `#[cfg(test)]` の外に `tracing::` が1つも無いこと**）。
//! 成功率は1回の呼び出しでは決まらないので、
//! 集計は `server` の責務（CLAUDE.md 絶対制約「パース成功率をログに残す」）。
//! ここは結果に [`ParseOutcome`] を載せて**呼び出し側が集計できる形**にすることに徹する。
//! [`ParseOutcome::fell_back`] が成功率の分母／分子、
//! [`ParseOutcome::label`] が内訳のキーになる。
//!
//! **⚠ `grep -rn "tracing::" src/parse` は、作業単位 B-39 から 0 件ではない。**
//! テスト側が**植えた欄**を1つ鳴らす（`a_key_the_check_rejects_is_misread_by_the_field_readers`）
//! ——本番の入口からはその状態を作れないため。**確かめ方を上のように書き換えたのはそのため。**
//!
//! # panic しないこと
//!
//! **絶対制約。** 走査は `chars()` だけで行い、**バイト添字によるスライスを作らない。**
//! `「`（U+300C）は3バイトなので、`raw[i..j]` を書いた瞬間にバイト境界の
//! 取り違えが panic になりうる。`unwrap` / `expect` / 添字アクセスも置かない。

/// 軸名の漏れの検出（ADR-0018）。**分解の後段**で、同じく純粋関数。
pub mod leak;

/// **記憶抽出の出力の分解**（ADR-0021）。本文の分解とは別の形式・別の規約。
///
/// **`parse_response` と混同しないこと。** あちらは本文（台詞／地の文）で、
/// 崩れたら生出力を丸ごと地の文にする。こちらは抽出サブLLM の出力で、
/// **崩れた行は捨てて数える**（フォールバック先が無い）。
pub mod extraction;

/// **性格ステータスの変容の提案の分解**（ADR-0024）。
///
/// `extraction` と同じ作法（捨てて数える）だが、**受ける集合はこちらの方が狭い**。
/// 理由はモジュールの doc（捨てても失うものが無い側だから）。
pub mod drift;

/// **軸の候補の応答の分解**（ADR-0046 §決定2 / 作業単位 A-9b）。
///
/// 名前を1つ取り出すか、取り出せなかった理由を返すだけ。**既存の名前との照合はしない**
/// （`domain::axis_name_clash`）。
pub mod axis_candidate;

/// **約束・決まりの提案の出力の読みと票と後段**（ADR-0051 §決定2 —— δ3 ＋ dedup2 / 作業単位 A-12c-1）。
///
/// `extraction` と同じく**崩れた行は捨てる**（フォールバック先が無い。0 件で終わる）。
/// 受理集合は測定の道具（`p1/tools/rp3.py`・`rp1.py`・`pc.py`・`rp4.py`）のまま。
pub mod agreement_proposal;

use crate::domain::{LINE_BREAKS, TRIMMABLE};
use extraction::strip_emphasis_counted;

/// 台詞の開き括弧（U+300C）。**3バイト。**
const OPEN: char = '「';

/// 台詞の閉じ括弧（U+300D）。**3バイト。**
const CLOSE: char = '」';

/// 復帰（U+000D）。CRLF の判定にだけ使う。
const CR: char = '\u{000D}';

/// 改行（U+000A）。CRLF の判定にだけ使う。
const LF: char = '\u{000A}';

/// 行区切りとして扱う文字か。**判定は `domain::LINE_BREAKS` に一本化する**（写しを作らない）。
///
/// `prompt::safe` と同じ集合でなければならない。片方だけ広げると、
/// プロンプトへ戻す経路と保存する経路で行の数え方がずれ、
/// 「`dialogue` の1行 = 台詞1つ」の不変条件が
/// **Rust から見て1行・表示上は2行**という形で静かに破れる。
fn is_line_separator(character: char) -> bool {
    LINE_BREAKS.contains(&character)
}

/// 分解の結果。
///
/// **`dialogue` / `narration` は両方 `None` になりうる**（ADR-0007）。
/// そのまま `domain::NewMessage` の同名フィールドに入る形にしてある。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedResponse {
    /// 台詞。複数あれば**改行で連結**され、1行が1つの台詞に対応する。
    pub dialogue: Option<String>,
    /// 地の文。フォールバック時は**生出力そのもの**が入る。
    pub narration: Option<String>,
    /// 分解がどうなったか。**呼び出し側が成功率を集計するための値。**
    pub outcome: ParseOutcome,
    /// **本文から落とした Markdown の強調の対の数**（作業単位 B-46 / 下記「強調の印」）。
    ///
    /// **崩れた回は必ず 0。** 生出力を丸ごと地の文にする枝では1つも落とさない。
    ///
    /// # 抽出側の欄とは**意味が違うので、名前も違える**
    ///
    /// `extraction::ExtractedMemories::emphasis_stripped` は**落とした行数**で、
    /// こちらは**対の数**。**同じ数にならない**（1行に3対あれば、あちらは 1、ここは 3）。
    ///
    /// **⚠ 名前を揃えてはいけない**（レビュー1周目の指摘 / CLAUDE.md の `16l`）。
    /// 両方ともログの欄になり、**どちらの行も `session_id` を持つ**ので、
    /// 同じ綴りにすると**単位の違う2つの数が集計で1つの列に混ざる。**
    /// 読み手は取り違えても気づけない——**行数と対の数は、どちらも小さい整数**である。
    /// だから本文側は `emphasis_pairs_stripped`、抽出側は `emphasis_stripped` とする。
    ///
    /// 呼び出し側（`server::turn`）はログに出す。**画面には出さない**——
    /// 落とした跡を本文の脇に書くと、モデルの出力の質の話が読み手の画面に漏れる。
    pub emphasis_pairs_stripped: usize,
}

impl ParsedResponse {
    /// 台詞を1つずつ取り出す。**空なら0件。**
    ///
    /// 「`dialogue` の1行 = 台詞1つ」という不変条件の読み出し口。
    /// 呼び出し側に `split('\n')` を書かせない（区切り文字の決定権をここに閉じる）。
    pub fn dialogue_lines(&self) -> impl Iterator<Item = &str> {
        self.dialogue.as_deref().unwrap_or_default().lines()
    }
}

/// 分解がどうなったか。
///
/// **フォールバックしたかどうか**（[`Self::fell_back`]）が成功率そのもので、
/// 内訳は [`Self::label`] を集計キーに使う。6つの値はすべて異なるラベルを持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseOutcome {
    /// 台詞と地の文に分解できた（台詞が1つ以上ある）。
    Structured,
    /// 分解できたが、**台詞の中にさらに `「」` があった**（`「外「内」外」`）。
    ///
    /// **崩れではない**（ADR-0014）。[`Self::Structured`] の細分で、
    /// 1段目の区間を台詞として取り、内側の `「」` は台詞本文にそのまま残る。
    ///
    /// **別の値にしてあるのは発生率を追うため。** `Structured` に混ぜると
    /// 「入れ子が増えているか」を後から数えられない。
    StructuredNested,
    /// 括弧の対応は取れたが、台詞が1つも無かった。**崩れではない。**
    ///
    /// 地の文だけの応答は正当なので、フォールバックと混ぜて数えない。
    /// 混ぜると「モデルが地の文しか書かない」と「形式が崩れている」が区別できず、
    /// プロンプト調整の判断材料にならなくなる。
    NarrationOnly,
    /// 台詞も地の文も残らなかった（生出力が空、または空の `「」` だけ）。
    ///
    /// **空白だけの入力（`"   \u{3000}  "`）もここに入る。** 整形で何も残らないため。
    ///
    /// **崩れではない。** ただし LLM が空を返したこと自体は調査材料なので、
    /// [`Self::NarrationOnly`] とは区別して数える。
    Empty,
    /// 形式が崩れたので、生出力を丸ごと地の文にした。
    Fallback(FallbackReason),
}

impl ParseOutcome {
    /// フォールバックしたか。**パース成功率の分子（の否定）。**
    pub const fn fell_back(self) -> bool {
        matches!(self, Self::Fallback(_))
    }

    /// 台詞の中にさらに `「」` があったか。**崩れではない**（ADR-0014）。
    ///
    /// 集計は [`Self::label`] で足りるが、
    /// 「入れ子だったか」だけを見たい側に文字列比較を書かせない。
    pub const fn nested(self) -> bool {
        matches!(self, Self::StructuredNested)
    }

    /// 崩れの理由。フォールバックしていなければ `None`。
    pub const fn fallback_reason(self) -> Option<FallbackReason> {
        match self {
            Self::Fallback(reason) => Some(reason),
            _ => None,
        }
    }

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    ///
    /// `tracing` のフィールドやメトリクスのラベルにそのまま使える。
    /// 生出力を含めないのは、ログに LLM 出力を丸ごと載せないため。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::StructuredNested => "structured_nested",
            Self::NarrationOnly => "narration_only",
            Self::Empty => "empty",
            Self::Fallback(reason) => reason.label(),
        }
    }
}

/// 崩れの理由。**この2つ以外を崩れと判定しない**（モジュールの doc を参照）。
///
/// **入れ子（`「外「内」外」`）はここに無い。** ADR-0014 で崩れから外し、
/// [`ParseOutcome::StructuredNested`] として正常系に置いた。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// `「` が閉じないまま入力が終わった。**入れ子の分も数える**（`「あ「い」`）。
    Unclosed,
    /// 対応する `「` の無い `」` が現れた。
    StrayClose,
}

impl FallbackReason {
    /// 集計キー。**値ごとに異なる定数。**
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unclosed => "fallback_unclosed",
            Self::StrayClose => "fallback_stray_close",
        }
    }
}

/// 生出力を台詞と地の文に分解する。**純粋関数**（I/O も乱数も時刻も見ない）。
///
/// **どんな入力でも panic せず、`Result` も返さない。** 形式が崩れたら
/// 生出力を丸ごと地の文にしたうえで、[`ParseOutcome::Fallback`] を返す
/// （ADR-0007）。呼び出し側にエラー処理を強いる形にすると、
/// 「失敗したときに何を保存するか」の判断が呼び出し側ごとに分岐する。
pub fn parse_response(raw: &str) -> ParsedResponse {
    // **割る前に強調の印を落とす**（作業単位 B-46 / モジュールの doc「強調の印」）。
    // **崩れた枝では使わない**——下の `fallback` には `raw` をそのまま渡している。
    let body = strip_body_emphasis(raw);
    let mut narration = NarrationLines::new();
    let mut dialogues: Vec<String> = Vec::new();
    let mut current = String::new();
    // 開いている `「` の数。**1段目だけが台詞の境界**で、2段目以降は本文の文字（ADR-0014）。
    // 真偽値では「内側の `」` で閉じてしまう」形を表現できない。
    let mut depth: usize = 0;
    // 台詞の中にさらに `「` が現れたか。**崩れではないが、発生率を追うために記録する。**
    let mut nested = false;
    // 直前の文字が「地の文の改行を1つ作った CR」か。
    // **CRLF を改行1つに畳むのは、生出力で隣接しているときだけ。**
    // 台詞を抜いた後の文字列に対して畳むと、`あ\r「x」\nい` のように
    // **元は離れていた CR と LF が隣接し、行が1つ消える。**
    let mut after_carriage_return = false;

    // **`chars()` だけで走査する。** バイト添字のスライスを作らないので、
    // `「`（3バイト）に対して境界を取り違える余地が構造的に無い。
    for character in body.text.chars() {
        // 何であれ1文字進んだ時点で CR との隣接は切れる。
        let follows_carriage_return = std::mem::take(&mut after_carriage_return);

        match character {
            OPEN => {
                if depth == 0 {
                    // この行に台詞があったことを覚える（跡地だけの行を落とすため）。
                    narration.mark_dialogue();
                } else {
                    // 入れ子。**内側の `「` は台詞本文にそのまま残す**（ADR-0014）。
                    nested = true;
                    current.push(character);
                }
                // `usize` なので、入力に載る文字数では飽和しない。
                depth += 1;
            }
            CLOSE => match depth {
                // 対応する `「` が無い。区間の左端が決まらない。
                0 => return fallback(raw, FallbackReason::StrayClose),
                // 1段目を閉じた。ここまでが台詞。
                1 => {
                    depth = 0;
                    dialogues.push(std::mem::take(&mut current));
                }
                // 内側の `」`。**除去も置換もせず本文に残す**（ADR-0014）。
                _ => {
                    depth -= 1;
                    current.push(character);
                }
            },
            // 台詞の中の行区切りは `join_dialogues` 側で空白へ畳む。
            _ if depth > 0 => current.push(character),
            // CRLF は改行1つ（LF 側を捨てる）。
            LF if follows_carriage_return => {}
            _ if is_line_separator(character) => {
                after_carriage_return = character == CR;
                narration.break_line();
            }
            _ => narration.push(character),
        }
    }

    if depth > 0 {
        // 閉じないまま終わった。区間の右端が決まらない。
        // **入れ子を受理しても、閉じ足りない入力は崩れのまま**（`「あ「い」` は `Unclosed`）。
        return fallback(raw, FallbackReason::Unclosed);
    }

    let dialogue = join_dialogues(&dialogues);
    let narration = narration.finish();
    // 入れ子があれば台詞は必ず残る（内側の `「」` の2文字は空白ではないので、
    // 整形で消えることが無い）。それでも `dialogue` の有無を先に見る形にして、
    // **台詞が無いのに `StructuredNested` を返す**組み合わせを作らない。
    let outcome = match (&dialogue, &narration, nested) {
        (Some(_), _, true) => ParseOutcome::StructuredNested,
        (Some(_), _, false) => ParseOutcome::Structured,
        (None, Some(_), _) => ParseOutcome::NarrationOnly,
        (None, None, _) => ParseOutcome::Empty,
    };

    ParsedResponse {
        dialogue,
        narration,
        outcome,
        emphasis_pairs_stripped: body.pairs,
    }
}

/// 本文から Markdown の強調の印を落とす。**純粋関数**（作業単位 B-46）。
///
/// **規則そのものは `extraction::strip_emphasis` が持つ**（写しを作らない）。
/// ここがやるのは**行に割って1行ずつ渡し、行区切りをそのまま戻す**ことだけ。
///
/// # なぜ行に割るのか
///
/// **規則が行単位で書かれている**から。対にするのも、宙に浮いた印を落とすのも、
/// 「その行の中で」決まる。**割らずに全体へ当てると、`**朝` と `**夜` という
/// 別々の行に落ちた印が対になり、行をまたいで本文が書き換わる。**
///
/// **行区切りは1文字も変えずに戻す。** CRLF を1つに畳むのも、台詞の中の改行を
/// 空白にするのも[`parse_response`] の側の仕事で、ここが先に畳むと
/// **元は離れていた CR と LF が隣接する**（`parse_response` の `after_carriage_return`
/// の doc と同じ事情）。
pub fn strip_body_emphasis(raw: &str) -> StrippedBody {
    let mut text = String::with_capacity(raw.len());
    let mut pairs = 0_usize;
    let mut line = String::new();

    for character in raw.chars() {
        if is_line_separator(character) {
            push_stripped_line(&mut text, &mut pairs, &mut line);
            text.push(character);
        } else {
            line.push(character);
        }
    }
    push_stripped_line(&mut text, &mut pairs, &mut line);

    StrippedBody { text, pairs }
}

/// 組み立て中の1行を落としてから積む。**`line` は空になる。**
fn push_stripped_line(text: &mut String, pairs: &mut usize, line: &mut String) {
    match strip_emphasis_counted(line) {
        // **落とすものが無ければ元の行**（`None` はその区別そのもの）。
        None => text.push_str(line),
        Some(stripped) => {
            text.push_str(&stripped.text);
            *pairs = pairs.saturating_add(stripped.pairs);
        }
    }
    line.clear();
}

/// [`strip_body_emphasis`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrippedBody {
    /// 落とした後の本文。**行区切りは1文字も変わっていない。**
    pub text: String,
    /// 落とした**対**の数（[`ParsedResponse::emphasis_pairs_stripped`]）。
    pub pairs: usize,
}

/// 崩れたときの結果。**生出力を丸ごと地の文にし、台詞は拾わない**（ADR-0007）。
///
/// 整形もしない。崩れた出力を整形すると、調査時に何を見て崩れと判断したかが分からなくなる。
fn fallback(raw: &str, reason: FallbackReason) -> ParsedResponse {
    ParsedResponse {
        dialogue: None,
        narration: Some(raw.to_owned()),
        outcome: ParseOutcome::Fallback(reason),
        // **強調の印も落とさない。** 崩れた回に整形すると、
        // 何を見て崩れと判断したかが分からなくなる（この関数の doc）。
        emphasis_pairs_stripped: 0,
    }
}

/// 台詞を1行1つに並べて連結する。中身が空のものは数えない。
fn join_dialogues(dialogues: &[String]) -> Option<String> {
    let lines: Vec<String> = dialogues
        .iter()
        // 行区切りを空白に畳んでから前後を落とす。
        // **順序が逆だと畳んだ空白が端に残る**（`「\nあ\n」` が `" あ "` になる）。
        .map(|dialogue| trim_blank(&fold_line_separators(dialogue)).to_owned())
        .filter(|dialogue| !dialogue.is_empty())
        .collect();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// 組み立て中の地の文の1行。
struct NarrationLine {
    /// 台詞を抜いた残り。
    text: String,
    /// この行に台詞があったか。**「跡地」と「元から空の行」を区別する唯一の手掛かり。**
    had_dialogue: bool,
}

impl NarrationLine {
    const fn new() -> Self {
        Self {
            text: String::new(),
            had_dialogue: false,
        }
    }
}

/// 地の文を**行単位で**組み立てる。
///
/// 連結してから行に割り直す形にしないのは、そうすると
/// **「台詞を抜いた跡だけが残った行」と「元から空だった行」が区別できなくなる**ため。
/// 前者を残すと台詞1つにつき空行が1つ湧き（台詞を単独行に置くのは
/// 日本語小説で最も一般的な体裁）、後者まで落とすと段落の区切りが消える（ADR-0007）。
struct NarrationLines {
    /// 改行で確定した行。
    done: Vec<NarrationLine>,
    /// 組み立て中の行。
    current: NarrationLine,
}

impl NarrationLines {
    const fn new() -> Self {
        Self {
            done: Vec::new(),
            current: NarrationLine::new(),
        }
    }

    /// 地の文の1文字を足す。
    fn push(&mut self, character: char) {
        self.current.text.push(character);
    }

    /// 現在の行に台詞があったことを記録する。
    ///
    /// 台詞の中の行区切りは空白へ畳まれる（1行1台詞の不変条件）ので、
    /// **複数行にまたがる台詞でも地の文の行は増えない。**
    fn mark_dialogue(&mut self) {
        self.current.had_dialogue = true;
    }

    /// 行を1つ確定する。
    fn break_line(&mut self) {
        self.done
            .push(std::mem::replace(&mut self.current, NarrationLine::new()));
    }

    /// 地の文を仕上げる。**行頭の空白（字下げ）は保持する。**
    fn finish(mut self) -> Option<String> {
        self.done.push(self.current);

        let lines: Vec<String> = self
            .done
            .into_iter()
            .filter_map(|line| {
                // 行末の空白だけ落とす（行頭は触らない）。
                let text = trim_blank_end(&line.text).to_owned();
                if text.is_empty() && line.had_dialogue {
                    None // 台詞を抜いた跡地。残すと空行になる。
                } else {
                    Some(text) // 元から空だった行は段落の区切りとして残す。
                }
            })
            .collect();

        // 先頭と末尾の空行を落とす。**中間の空行は段落の区切りなので残す。**
        // スライスを削っていく形にしてあるので、添字も `unwrap` も出てこない。
        let mut kept: &[String] = &lines;
        while let Some((first, rest)) = kept.split_first() {
            if !first.is_empty() {
                break;
            }
            kept = rest;
        }
        while let Some((last, rest)) = kept.split_last() {
            if !last.is_empty() {
                break;
            }
            kept = rest;
        }

        if kept.is_empty() {
            None
        } else {
            Some(kept.join("\n"))
        }
    }
}

/// 行区切りを半角スペースへ畳む。**結果に改行は残らない。**
fn fold_line_separators(text: &str) -> String {
    // CRLF は空白1つ分にする（2つに割らない）。
    // ここは**台詞の中身だけ**を受け取る（生出力で連続した範囲）ので、
    // 隣接していない CR と LF を取り違える余地が無い。
    let unified = text.replace("\r\n", "\n");
    unified
        .chars()
        .map(|character| {
            if is_line_separator(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// 前後の空白を落とす（`domain::TRIMMABLE` と同じ集合）。
fn trim_blank(text: &str) -> &str {
    text.trim_matches(|character| TRIMMABLE.contains(&character))
}

/// 行末の空白だけ落とす。
fn trim_blank_end(text: &str) -> &str {
    text.trim_end_matches(|character| TRIMMABLE.contains(&character))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 集計キーの形の判定。**作業単位 B-40 で `crate::aggkey` へ引き上げた**
    /// （B-39 の時点ではこのモジュールの私有だった）。
    use crate::aggkey::fault_in_aggregation_key;

    /// 台詞だけを取り出す補助。
    fn dialogue(raw: &str) -> Option<String> {
        parse_response(raw).dialogue
    }

    /// 地の文だけを取り出す補助。
    fn narration(raw: &str) -> Option<String> {
        parse_response(raw).narration
    }

    /// 結果だけを取り出す補助。
    fn outcome(raw: &str) -> ParseOutcome {
        parse_response(raw).outcome
    }

    // ------------------------------------------------------------------
    // 正常系
    // ------------------------------------------------------------------

    /// ADR-0007 の例がそのまま分解できる。
    #[test]
    fn the_adr_example_splits_into_dialogue_and_narration() {
        let raw = "　ふと顔を上げて、彼女は目を細めた。「おかえりなさい」\n　声には少しだけ安堵が混じっている。";
        let parsed = parse_response(raw);

        assert_eq!(parsed.dialogue.as_deref(), Some("おかえりなさい"));
        assert_eq!(
            parsed.narration.as_deref(),
            Some("　ふと顔を上げて、彼女は目を細めた。\n　声には少しだけ安堵が混じっている。")
        );
        assert_eq!(parsed.outcome, ParseOutcome::Structured);
    }

    /// **地の文の字下げは行頭に残る。** 1行目だけ様式が崩れないこと。
    #[test]
    fn the_indentation_of_narration_is_preserved() {
        let parsed = parse_response("　地の文。「台詞」\n　次の行。\n");
        assert_eq!(
            parsed.narration.as_deref(),
            Some("　地の文。\n　次の行。"),
            "行頭の全角空白を落とすと1行目だけ字下げが消える"
        );
    }

    /// **複数の台詞は改行で連結し、1行が1つの台詞に対応する。**
    #[test]
    fn multiple_dialogues_are_joined_one_per_line() {
        let parsed = parse_response("「おはよう」と彼女は言った。「今日は晴れね」");

        assert_eq!(parsed.dialogue.as_deref(), Some("おはよう\n今日は晴れね"));
        assert_eq!(
            parsed.dialogue_lines().collect::<Vec<_>>(),
            vec!["おはよう", "今日は晴れね"]
        );
        assert_eq!(parsed.narration.as_deref(), Some("と彼女は言った。"));
        assert_eq!(parsed.outcome, ParseOutcome::Structured);
    }

    /// **台詞の中の改行は空白へ畳む**（1行1台詞の不変条件を守る）。
    #[test]
    fn line_breaks_inside_a_dialogue_are_folded_into_spaces() {
        let parsed = parse_response("「一行目\n二行目」「別の台詞」");

        assert_eq!(parsed.dialogue.as_deref(), Some("一行目 二行目\n別の台詞"));
        assert_eq!(parsed.dialogue_lines().count(), 2, "台詞は2つ");
    }

    /// 行区切りとして解釈されうる文字も畳む（`char::is_control` では足りない）。
    ///
    /// **`domain::LINE_BREAKS` の全文字を回す。** `prompt::safe` 側の
    /// `unicode_line_separators_become_newlines` と対で、共有した集合を固定している。
    #[test]
    fn every_line_separator_is_folded_inside_a_dialogue() {
        for separator in LINE_BREAKS {
            let parsed = parse_response(&format!("「あ{separator}い」"));
            assert_eq!(
                parsed.dialogue.as_deref(),
                Some("あ い"),
                "separator = {separator:?}"
            );
            assert_eq!(parsed.dialogue_lines().count(), 1);
        }
        // CRLF は改行1つ分（空白2つに割れない）。
        assert_eq!(dialogue("「あ\r\nい」").as_deref(), Some("あ い"));
    }

    /// 応答全体が台詞1つなら、地の文は `None`。
    #[test]
    fn a_response_that_is_only_dialogue_has_no_narration() {
        let parsed = parse_response("「ただいま」");
        assert_eq!(parsed.dialogue.as_deref(), Some("ただいま"));
        assert_eq!(parsed.narration, None);
        assert_eq!(parsed.outcome, ParseOutcome::Structured);
    }

    /// 台詞が無ければ全部が地の文。**崩れ扱いにしない。**
    #[test]
    fn a_response_without_dialogue_is_all_narration() {
        let parsed = parse_response("　彼女は黙って窓の外を見ていた。");

        assert_eq!(parsed.dialogue, None);
        assert_eq!(
            parsed.narration.as_deref(),
            Some("　彼女は黙って窓の外を見ていた。")
        );
        assert_eq!(parsed.outcome, ParseOutcome::NarrationOnly);
        assert!(!parsed.outcome.fell_back(), "地の文だけは正常系");
    }

    /// 台詞の前後の空白は落とす。
    #[test]
    fn a_dialogue_is_trimmed() {
        assert_eq!(dialogue("「 おかえり 」").as_deref(), Some("おかえり"));
        assert_eq!(
            dialogue("「\u{3000}おかえり\t」").as_deref(),
            Some("おかえり")
        );
        // 中間の空白は残す。
        assert_eq!(dialogue("「ただいま 葵」").as_deref(), Some("ただいま 葵"));
    }

    /// **空の `「」` は台詞として数えず、地の文にも残さない。**
    #[test]
    fn empty_quotes_produce_no_dialogue() {
        let parsed = parse_response("　彼女は口ごもった。「」");
        assert_eq!(parsed.dialogue, None);
        assert_eq!(parsed.narration.as_deref(), Some("　彼女は口ごもった。"));
        assert_eq!(parsed.outcome, ParseOutcome::NarrationOnly);

        // 空白だけの台詞も同じ扱い。
        assert_eq!(dialogue("「 \u{3000}\n 」").as_deref(), None);

        // 中身のある台詞と混ざっても、空の分だけが落ちる（空行を作らない）。
        let mixed = parse_response("「あ」「」「い」");
        assert_eq!(mixed.dialogue.as_deref(), Some("あ\nい"));
        assert_eq!(mixed.dialogue_lines().count(), 2);
    }

    /// 台詞も地の文も残らなければ両方 `None`。
    #[test]
    fn a_blank_response_yields_nothing() {
        for raw in ["", " ", "\n", "\r\n\u{3000}\t", "「」", "\n「 」\n"] {
            let parsed = parse_response(raw);
            assert_eq!(parsed.dialogue, None, "raw = {raw:?}");
            assert_eq!(parsed.narration, None, "raw = {raw:?}");
            assert_eq!(parsed.outcome, ParseOutcome::Empty, "raw = {raw:?}");
            assert!(!parsed.outcome.fell_back(), "raw = {raw:?}");
        }
    }

    /// 地の文の段落（中間の空行）は保つ。行末の空白だけ落とす。
    #[test]
    fn narration_keeps_inner_blank_lines_and_drops_trailing_spaces() {
        let parsed = parse_response("一段落目。  \n\n二段落目。  \n\n");
        assert_eq!(
            parsed.narration.as_deref(),
            Some("一段落目。\n\n二段落目。")
        );
    }

    /// 台詞を抜いた地の文は**そのまま連結**する（空白を補わない）。
    #[test]
    fn narration_fragments_are_concatenated_as_is() {
        assert_eq!(
            narration("彼女は「うん」と頷いた。").as_deref(),
            Some("彼女はと頷いた。")
        );
    }

    /// **単独行の台詞を抜いた跡に空行を残さない**（ADR-0007「台詞を抜いた跡の空行」）。
    ///
    /// 台詞を単独行に置くのは日本語小説で最も一般的な体裁で、モデルが最も高い頻度で出す形。
    /// 跡地を残すと**台詞 n 個につき空行が n 個増える。**
    #[test]
    fn a_dialogue_on_its_own_line_leaves_no_blank_line() {
        assert_eq!(
            narration("地の文1\n「台詞」\n地の文2").as_deref(),
            Some("地の文1\n地の文2")
        );

        // 字下げや行末の空白が付いていても「台詞だけの行」。
        assert_eq!(
            narration("地の文1\n　「台詞」  \n地の文2").as_deref(),
            Some("地の文1\n地の文2")
        );

        // 台詞が増えても空行は増えない。
        assert_eq!(
            narration("あ\n「x」\n「y」\nい").as_deref(),
            Some("あ\nい"),
            "台詞の数だけ空行が湧いてはいけない"
        );

        // 先頭・末尾に置かれた台詞も跡を残さない。
        assert_eq!(narration("「x」\nあ\n「y」").as_deref(), Some("あ"));
    }

    /// **元から空だった行は段落の区切りとして残す**（ADR-0007）。
    ///
    /// 跡地と一緒に畳むと本来の段落構造が消える。区別が要る。
    #[test]
    fn blank_lines_that_were_already_blank_are_kept() {
        assert_eq!(narration("あ\n\nい").as_deref(), Some("あ\n\nい"));
        assert_eq!(
            narration("あ\n\n「x」\nい").as_deref(),
            Some("あ\n\nい"),
            "台詞の跡地は落ちるが、その前の空行は段落区切りとして残る"
        );
        // 空白だけの行も「元から空」。
        assert_eq!(narration("あ\n\u{3000} \nい").as_deref(), Some("あ\n\nい"));

        // 台詞の前後に空行があれば、**両方とも**残る（元からある区切りは畳まない）。
        assert_eq!(
            narration("あ\n\n「x」\n\nい").as_deref(),
            Some("あ\n\n\nい"),
            "空行2つは元の出力に2つあったもの。跡地の1行だけが落ちる"
        );
    }

    /// **CR と LF は、生出力で隣接しているときだけ改行1つに畳む。**
    ///
    /// 台詞を抜いた後の文字列に対して CRLF を畳むと、
    /// **生出力では隣接していない CR と LF が1つの改行になる**（行が1つ消える）。
    #[test]
    fn a_cr_and_an_lf_merge_only_when_adjacent_in_the_raw_output() {
        // 隣接 → 改行1つ。
        assert_eq!(narration("あ「x」\r\nい").as_deref(), Some("あ\nい"));
        // 台詞を挟んで離れている → それぞれ改行。間の行は台詞の跡地なので落ちる。
        assert_eq!(narration("あ\r「x」\nい").as_deref(), Some("あ\nい"));
        // 間の行に地の文が残るなら、行は3つのまま。
        assert_eq!(
            narration("あ\r「x」い\nう").as_deref(),
            Some("あ\nい\nう"),
            "CR と LF を1つに畳むと「い」が「あ」と同じ行になる"
        );
        // CR + CRLF は改行2つ。
        assert_eq!(narration("あ\r\r\nい").as_deref(), Some("あ\n\nい"));
        // 台詞を挟んだ CR と LF を畳むと、後ろの空行まで巻き込んで段落が消える。
        assert_eq!(narration("あ\r「x」\n\nい").as_deref(), Some("あ\n\nい"));
    }

    /// **地の文でも `domain::LINE_BREAKS` の全文字が行を分ける。**
    ///
    /// 台詞側（`every_line_separator_is_folded_inside_a_dialogue`）と対。
    /// 片方だけ広げると、同じ文字が台詞では行区切り・地の文では素通しになる。
    #[test]
    fn every_line_separator_splits_the_narration() {
        for separator in LINE_BREAKS {
            assert_eq!(
                narration(&format!("あ{separator}い")).as_deref(),
                Some("あ\nい"),
                "separator = {separator:?}"
            );
            // 行が分かれていれば、単独行の台詞は跡地を残さずに落ちる。
            assert_eq!(
                narration(&format!("あ{separator}「x」{separator}い")).as_deref(),
                Some("あ\nい"),
                "separator = {separator:?}"
            );
        }
    }

    // ------------------------------------------------------------------
    // Markdown の強調（作業単位 B-46）
    // ------------------------------------------------------------------

    /// **台詞の中と外の両方から印が落ちる**（モジュールの doc「強調の印」）。
    ///
    /// **規則そのものの権威は `tests/emphasis_corpus.rs`**（手書きの表）。
    /// ここが見ているのは**落ちた先が台詞と地の文の両方である**ことで、
    /// 「台詞にだけ落とす」実装を緑にしないためにある。
    #[test]
    fn the_emphasis_is_stripped_from_the_dialogue_and_the_narration_alike() {
        let parsed = parse_response("「**朝**がいいよ」と**笑った**。");

        assert_eq!(parsed.dialogue.as_deref(), Some("朝がいいよ"));
        assert_eq!(parsed.narration.as_deref(), Some("と笑った。"));
        assert_eq!(parsed.outcome, ParseOutcome::Structured);
        // **対は2つ**（台詞の中に1つ、外に1つ）。
        assert_eq!(parsed.emphasis_pairs_stripped, 2);
    }

    /// **崩れた回は印も落とさない。** 生出力そのままが地の文に入る。
    ///
    /// # なぜ分けるか
    ///
    /// 崩れた回の地の文は**調査の材料**（ADR-0007「整形もしない」）。
    /// ここで印だけ落とすと、`raw_content` と地の文が「どこが違うか」で
    /// 2通りに割れ、崩れの原因を追うときに一致を当てにできなくなる。
    #[test]
    fn a_broken_response_keeps_its_emphasis_marks() {
        let raw = "彼女は**口をつぐんだ**。「ヒミツ";
        let parsed = parse_response(raw);

        assert_eq!(parsed.narration.as_deref(), Some(raw), "生出力を丸ごと");
        assert_eq!(parsed.dialogue, None);
        assert!(parsed.outcome.fell_back());
        // **落としていないので 0。** 落としてから捨てた、にもしない。
        assert_eq!(parsed.emphasis_pairs_stripped, 0);
        // 対照: 同じ本文が閉じていれば、印は落ちて数も動く。
        let closed = parse_response(&format!("{raw}」"));
        assert!(!closed.outcome.fell_back(), "前提: 閉じれば分解できること");
        assert_eq!(closed.emphasis_pairs_stripped, 1);
    }

    // ------------------------------------------------------------------
    // 崩れ方（フォールバック）
    // ------------------------------------------------------------------

    /// **`「` が閉じないまま終わったら、生出力を丸ごと地の文にする。**
    #[test]
    fn an_unclosed_quote_falls_back() {
        let raw = "　彼女は言った。「おかえりなさい";
        let parsed = parse_response(raw);

        assert_eq!(parsed.dialogue, None, "部分的に拾わない");
        assert_eq!(parsed.narration.as_deref(), Some(raw), "生出力を丸ごと");
        assert_eq!(
            parsed.outcome,
            ParseOutcome::Fallback(FallbackReason::Unclosed)
        );
    }

    /// 閉じなかったのが2つ目の台詞でも、**最初の台詞まで拾わない。**
    #[test]
    fn a_later_unclosed_quote_discards_the_earlier_dialogue() {
        let raw = "「おはよう」と言った。「今日は";
        let parsed = parse_response(raw);

        assert_eq!(parsed.dialogue, None, "拾えた分だけ拾う挙動にしない");
        assert_eq!(parsed.narration.as_deref(), Some(raw));
        assert!(parsed.outcome.fell_back());
    }

    /// **対応する `「` の無い `」` はフォールバック。**
    #[test]
    fn a_stray_closing_quote_falls_back() {
        for raw in ["」おかえり", "地の文」「台詞」", "「台詞」」"] {
            let parsed = parse_response(raw);
            assert_eq!(parsed.dialogue, None, "raw = {raw:?}");
            assert_eq!(parsed.narration.as_deref(), Some(raw), "raw = {raw:?}");
            assert_eq!(
                parsed.outcome,
                ParseOutcome::Fallback(FallbackReason::StrayClose),
                "raw = {raw:?}"
            );
        }
    }

    /// **フォールバック時の地の文は生出力そのまま**（整形しない）。
    ///
    /// 崩れた出力を整形すると、調査時に「パーサが何を見て崩れと判断したか」が
    /// 分からなくなる。ADR-0007 の「丸ごと」はここを指している。
    #[test]
    fn the_fallback_narration_is_verbatim() {
        let raw = "  　崩れた\r\n出力「  \n";
        let parsed = parse_response(raw);

        assert_eq!(parsed.narration.as_deref(), Some(raw));
        assert!(parsed.outcome.fell_back());
    }

    /// **フォールバックしたら `dialogue` は必ず `None`。**
    #[test]
    fn a_fallback_never_reports_a_dialogue() {
        // **入れ子（`「あ「い」う」`）はここに入れない。** ADR-0014 で正常系になった。
        for raw in [
            "「",
            "」",
            "「「",
            "」」",
            "「あ」」",
            "あ「い",
            "「あ「い」",
        ] {
            let parsed = parse_response(raw);
            assert!(parsed.outcome.fell_back(), "raw = {raw:?}");
            assert_eq!(parsed.dialogue, None, "raw = {raw:?}");
            assert_eq!(parsed.narration.as_deref(), Some(raw), "raw = {raw:?}");
        }
    }

    /// 最初に見つかった崩れの理由を返す（`」「` は迷子の閉じ）。
    ///
    /// **`「「` は崩れではなくなった**（ADR-0014）。閉じ足りなければ `Unclosed`。
    #[test]
    fn the_first_breakage_decides_the_reason() {
        assert_eq!(
            outcome("「あ「い").fallback_reason(),
            Some(FallbackReason::Unclosed)
        );
        assert_eq!(
            outcome("」「あ").fallback_reason(),
            Some(FallbackReason::StrayClose)
        );
        assert_eq!(
            outcome("「あ」「い").fallback_reason(),
            Some(FallbackReason::Unclosed)
        );
    }

    // ------------------------------------------------------------------
    // 入れ子の `「」`（ADR-0014）
    // ------------------------------------------------------------------

    /// **1段目の `「` から、対応する `」` までが台詞。** 内側の `「」` は本文に残る。
    ///
    /// ADR-0014 の模式（`「外「内」外」` → `外「内」外`）をそのまま置く。
    /// 壊れ方は2通りあり、どちらもここで落ちる。
    ///
    /// * 最初の `」` で切ると `外「内`
    /// * 内側の `「」` を除去すると `外内外`
    #[test]
    fn a_nested_quote_yields_the_first_level_span() {
        let parsed = parse_response("「外「内」外」");

        assert_eq!(parsed.dialogue.as_deref(), Some("外「内」外"));
        assert_eq!(parsed.narration, None, "台詞の外に残るものは無い");
        assert_eq!(parsed.dialogue_lines().count(), 1, "台詞は1つ");
        assert!(parsed.outcome.nested(), "入れ子だったことを記録すること");
        assert!(!parsed.outcome.fell_back(), "入れ子は崩れではない");
    }

    /// **多重の入れ子でも1段目が取れる。**
    ///
    /// 深さを数えずに「`「` を見たら次の `」` で閉じる」と書くと `あ「い「う` で切れる。
    #[test]
    fn a_deeply_nested_quote_still_yields_the_first_level_span() {
        let parsed = parse_response("「あ「い「う」え」お」");

        assert_eq!(parsed.dialogue.as_deref(), Some("あ「い「う」え」お"));
        assert_eq!(parsed.dialogue_lines().count(), 1);
        assert!(parsed.outcome.nested());
        assert!(!parsed.outcome.fell_back());
    }

    /// **実測した現物**（ADR-0012 の結論2 / ADR-0014 の背景）。
    ///
    /// `ユーモア = 10` × 失敗の告白で**再現した唯一の崩れ**がこの形だった。
    /// 内側の `『』` は素通し、内側の `「」` だけが入れ子として記録される。
    #[test]
    fn the_measured_nested_output_is_parsed_as_a_dialogue() {
        let raw = "「この前ね、『誰かが「生ケーキ」って言い出して大騒ぎ』だったんだよ」";
        let parsed = parse_response(raw);

        assert_eq!(
            parsed.dialogue.as_deref(),
            Some("この前ね、『誰かが「生ケーキ」って言い出して大騒ぎ』だったんだよ"),
            "実測で失われていた台詞が拾えること"
        );
        assert_eq!(parsed.narration, None);
        assert!(parsed.outcome.nested());
        assert!(
            !parsed.outcome.fell_back(),
            "この回が成功率の分母から外れないこと"
        );
    }

    /// **入れ子と通常の台詞が混在しても、それぞれ正しく取れる。**
    ///
    /// 地の文の側も、台詞を抜いた残りだけになる。
    #[test]
    fn a_nested_quote_mixes_with_ordinary_dialogues() {
        let parsed = parse_response("彼女は笑った。「ふつうの台詞」と続ける。「外側「内側」外側」");

        assert_eq!(
            parsed.dialogue_lines().collect::<Vec<_>>(),
            vec!["ふつうの台詞", "外側「内側」外側"],
            "入れ子でない台詞まで巻き込まないこと"
        );
        assert_eq!(
            parsed.narration.as_deref(),
            Some("彼女は笑った。と続ける。")
        );
        assert!(parsed.outcome.nested(), "1つでも入れ子があれば記録する");
    }

    /// **入れ子の中の行区切りも空白へ畳む**（1行1台詞の不変条件）。
    #[test]
    fn line_breaks_inside_a_nested_quote_are_folded() {
        let parsed = parse_response("「上の行「引用\n続き」下の行」");

        assert_eq!(
            parsed.dialogue.as_deref(),
            Some("上の行「引用 続き」下の行")
        );
        assert_eq!(parsed.dialogue_lines().count(), 1);
        assert!(parsed.outcome.nested());
    }

    /// **中身が内側の括弧だけでも台詞として数える**（`「」` の2文字は空ではない）。
    #[test]
    fn a_nested_quote_with_only_brackets_is_still_a_dialogue() {
        let parsed = parse_response("「「」」");

        assert_eq!(parsed.dialogue.as_deref(), Some("「」"));
        assert!(parsed.outcome.nested());
        assert!(!parsed.outcome.fell_back());
    }

    /// **入れ子は地の文の行に跡地を残さない**（通常の台詞と同じ扱い）。
    #[test]
    fn a_nested_quote_on_its_own_line_leaves_no_blank_line() {
        assert_eq!(
            narration("地の文1\n「外側「内側」外側」\n地の文2").as_deref(),
            Some("地の文1\n地の文2")
        );
    }

    /// **緩めるのは入れ子だけ。** 不均衡な入力の壊れ方は変わらない（ADR-0014）。
    #[test]
    fn an_unbalanced_input_still_falls_back() {
        // 閉じ切らない（`「あ「い」` は深さ1のまま終わる）。
        let unclosed = parse_response("「あ「い」");
        assert_eq!(
            unclosed.outcome,
            ParseOutcome::Fallback(FallbackReason::Unclosed),
            "入れ子を受理して閉じたことにしないこと"
        );
        assert_eq!(unclosed.dialogue, None);

        // 対応する `「` が無い `」` が先に来る。
        let stray = parse_response("」あ「い」");
        assert_eq!(
            stray.outcome,
            ParseOutcome::Fallback(FallbackReason::StrayClose)
        );
        assert_eq!(stray.dialogue, None);

        // 深い側が閉じ切らない形も同じ。
        assert_eq!(
            outcome("「あ「い「う」え").fallback_reason(),
            Some(FallbackReason::Unclosed)
        );
    }

    /// **入れ子は独自の集計キーを持つ**（`Structured` と混ぜない）。
    ///
    /// 混ぜると ADR-0014 の「発生率を追えるようにする」が成立しない。
    #[test]
    fn a_nested_quote_has_its_own_label() {
        let plain = outcome("「ふつうの台詞」");
        let nested = outcome("「外側「内側」外側」");

        assert_eq!(plain, ParseOutcome::Structured);
        assert_eq!(nested, ParseOutcome::StructuredNested);
        assert_ne!(
            plain.label(),
            nested.label(),
            "同じラベルだと入れ子の発生率が数えられない"
        );
        assert!(!plain.nested(), "入れ子でない台詞に印を付けないこと");
        assert!(nested.nested());
        assert_eq!(nested.fallback_reason(), None, "崩れではない");
    }

    // ------------------------------------------------------------------
    // `「」` 以外の引用符
    // ------------------------------------------------------------------

    /// **`『』` / `""` / `“”` は台詞にしない**（ADR-0007 は `「」` のみと決めている）。
    #[test]
    fn other_quotation_marks_are_not_dialogue() {
        for raw in [
            "彼女は『行く』と言った。",
            "彼女は\"行く\"と言った。",
            "彼女は“行く”と言った。",
            "彼女は〈行く〉と言った。",
            "彼女は【行く】と言った。",
        ] {
            let parsed = parse_response(raw);
            assert_eq!(parsed.dialogue, None, "raw = {raw:?}");
            assert_eq!(parsed.narration.as_deref(), Some(raw), "raw = {raw:?}");
            assert_eq!(
                parsed.outcome,
                ParseOutcome::NarrationOnly,
                "崩れ扱いにしない: raw = {raw:?}"
            );
        }
    }

    /// **`「」` の中の `『』` は入れ子と見なさない**（慣習どおりの正しい出力）。
    #[test]
    fn inner_double_brackets_do_not_count_as_nesting() {
        let parsed = parse_response("「彼は『行く』と言ったの」");

        assert_eq!(parsed.dialogue.as_deref(), Some("彼は『行く』と言ったの"));
        assert_eq!(parsed.outcome, ParseOutcome::Structured);
    }

    // ------------------------------------------------------------------
    // Unicode
    // ------------------------------------------------------------------

    /// アストラル面（サロゲートペア）と結合文字が壊れない。
    #[test]
    fn astral_and_combining_characters_survive() {
        let parsed = parse_response("　彼女は笑った😀。「が\u{3099}んばれ🎌」");

        assert_eq!(parsed.dialogue.as_deref(), Some("が\u{3099}んばれ🎌"));
        assert_eq!(parsed.narration.as_deref(), Some("　彼女は笑った😀。"));
    }

    /// 半角の `｢` `｣`（U+FF62 / U+FF63）は台詞の括弧ではない。
    ///
    /// 見た目が近いので拾いたくなるが、拾うと**バイト長の違う別の文字**を
    /// 同じ扱いにすることになり、ADR-0007 の決めた形式から静かにずれる。
    #[test]
    fn halfwidth_corner_brackets_are_not_dialogue_markers() {
        let raw = "彼女は｢おかえり｣と言った。";
        let parsed = parse_response(raw);

        assert_eq!(parsed.dialogue, None);
        assert_eq!(parsed.narration.as_deref(), Some(raw));
        assert_eq!(parsed.outcome, ParseOutcome::NarrationOnly);
    }

    /// **すべての Unicode スカラ値について panic しない。**
    ///
    /// `「`（U+300C）は3バイトなので、バイト境界を取り違えると即 panic する。
    /// 1文字だけ・台詞の中・台詞の外の3通りで全スカラ値を通す。
    #[test]
    fn every_unicode_scalar_value_is_safe() {
        for code in 0_u32..=0x10_FFFF {
            let Some(character) = char::from_u32(code) else {
                continue; // サロゲート符号位置はスカラ値ではない
            };

            for raw in [
                character.to_string(),
                format!("「{character}」"),
                format!("あ{character}「い」う"),
            ] {
                let parsed = parse_response(&raw);
                check_invariants(&raw, &parsed);
            }
        }
    }

    // ------------------------------------------------------------------
    // 性質テスト（ファズ）
    // ------------------------------------------------------------------

    /// 決定的な擬似乱数（xorshift64）。**依存クレートを増やさないための最小実装。**
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            let mut state = self.0;
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            self.0 = state;
            state
        }

        /// `0..bound` の値。`bound` は 0 でないこと（テスト内でのみ使う）。
        fn below(&mut self, bound: usize) -> usize {
            (self.next_u64() % bound as u64) as usize
        }
    }

    /// 崩れやすい文字を集めたアルファベット。
    const HOSTILE: [char; 20] = [
        '「', '」', '『', '』', '"', '“', '”', '｢', '｣', 'あ', 'a', ' ', '\u{3000}', '\t', '\n',
        '\r', '\u{0000}', '\u{2028}', '\u{2029}', '😀',
    ];

    /// 落としてよい文字（空白・行区切り）。整形で消えたり空白に変わったりする。
    fn is_droppable(character: char) -> bool {
        TRIMMABLE.contains(&character) || is_line_separator(character)
    }

    /// 括弧でも空白でもない文字を、並べ替えて取り出す。
    fn visible_characters(text: &str) -> Vec<char> {
        let mut characters: Vec<char> = text
            .chars()
            .filter(|c| *c != OPEN && *c != CLOSE && !is_droppable(*c))
            .collect();
        characters.sort_unstable();
        characters
    }

    /// どんな入力でも成り立つ性質。**ファズと全スカラ値走査の共通の判定。**
    ///
    /// 決定性（同じ入力なら同じ結果）は呼び出し回数が倍になるため、
    /// ここではなく `parsing_is_deterministic` とファズ側で確かめる。
    fn check_invariants(raw: &str, parsed: &ParsedResponse) {
        if parsed.outcome.fell_back() {
            // 1. フォールバックしたら、台詞は無く、地の文は生出力そのもの。
            assert_eq!(parsed.dialogue, None, "raw = {raw:?}");
            assert_eq!(parsed.narration.as_deref(), Some(raw), "raw = {raw:?}");
            return;
        }

        // 2. 正常系では1段目の括弧は消費されており、結果に残らない。
        //    **地の文には一切残らない。** 台詞には**入れ子のときだけ**内側の括弧が残る
        //    （ADR-0014。残っているなら `StructuredNested` でなければならない）。
        for text in [parsed.dialogue.as_deref(), parsed.narration.as_deref()]
            .into_iter()
            .flatten()
        {
            assert!(!text.is_empty(), "空文字は None にすること: raw = {raw:?}");
        }
        if let Some(narration) = parsed.narration.as_deref() {
            assert!(
                !narration.contains(OPEN) && !narration.contains(CLOSE),
                "地の文に括弧が残った: raw = {raw:?} / narration = {narration:?}"
            );
        }
        if let Some(dialogue) = parsed.dialogue.as_deref() {
            let has_bracket = dialogue.contains(OPEN) || dialogue.contains(CLOSE);
            assert_eq!(
                has_bracket,
                parsed.outcome.nested(),
                "括弧が残るのは入れ子のときだけ: raw = {raw:?} / dialogue = {dialogue:?}"
            );
            // 残った括弧は台詞ごとに対応が取れている（1段目を正しく取れた証拠）。
            for line in dialogue.lines() {
                let mut depth = 0_i64;
                for character in line.chars() {
                    match character {
                        OPEN => depth += 1,
                        CLOSE => depth -= 1,
                        _ => {}
                    }
                    assert!(depth >= 0, "台詞の中で括弧が閉じ過ぎた: raw = {raw:?}");
                }
                assert_eq!(depth, 0, "台詞の中で括弧が閉じていない: raw = {raw:?}");
            }
        }

        // 3. 台詞は「1行1台詞」。空行も、行頭・行末の空白も無い。
        if let Some(dialogue) = parsed.dialogue.as_deref() {
            assert!(
                dialogue.chars().all(|c| c == '\n' || !is_line_separator(c)),
                "行区切りが畳まれていない: raw = {raw:?} / dialogue = {dialogue:?}"
            );
            for line in dialogue.lines() {
                assert!(!line.is_empty(), "空の台詞が残った: raw = {raw:?}");
                assert_eq!(
                    line.trim_matches(|c| TRIMMABLE.contains(&c)),
                    line,
                    "台詞の前後に空白が残った: raw = {raw:?}"
                );
            }
        }

        // 4. 地の文は先頭・末尾の空行が無く、行末に空白が残らない。
        if let Some(narration) = parsed.narration.as_deref() {
            let lines: Vec<&str> = narration.split('\n').collect();
            assert!(
                lines.first().is_some_and(|line| !line.is_empty()),
                "先頭が空行: raw = {raw:?} / narration = {narration:?}"
            );
            assert!(
                lines.last().is_some_and(|line| !line.is_empty()),
                "末尾が空行: raw = {raw:?} / narration = {narration:?}"
            );
            for line in lines {
                assert_eq!(
                    line.trim_end_matches(|c| TRIMMABLE.contains(&c)),
                    line,
                    "行末に空白が残った: raw = {raw:?}"
                );
            }
        }

        // 5. **文字が失われない。** 括弧と空白以外は、台詞と地の文のどちらかに必ず残る。
        //
        //    **⚠ 比べる相手を「生出力」から「強調を落とした本文」へ変えた**
        //    （作業単位 B-46。**見張り番の意味を変えたので、司令塔の採否を仰いである**）。
        //    **落とすものが1つ増えた以上、生出力とは一致しない**——
        //    たとえば `` ` `` 1文字の入力は、宙に浮いた印として落ちて `Empty` になる。
        //
        //    **強調の印以外に対しては、以前と同じ強さで効いている。**
        //    落とし方そのもの（どの印を、どの条件で）は**この検査の担当ではない**
        //    ——権威は `tests/emphasis_corpus.rs`（手書きの表）。
        //    ここが見るのは**「強調を落とした後の本文から、さらに1文字も失わない」**こと。
        let reference = strip_body_emphasis(raw).text;
        let mut kept: Vec<char> = Vec::new();
        for text in [parsed.dialogue.as_deref(), parsed.narration.as_deref()]
            .into_iter()
            .flatten()
        {
            kept.extend(
                text.chars()
                    .filter(|c| *c != OPEN && *c != CLOSE && !is_droppable(*c)),
            );
        }
        kept.sort_unstable();
        assert_eq!(
            kept,
            visible_characters(&reference),
            "文字が失われた（または増えた）: raw = {raw:?} / parsed = {parsed:?}"
        );

        // 5b. **落ちる括弧は1段目の対だけ。** 内側の `「」` は本文に残る（ADR-0014）。
        //     期待値は `parse_response` と別系統（括弧列の深さ）で導く。
        let brackets = bracket_sequence(raw);
        let retained = [parsed.dialogue.as_deref(), parsed.narration.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(str::chars)
            .filter(|c| *c == OPEN || *c == CLOSE)
            .count();
        assert_eq!(
            retained,
            brackets.len() - top_level_brackets(raw),
            "1段目以外の括弧を落としている（または1段目を残している）: raw = {raw:?} / parsed = {parsed:?}"
        );

        // 6. 結果と `outcome` が食い違わない。
        match parsed.outcome {
            ParseOutcome::Structured | ParseOutcome::StructuredNested => {
                assert!(parsed.dialogue.is_some(), "raw = {raw:?}");
            }
            ParseOutcome::NarrationOnly => {
                assert!(parsed.dialogue.is_none(), "raw = {raw:?}");
                assert!(parsed.narration.is_some(), "raw = {raw:?}");
            }
            ParseOutcome::Empty => {
                assert!(
                    parsed.dialogue.is_none() && parsed.narration.is_none(),
                    "raw = {raw:?}"
                );
            }
            ParseOutcome::Fallback(_) => unreachable!("フォールバックは上で処理済み"),
        }
    }

    /// **でたらめな入力でも panic せず、性質が保たれる。**
    #[test]
    fn random_hostile_inputs_never_panic() {
        let mut rng = Rng(0x2026_0813_DEAD_BEEF);

        for _ in 0..20_000 {
            let length = rng.below(24);
            let mut raw = String::new();
            for _ in 0..length {
                if rng.below(8) == 0 {
                    // たまに完全にでたらめなスカラ値を混ぜる。
                    let code = (rng.next_u64() % 0x11_0000) as u32;
                    if let Some(character) = char::from_u32(code) {
                        raw.push(character);
                        continue;
                    }
                }
                let index = rng.below(HOSTILE.len());
                if let Some(character) = HOSTILE.get(index) {
                    raw.push(*character);
                }
            }

            let parsed = parse_response(&raw);
            check_invariants(&raw, &parsed);
            // 純粋関数（同じ入力なら同じ結果）。
            assert_eq!(parse_response(&raw), parsed, "raw = {raw:?}");
        }
    }

    /// 同じ入力なら必ず同じ結果（I/O も乱数も時刻も見ていない）。
    #[test]
    fn parsing_is_deterministic() {
        for raw in [
            "「あ」い",
            "地の文だけ",
            "「閉じない",
            "」迷子",
            "「入れ子「あ」」",
            "",
        ] {
            let first = parse_response(raw);
            for _ in 0..3 {
                assert_eq!(parse_response(raw), first, "raw = {raw:?}");
            }
        }
    }

    /// 括弧の対応が取れた入力は、必ず正常系として分解される（フォールバックしない）。
    ///
    /// **フォールバック判定が過剰になっていないこと**を押さえる。
    /// 「迷ったら全部フォールバック」にすれば panic はしないが、パーサとして無意味になる。
    #[test]
    fn well_formed_inputs_never_fall_back() {
        let mut rng = Rng(0x1234_5678_9ABC_DEF0);

        for _ in 0..5_000 {
            let mut raw = String::new();
            let mut expected = 0_usize;
            // **入れ子も「対応の取れた入力」に含める**（ADR-0014）。
            let mut expects_nested = false;
            for _ in 0..rng.below(6) {
                raw.push_str(match rng.below(3) {
                    0 => "地の文。",
                    1 => "　字下げ。",
                    _ => "\n",
                });
                if rng.below(2) == 0 {
                    let body = match rng.below(5) {
                        0 => "台詞",
                        1 => "台詞\n続き",
                        2 => "外「内」外",         // 入れ子
                        3 => "外「内「奥」内」外", // 多重の入れ子
                        _ => "",                   // 空の台詞は数えない
                    };
                    raw.push(OPEN);
                    raw.push_str(body);
                    raw.push(CLOSE);
                    if !body.is_empty() {
                        expected += 1;
                    }
                    expects_nested |= body.contains(OPEN);
                }
            }

            let parsed = parse_response(&raw);
            assert!(
                !parsed.outcome.fell_back(),
                "対応の取れた入力でフォールバックした: raw = {raw:?}"
            );
            assert_eq!(
                parsed.dialogue_lines().count(),
                expected,
                "台詞の数が合わない: raw = {raw:?} / parsed = {parsed:?}"
            );
            assert_eq!(
                parsed.outcome.nested(),
                expects_nested,
                "入れ子の記録が実態と合わない: raw = {raw:?} / parsed = {parsed:?}"
            );
            check_invariants(&raw, &parsed);
        }
    }

    /// 入力から括弧だけを取り出す。**期待値の導出は `parse_response` と別系統にする。**
    fn bracket_sequence(raw: &str) -> Vec<char> {
        raw.chars().filter(|c| *c == OPEN || *c == CLOSE).collect()
    }

    /// 括弧の対応が取れているか。**入れ子も「取れている」に数える**（ADR-0014）。
    ///
    /// 深さが負にならず、最後に 0 へ戻るかだけを見る。
    fn brackets_are_balanced(raw: &str) -> bool {
        let mut depth = 0_i64;
        for bracket in bracket_sequence(raw) {
            depth += if bracket == OPEN { 1 } else { -1 };
            if depth < 0 {
                return false;
            }
        }
        depth == 0
    }

    /// **1段目の括弧の数**（`「外「内」外」` なら 2）。
    ///
    /// 落ちるのはこれだけで、内側の `「」` は台詞本文に残る（ADR-0014）。
    /// 期待値の導出を `parse_response` と別系統にするため、括弧列の深さだけを見る。
    /// 対応の取れた入力にだけ使う。
    fn top_level_brackets(raw: &str) -> usize {
        let mut depth = 0_usize;
        let mut count = 0_usize;
        for bracket in bracket_sequence(raw) {
            if bracket == OPEN {
                if depth == 0 {
                    count += 1;
                }
                depth += 1;
            } else {
                if depth == 1 {
                    count += 1;
                }
                depth = depth.saturating_sub(1);
            }
        }
        count
    }

    /// 崩れの理由の期待値（最初の違反）。括弧列だけを見て決める。
    ///
    /// **入れ子はここに出てこない。** 深さが増えるだけで、崩れは
    /// 「深さが負になる」（`StrayClose`）と「0 に戻らない」（`Unclosed`）の2つ。
    fn expected_fallback_reason(raw: &str) -> Option<FallbackReason> {
        let mut depth = 0_i64;
        for bracket in bracket_sequence(raw) {
            depth += if bracket == OPEN { 1 } else { -1 };
            if depth < 0 {
                return Some(FallbackReason::StrayClose);
            }
        }
        (depth > 0).then_some(FallbackReason::Unclosed)
    }

    /// **括弧の対応が取れていない入力は、必ずフォールバックする。**
    ///
    /// `well_formed_inputs_never_fall_back` の逆向き。あちらは
    /// **過剰なフォールバック**しか押さえておらず、崩れの**見逃し**
    /// （たとえば入れ子の判定を落とす）を1つも検出しない。
    /// **性質は対で持たないと片側が丸ごと抜ける。**
    ///
    /// 期待値は `parse_response` とは**別の導出**で作る。同じ走査を書き写すと、
    /// 両方が同時に間違っても一致してしまう。
    #[test]
    fn ill_formed_inputs_always_fall_back() {
        let check = |raw: &str| {
            let parsed = parse_response(raw);
            assert_eq!(
                parsed.outcome.fallback_reason(),
                expected_fallback_reason(raw),
                "崩れの判定が期待と違う: raw = {raw:?} / parsed = {parsed:?}"
            );
            // 2つ目の導出とも突き合わせる（「対応が取れない ⟺ フォールバック」）。
            assert_eq!(
                parsed.outcome.fell_back(),
                !brackets_are_balanced(raw),
                "raw = {raw:?}"
            );
            check_invariants(raw, &parsed);
        };

        // 1. 括弧を含む短い文字列を**全数**（長さ5まで、1365通り）。
        //    `「あ「い」` のように「見逃すと崩れが素通りする」形がここに入る。
        const ALPHABET: [char; 4] = [OPEN, CLOSE, 'あ', '\n'];
        for length in 0..=5_u32 {
            for index in 0..ALPHABET.len().pow(length) {
                let mut rest = index;
                let mut raw = String::new();
                for _ in 0..length {
                    if let Some(character) = ALPHABET.get(rest % ALPHABET.len()) {
                        raw.push(*character);
                    }
                    rest /= ALPHABET.len();
                }
                check(&raw);
            }
        }

        // 2. 崩れやすい文字を混ぜた長めの入力。
        let mut rng = Rng(0x0BAD_C0DE_0007_0007);
        for _ in 0..5_000 {
            let mut raw = String::new();
            for _ in 0..rng.below(16) {
                let index = rng.below(HOSTILE.len());
                if let Some(character) = HOSTILE.get(index) {
                    raw.push(*character);
                }
            }
            check(&raw);
        }
    }

    // ------------------------------------------------------------------
    // 観測可能性（パース成功率）
    // ------------------------------------------------------------------

    /// **成功とフォールバックが区別できる。** 成功率の集計はこの区別に乗る。
    #[test]
    fn the_outcome_separates_success_from_fallback() {
        assert!(!outcome("「あ」い").fell_back());
        assert!(!outcome("いだけ").fell_back());
        assert!(!outcome("").fell_back());
        assert!(outcome("「あ").fell_back());
        assert!(outcome("」あ").fell_back());
        assert!(
            !outcome("「あ「い」う」").fell_back(),
            "入れ子は成功率の分母から外れない（ADR-0014）"
        );
    }

    /// すべての [`ParseOutcome`] をちょうど1回ずつ並べる（作業単位 B-39 の是正2）。
    ///
    /// **⚠ 手書きの配列に戻さないこと。** 是正前はここが手書きで、
    /// **7つ目の値を足しても下の形の検査が一度も見なかった** ——
    /// `FallbackReason::Planted => "fallback planted"`（**空白入りの集計キー**）を足した状態で
    /// **スイート全体が緑**だった（1905 / 0 / exit 0。B-39 のレビューが実測）。
    /// **コンパイラが強制したのは `tests/dialogue_corpus.rs` の `code()` の腕だけ**で、
    /// あれは**フィクスチャの写像へは誘導するが、形の検査へは誘導しない。**
    ///
    /// 並べ忘れと値の追加は [`all_outcomes_lists_every_value_exactly_once`] が見る。
    ///
    /// [`all_outcomes_lists_every_value_exactly_once`]: all_outcomes_lists_every_value_exactly_once
    const ALL_OUTCOMES: [ParseOutcome; 6] = [
        ParseOutcome::Structured,
        ParseOutcome::StructuredNested,
        ParseOutcome::NarrationOnly,
        ParseOutcome::Empty,
        ParseOutcome::Fallback(FallbackReason::Unclosed),
        ParseOutcome::Fallback(FallbackReason::StrayClose),
    ];

    /// [`ALL_OUTCOMES`] が**すべての値をちょうど1回ずつ**並べていること（是正2）。
    ///
    /// **`match` は網羅**なので、値が増えたら**このテストターゲットのコンパイルが止まる**
    /// （`parse::extraction` の `all_lists_every_reason_exactly_once` と
    /// `domain::drift` の `DriftDiscardReason::ALL` が同じ作法）。
    /// **並べ忘れ・重複は添字の突き合わせで赤になる。**
    ///
    /// **⚠ 綴りはここで固定しない。** それは `server::turn` の
    /// `every_outcome_is_recorded_under_its_own_label` の仕事で、
    /// ここへ写すと片方を直したときにもう片方が黙って古びる
    /// （CLAUDE.md「同じ発見を2箇所に書かない」）。
    #[test]
    fn all_outcomes_lists_every_value_exactly_once() {
        let mut seen: Vec<usize> = Vec::new();
        for outcome in ALL_OUTCOMES {
            // **網羅の `match`。** 値が増えたら、ここでコンパイルが止まる。
            seen.push(match outcome {
                ParseOutcome::Structured => 0,
                ParseOutcome::StructuredNested => 1,
                ParseOutcome::NarrationOnly => 2,
                ParseOutcome::Empty => 3,
                ParseOutcome::Fallback(FallbackReason::Unclosed) => 4,
                ParseOutcome::Fallback(FallbackReason::StrayClose) => 5,
            });
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            [0, 1, 2, 3, 4, 5],
            "ALL_OUTCOMES が値を並べ忘れているか、同じ値を2回並べている"
        );
    }

    /// ラベルは**値ごとに異なる定数**で、入力を反射せず、
    /// **ログの欄として読み戻せる形**である（作業単位 B-39）。
    ///
    /// 値の一覧は [`ALL_OUTCOMES`]（是正2）。**手書きの配列に戻さないこと**
    /// ——戻すと**7つ目の値がこの検査の下を素通りする**（あちらの doc に実測がある）。
    ///
    /// 形の判定は [`crate::aggkey::fault_in_aggregation_key`] に預けてある
    /// （**B-39 ではここの私有だった。作業単位 B-40 で2つ目の持ち主
    /// `domain::drift` が現れたので引き上げた**——写しを2つにしないため）
    /// ——**いまの6つの値はすべて前提を満たしている**ので、
    /// この検査だけでは「判定が広がったこと」を一度も観測できない
    /// （CLAUDE.md `13c-3a`「フィクスチャがその分岐が効く状態に一度も入らない」）。
    /// **広げたことの権威は [`the_aggregation_key_check_finds_each_fault`] と
    /// [`a_key_the_check_rejects_is_misread_by_the_field_readers`] の2本。**
    ///
    /// **⚠ 「一度も観測できない」は測ってある**（B-39 の変異検査）——
    /// **ここの呼び出しを `None` へ差し替えて判定を丸ごと使わなくしても 1905 全緑。**
    /// **逆向き（判定が何でも拒む）はここが赤になる**ので、
    /// **この検査が受け持つのは「広げすぎていないこと」の側だけ**である。
    /// **⚠ それでよい** ——この検査の存在理由は「いまの6つ」ではなく
    /// **「これから足される値」**で、そちらは [`ALL_OUTCOMES`] の `match` が型で強制する。
    ///
    /// [`the_aggregation_key_check_finds_each_fault`]:
    ///     crate::aggkey::tests::the_aggregation_key_check_finds_each_fault
    /// [`a_key_the_check_rejects_is_misread_by_the_field_readers`]:
    ///     crate::aggkey::tests::a_key_the_check_rejects_is_misread_by_the_field_readers
    #[test]
    fn labels_are_distinct_and_carry_no_input() {
        let mut labels: Vec<&str> = ALL_OUTCOMES.iter().map(|o| o.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), ALL_OUTCOMES.len(), "ラベルが重複している");

        for label in labels {
            assert_eq!(
                fault_in_aggregation_key(label),
                None,
                "集計キーに使える形にすること: {label:?}"
            );
        }
    }

    /// `fallback_reason` は理由を落とさない。
    #[test]
    fn the_fallback_reason_is_observable() {
        assert_eq!(ParseOutcome::Structured.fallback_reason(), None);
        assert_eq!(ParseOutcome::StructuredNested.fallback_reason(), None);
        assert_eq!(ParseOutcome::NarrationOnly.fallback_reason(), None);
        assert_eq!(ParseOutcome::Empty.fallback_reason(), None);
        assert_eq!(
            ParseOutcome::Fallback(FallbackReason::StrayClose).fallback_reason(),
            Some(FallbackReason::StrayClose)
        );
    }

    /// `nested` は**入れ子の値だけ**で真になる（他と混ざらない）。
    #[test]
    fn only_the_nested_outcome_is_nested() {
        assert!(ParseOutcome::StructuredNested.nested());
        for other in [
            ParseOutcome::Structured,
            ParseOutcome::NarrationOnly,
            ParseOutcome::Empty,
            ParseOutcome::Fallback(FallbackReason::Unclosed),
            ParseOutcome::Fallback(FallbackReason::StrayClose),
        ] {
            assert!(!other.nested(), "{other:?}");
        }
    }

    /// 台詞が無いときの `dialogue_lines` は0件（`None` の扱いで panic しない）。
    #[test]
    fn dialogue_lines_is_empty_without_a_dialogue() {
        assert_eq!(parse_response("地の文").dialogue_lines().count(), 0);
        assert_eq!(parse_response("").dialogue_lines().count(), 0);
        assert_eq!(parse_response("「あ").dialogue_lines().count(), 0);
    }
}
