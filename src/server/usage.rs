//! 上流が数えた実トークン数との**答え合わせ**（ADR-0013 第3層）。
//!
//! # なぜ毎回やるか
//!
//! 第2層（起動時の照会 / `super::context`）だけでは、**稼働中の変化を検出できない。**
//!
//! | 変化 | 起動時の照会 | ここ |
//! |---|---|---|
//! | 稼働中に別のコンテキスト長で再ロード | 気付けない | **次の1リクエストで分かる**（llama で窓を超えた回は 400 で断られ、ここは呼ばれない —— 分かるのは 200 で返った回だけ / ADR-0052 M5） |
//! | 上流を vLLM / Ollama へ差し替え | 既定値へ落ちるだけ | 同上（`usage` を返す上流なら） |
//! | チャットテンプレートの重いモデルへ変更 | 気付けない | **上界の破れとして出る**（バイト数の計画を送った回だけ。下の「2つの判定」） |
//!
//! # 2つの判定と、送った計画（ADR-0052 / 作業単位 B-60d-1）
//!
//! | 判定 | バイト数の計画 | 数えた計画（封をした回） |
//! |---|---|---|
//! | 上界の破れ（見積もり < 実） | 行う | **行わない**（計画の和が上界ではない —— 部品の和は全体を下回りうる / ADR-0052 §背景） |
//! | コンテキストの超過（実 ＋ 生成上限 > 想定コンテキスト） | 行う | **行う**（数えた計画は窓の9割前後を使うので、第2層が誤ったときの手掛かりはこれしか無い） |
//!
//! どちらかは呼び出し側が [`Estimate`] で渡す（`super::seal::TurnSeal::estimate`）。
//! **封をした回の答え合わせ（封 < 実）はここではない**（作業単位 B-60d-2 / ADR-0052 §決定5）。
//!
//! 壊れ方は上流によって違う。
//! * **LM Studio**: 窓を超えたプロンプトは前方から黙って切り捨てられる（ADR-0013）。
//!   どれも「エラーも出ず応答も返る」形で壊れるので、**ログだけが手掛かりになる。**
//! * **llama**: プロンプトそのものが窓を超えれば 400 `exceed_context_size_error` で断る
//!   （ADR-0052 M5。エラーとして見える）。ただし、プロンプトは窓に入り
//!   プロンプト ＋ 応答の上限が窓を超えるときは断られず、応答が黙って短くなる
//!   （`finish_reason=length`）——**こちらはエラーも出ず応答も返るので、ログだけが手掛かりになる。**
//!
//! # なぜ `server` に置くか
//!
//! 材料が `chat` の手元にしか揃わないため（`BudgetPlan` は組み立ての結果、
//! `TokenBudget` は `AppState`、`max_tokens` は要求を切り下げた後の値）。
//! **判断そのものは純粋関数**（`crosscheck` / `UsageCheck`）に閉じ込め、
//! `chat` からは `report` を1行呼ぶだけにしてある——`super::context::choose_budget` と同じ形。
//!
//! `prompt` 側へ持っていくと `prompt` が `llm::TokenUsage` に依存することになり、
//! **純粋な予算配分の層に上流の応答形式が染み出す。**
//!
//! # 毎リクエスト鳴ってよい警告か
//!
//! **よい。** ここで鳴る2つは「本来起きてはいけないこと」で、
//! **鳴っている間は実際に壊れている**（LM Studio ではプロンプトが黙って切り捨てられうる /
//! ADR-0013。llama では応答が黙って短くなりうる（`finish_reason=length` / ADR-0052 M5）——
//! ここは上流が 200 で返した回にしか呼ばれないので、llama が 400 で断った回はここに来ない）。
//! 記憶が予算を圧迫している旨の `warn`（docs/06-roadmap.md の既知の問題）は
//! **正常な運用の延長で鳴り続ける**のに対し、こちらは
//! **正常な運用では 1 回も鳴らない**——鳴ったら設定かモデルを直すまで直らない。
//! 抑制すると「直したかどうか」がログから読めなくなる。
//!
//! **一方で `usage` の欠落では鳴らさない。** `usage` を返さない上流の実装があり、
//! そちらは「答え合わせできない」だけで壊れてはいない。鳴らすと差し替えた瞬間から
//! 毎リクエスト流れ続け、上の2つが埋もれる。
//!
//! # ここで**やらないこと**
//!
//! * **予算の自動調整。** 観測だけに留める（実測を貯めてから別途判断する / ADR-0013）。
//! * **応答への反映。** 答え合わせの値は運用者向けであって、クライアントへ返さない。
//!
//! # 呼び出し口は2つ
//!
//! | 経路 | 呼ぶ場所 | `usage` の出どころ |
//! |---|---|---|
//! | 非ストリーミング | `server::chat` | 応答本文の `usage` |
//! | ストリーミング | `server::relay::finish_relay` | **最終チャンク**の `usage` |
//!
//! ストリーミングでは `usage` が最終チャンクにしか載らず、しかも
//! **`stream_options.include_usage` を要求しないと載らない**（`llm::wire_request`）。
//! 要求していても載せない上流はありうるので、**欠落で警告を出さない**方針は
//! どちらの経路でも同じ。

use crate::llm::TokenUsage;
use crate::prompt::{BudgetPlan, TokenBudget};

/// 答え合わせ1回分。**判断はここに閉じ込める**（`chat` は呼ぶだけ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsageCheck {
    /// こちらの見積もり（`BudgetPlan::prompt_tokens` = UTF-8 バイト数）。
    estimated_tokens: u32,
    /// **上流が数えた真値**（`usage.prompt_tokens`）。
    actual_tokens: u32,
    /// 上流へ渡した生成トークン上限（切り下げ後の値）。
    max_tokens: u32,
    /// こちらが想定しているコンテキスト長（第2層で決めた値）。
    context_tokens: u32,
}

impl UsageCheck {
    /// 材料をそのまま束ねる。**判断はメソッド側**（ここでは何も決めない）。
    pub(crate) const fn new(
        estimated_tokens: u32,
        actual_tokens: u32,
        max_tokens: u32,
        context_tokens: u32,
    ) -> Self {
        Self {
            estimated_tokens,
            actual_tokens,
            max_tokens,
            context_tokens,
        }
    }

    /// **第1層の上界が破れたか**（`見積もり < 実トークン`）。
    ///
    /// 見積もりは「どのトークナイザでも1トークンは1バイト以上を覆う」ことに
    /// 依存した上界なので、ここが真になるのは**前提そのものが崩れた**とき。
    /// 等しい場合は破れていない（境界は含めない）。
    pub(crate) const fn upper_bound_broken(self) -> bool {
        self.actual_tokens > self.estimated_tokens
    }

    /// **実トークン数と生成上限の合計が、想定コンテキスト長を超えたか。**
    ///
    /// 超えていれば、上流はどこかを切り捨てないと応答を返せない。
    /// **`response_tokens`（生成用の余白）と比べないこと。** 余白と比べると
    /// 正常なリクエストのほとんどが超過扱いになり、警告が毎回鳴って意味を失う。
    ///
    /// `u64` で足すのは、`u32` の飽和で**超過を小さく見せない**ため。
    pub(crate) fn overruns_the_context(self) -> bool {
        u64::from(self.actual_tokens) + u64::from(self.max_tokens) > u64::from(self.context_tokens)
    }

    /// 実トークン数は見積もりの何 % か。**どれだけ無駄にしているかの目安。**
    ///
    /// 見積もりが 0 なら `None`（0 で割らない）。
    pub(crate) fn actual_percent_of_estimate(self) -> Option<u64> {
        (self.estimated_tokens > 0)
            .then(|| u64::from(self.actual_tokens) * 100 / u64::from(self.estimated_tokens))
    }
}

/// 答え合わせの材料を束ねる。**純粋関数**（ログも通信も持たない）。
///
/// **`usage` が無ければ `None`。** 「答え合わせできない」を `0` などで埋めないこと。
/// 埋めると `見積もり ≥ 実トークン` が常に成立して見え、第3層が黙って死ぬ。
pub(crate) fn crosscheck(
    plan: &BudgetPlan,
    budget: TokenBudget,
    max_tokens: u32,
    usage: Option<TokenUsage>,
) -> Option<UsageCheck> {
    let usage = usage?;

    Some(UsageCheck::new(
        // **見積もりは削った後の実際の値**（`fixed_tokens` ではない）。
        plan.prompt_tokens(),
        usage.prompt_tokens,
        max_tokens,
        // **比べる相手はコンテキスト全体。** 生成用の余白ではない。
        budget.context_tokens(),
    ))
}

/// **送った計画の和が何を言っているか**（ADR-0052 / 作業単位 B-60d-1 のレビュー 要修正1）。
///
/// | 値 | 計画の和 | 上界の判定 | 超過の判定・照合の `debug` |
/// |---|---|---|---|
/// | `UpperBound` | バイト数（どのトークナイザでも上界） | **行う** | 行う |
/// | `Planning` | 上流が数えた部品の和（**上界ではない** / ADR-0052 §背景） | **行わない**（毎ターン鳴る嘘の警告になる） | **行う**（数えた計画は窓の9割前後を使うので、第2層が誤ったときの手掛かりはこれしか無い） |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Estimate {
    UpperBound,
    Planning,
}

/// 1リクエスト分の答え合わせを記録する。**`chat` から呼ぶのはこれだけ。**
///
/// `estimate` は送った計画の和が上界か（[`Estimate`]）。**外すのは上界の判定だけ**で、
/// コンテキストの超過の判定は毎回行う（ADR-0052 §推奨5 の表の4行目「今と同じ」）。
pub(crate) fn report(
    plan: &BudgetPlan,
    estimate: Estimate,
    budget: TokenBudget,
    max_tokens: u32,
    usage: Option<TokenUsage>,
) {
    match crosscheck(plan, budget, max_tokens, usage) {
        Some(check) => log_check(check, estimate),
        // **欠落で `warn` を出さないこと**（モジュールの doc）。
        None => tracing::debug!("上流が usage を返さないので答え合わせできません"),
    }
}

/// 判定結果をログへ流す。**深刻度の割り当てがこの関数の中身。**
///
/// **ミドルウェアから切り出した `body::log_oversized_body` と同じ形。**
/// ここを単体テストで観測できるようにするために、`report` から分けてある。
fn log_check(check: UsageCheck, estimate: Estimate) {
    if estimate == Estimate::UpperBound && check.upper_bound_broken() {
        // **静かに進ませない。** 切り分けに要る値（見積もり・実トークン・想定
        // コンテキスト）を載せる。**応答には載せない**（運用者向けの情報）。
        tracing::warn!(
            estimated_tokens = check.estimated_tokens,
            actual_tokens = check.actual_tokens,
            context_tokens = check.context_tokens,
            "トークン見積もりの上界が破れました（上流が数えた実トークン数の方が多い）"
        );
    }

    if check.overruns_the_context() {
        tracing::warn!(
            actual_tokens = check.actual_tokens,
            max_tokens = check.max_tokens,
            context_tokens = check.context_tokens,
            "実トークン数と生成上限の合計が想定コンテキスト長を超えました（上流が切り捨てている可能性があります）"
        );
    }

    // 無駄の量は**集計してから**考える材料なので `debug`（ADR-0013）。
    // 毎リクエスト出るため、既定フィルタに載せない。
    tracing::debug!(
        estimated_tokens = check.estimated_tokens,
        actual_tokens = check.actual_tokens,
        percent = ?check.actual_percent_of_estimate(),
        "トークン見積もりを実測と照合しました"
    );
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::domain::fixture_scale;
    use crate::domain::{
        Character, CharacterId, DisplayName, Handle, Memory, MemoryContent, MemoryId, Message,
        MessageId, Persona, Role, SessionId, Subject,
    };
    use crate::prompt::{CountedParts, PromptInput, build_prompt};
    use crate::testlog::{
        VERBOSE_FILTER, captured_under_default_filter, captured_with_filter, has_field,
        line_containing,
    };

    /// 上界が破れた側のログの手掛かり。**全文一致にしないこと**
    /// （文言の推敲で、見張りたい「既定フィルタで残るか」と無関係に割れる）。
    const UPPER_BOUND_MARKER: &str = "上界が破れ";

    /// 想定コンテキストを超えた側の手掛かり。**上と混ぜないこと**
    /// （調べる先が「見積もりの前提」と「コンテキスト長の設定」で違う）。
    const OVERRUN_MARKER: &str = "想定コンテキスト長を超え";

    /// 比率の記録（`debug`）の手掛かり。**既定フィルタには載らない側。**
    const RATIO_MARKER: &str = "実測と照合";

    /// `usage` の欠落を記録した側（`debug`）の手掛かり。
    const MISSING_MARKER: &str = "答え合わせできません";

    /// 答え合わせに使う想定コンテキスト長。**生成用の余白（1024）とは別の値。**
    const CONTEXT: u32 = 4_096;

    fn budget() -> TokenBudget {
        TokenBudget::new(CONTEXT, TokenBudget::DEFAULT_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること")
    }

    fn character() -> Character {
        Character {
            id: CharacterId::from_row(1),
            handle: Handle::parse("aoi").expect("正当な名前であること"),
            display_name: DisplayName::parse("葵").expect("正当な表示名であること"),
            base_persona: Persona::parse("落ち着いた口調で話す。")
                .expect("正当なペルソナであること"),
            output_format_instruction: None,
            stat_scale: fixture_scale(),
            stat_drift_mode: None,
            axis_discovery_slots: crate::domain::AxisDiscoverySlots::NONE,
            created_at: Utc::now(),
            archived_at: None,
        }
    }

    /// 記憶1件。**削れる枠を空にしないこと**（下の `plan` の doc）。
    fn memory(subject: Subject, content: &str) -> Memory {
        Memory {
            id: MemoryId::from_row(1),
            character_id: CharacterId::from_row(1),
            subject,
            content: MemoryContent::parse(content).expect("正当な本文であること"),
            category: None,
            created_at: Utc::now(),
            locked: false,
        }
    }

    /// 履歴1件。同上。
    fn message(role: Role, raw: &str) -> Message {
        Message {
            id: MessageId::from_row(1),
            session_id: SessionId::from_row(1),
            role,
            dialogue: None,
            narration: None,
            raw_content: raw.to_owned(),
            created_at: Utc::now(),
        }
    }

    /// **実際に組み立てたプロンプトの配分。**
    ///
    /// 見積もりの値を手で置かないのは、`crosscheck` が `BudgetPlan` の
    /// **どの項目**を読んでいるか（`prompt_tokens` か `fixed_tokens` か）を
    /// 実物で確かめるため。
    ///
    /// # 記憶と履歴を必ず載せること
    ///
    /// **どちらも空にすると `prompt_tokens() == fixed_tokens()` になり、
    /// この doc に書いた確認が原理的に成立しなくなる**（実測: どちらも 351 で、
    /// `crosscheck` を `fixed_tokens` に潰しても全件緑だった / 11c-2 のレビュー指摘）。
    /// 実運用では記憶・履歴が数千バイトを占めるので、そこが抜けた見積もりは
    /// 上流の実トークンに必ず負け、**「上界が破れました」が毎ターン鳴り続ける。**
    /// 前提は `assert` で固定してあるので、空へ戻すとテスト側が落ちる。
    fn plan() -> BudgetPlan {
        let memories = [
            memory(Subject::User, "ユーザーは猫を二匹飼っている。"),
            memory(Subject::Character, "葵は雨の日の匂いが好きだと話した。"),
        ];
        let history = [
            message(Role::User, "この前の話の続きを聞かせて。"),
            message(Role::Assistant, "「いいですよ」彼女は静かに頷いた。"),
        ];

        let plan = build_prompt(
            &PromptInput {
                character: &character(),
                stats: &[],
                attributes: &[],
                agreements: &[],
                memories: &memories,
                history: &history,
                user_message: "こんにちは",
            },
            budget(),
            &CountedParts::default(),
        )
        .expect("テストのプロンプトは予算に収まること")
        .plan;

        assert!(
            plan.memory_tokens() > 0 && plan.history_tokens() > 0,
            "前提: 記憶と履歴が実際に採用されていること"
        );
        assert!(
            plan.prompt_tokens() > plan.fixed_tokens(),
            "前提: 合計見積もりと削れない枠が別の値であること"
        );

        plan
    }

    fn estimated() -> u32 {
        plan().prompt_tokens()
    }

    fn checked(actual_tokens: u32, max_tokens: u32) -> UsageCheck {
        crosscheck(
            &plan(),
            budget(),
            max_tokens,
            Some(TokenUsage {
                prompt_tokens: actual_tokens,
            }),
        )
        .expect("usage があれば答え合わせできること")
    }

    /// **見積もりが実トークン数を覆っていれば、それが正常。**
    ///
    /// 境界（ちょうど同じ）でも「破れた」にしない。
    #[test]
    fn an_estimate_that_covers_the_actual_count_is_not_a_signal() {
        let estimated = estimated();
        assert!(estimated > 1, "前提: 見積もりが 1 より大きいこと");

        for actual in [1, estimated / 2, estimated - 1, estimated] {
            let check = checked(actual, TokenBudget::DEFAULT_RESPONSE_TOKENS);
            assert!(
                !check.upper_bound_broken(),
                "actual = {actual} / estimated = {estimated}: 上界は破れていない"
            );
            assert!(
                !check.overruns_the_context(),
                "actual = {actual}: コンテキストも超えていない"
            );
        }
    }

    /// **見積もりを1でも超えたら上界が破れている。**
    ///
    /// ここが鳴らないと、チャットテンプレートの重いモデルへ変わったときに
    /// 窓を超える状態に落ちる。LM Studio では「エラーも出ず応答も返る」まま前方から黙って
    /// 切り捨てられる（ADR-0013）。llama では応答が黙って短くなりうる（`finish_reason=length` /
    /// ADR-0052 M5。llama が 400 で断った回は 200 で返らないので、ここには来ない）。
    #[test]
    fn an_actual_count_above_the_estimate_breaks_the_upper_bound() {
        let estimated = estimated();

        for actual in [estimated + 1, estimated * 2, u32::MAX] {
            assert!(
                checked(actual, TokenBudget::DEFAULT_RESPONSE_TOKENS).upper_bound_broken(),
                "actual = {actual} / estimated = {estimated}: 上界が破れたと判定すること"
            );
        }
    }

    /// **超過は「想定コンテキスト長」と比べる。生成用の余白とではない。**
    ///
    /// 余白（1024）と比べると、正常なリクエストのほとんどが超過扱いになり、
    /// 警告が毎回鳴って意味を失う。
    #[test]
    fn the_overrun_is_measured_against_the_assumed_context() {
        let max_tokens = TokenBudget::DEFAULT_RESPONSE_TOKENS;

        // 余白は超えているが、想定コンテキストには収まっている。
        let fits = checked(2_000, max_tokens);
        assert!(
            2_000 + max_tokens > TokenBudget::DEFAULT_RESPONSE_TOKENS,
            "前提: 生成用の余白は超えていること"
        );
        assert!(
            !fits.overruns_the_context(),
            "想定コンテキスト（{CONTEXT}）に収まるなら鳴らないこと"
        );

        // 境界: ちょうど収まるなら鳴らない。1 超えたら鳴る。
        assert!(!checked(CONTEXT - max_tokens, max_tokens).overruns_the_context());
        assert!(checked(CONTEXT - max_tokens + 1, max_tokens).overruns_the_context());
    }

    /// **`usage` が無ければ答え合わせしない。**
    ///
    /// ここを `0` で埋めると「見積もり ≥ 実トークン」が常に成立して見え、
    /// 第3層が丸ごと死ぬ（しかも全テストが緑のまま）。
    #[test]
    fn a_missing_usage_yields_no_crosscheck() {
        assert_eq!(
            crosscheck(
                &plan(),
                budget(),
                TokenBudget::DEFAULT_RESPONSE_TOKENS,
                None
            ),
            None,
            "答え合わせできないことを「合っていた」に潰さないこと"
        );
    }

    /// 比は**実トークン ÷ 見積もり**（どれだけ無駄にしているか）。
    #[test]
    fn the_ratio_shows_how_much_the_estimate_overshot() {
        // 日本語で 3 倍過大に計上している状態（ADR-0013 の実測に相当）。
        assert_eq!(
            UsageCheck::new(300, 100, 0, CONTEXT).actual_percent_of_estimate(),
            Some(33)
        );
        assert_eq!(
            UsageCheck::new(100, 100, 0, CONTEXT).actual_percent_of_estimate(),
            Some(100)
        );
        // 上界が破れている側は 100% を超える。
        assert_eq!(
            UsageCheck::new(100, 250, 0, CONTEXT).actual_percent_of_estimate(),
            Some(250)
        );
        // 0 で割らない。
        assert_eq!(
            UsageCheck::new(0, 100, 0, CONTEXT).actual_percent_of_estimate(),
            None
        );
    }

    /// **見積もりは組み上がったプロンプト全体。削れない枠だけではない。**
    ///
    /// `fixed_tokens` を読むと、記憶・履歴の分（実運用では数千バイト）が
    /// 見積もりから丸ごと抜ける。上流の実トークンはそれを含むので
    /// **`実トークン > 見積もり` が恒常的に真**になり、
    /// 「上界が破れました」の `warn` が**会話1ターンごとに鳴り続ける**。
    #[test]
    fn the_estimate_is_the_whole_prompt_not_just_the_fixed_frame() {
        let plan = plan();
        assert!(
            plan.prompt_tokens() > plan.fixed_tokens(),
            "前提: 記憶と履歴が載っていて、2つの値が別物であること（{} / {}）",
            plan.prompt_tokens(),
            plan.fixed_tokens()
        );

        let check = crosscheck(
            &plan,
            budget(),
            TokenBudget::DEFAULT_RESPONSE_TOKENS,
            Some(TokenUsage { prompt_tokens: 1 }),
        )
        .expect("usage があれば答え合わせできること");

        assert_eq!(
            check.estimated_tokens,
            plan.prompt_tokens(),
            "削った後の実際の見積もりを使うこと"
        );
        assert_ne!(
            check.estimated_tokens,
            plan.fixed_tokens(),
            "削れない枠だけを見積もりにしないこと"
        );
    }

    /// **生成上限は引数の値。予算の余白で代用しない。**
    ///
    /// `budget.response_tokens()` に取り違えると、クライアントが
    /// `max_tokens` を小さく指定したリクエストで超過判定が過大になる
    /// （逆に切り下げ後の値が余白と一致する構成では永久に露見しない）。
    #[test]
    fn the_generation_cap_comes_from_the_argument_not_from_the_budget() {
        let capped = 16;
        assert_ne!(
            capped,
            budget().response_tokens(),
            "前提: 余白と別の値であること（同値だと取り違えを区別できない）"
        );

        assert_eq!(
            checked(1, capped).max_tokens,
            capped,
            "切り下げ後の生成上限をそのまま使うこと"
        );
    }

    /// 極端な値でも panic しない（u32 の足し算で溢れない）。
    #[test]
    fn extreme_values_do_not_panic() {
        let check = UsageCheck::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX);
        assert!(!check.upper_bound_broken());
        assert!(check.overruns_the_context(), "u32 を飽和させて隠さないこと");
        assert_eq!(check.actual_percent_of_estimate(), Some(100));

        let zero = UsageCheck::new(0, 0, 0, 0);
        assert!(!zero.upper_bound_broken());
        assert!(!zero.overruns_the_context());
    }

    /// **上界の破れは既定のログ設定で見える。しかも切り分けに要る値が載っている。**
    ///
    /// `debug` に落とすと既定フィルタ（`warn,personacore=info`）で消え、
    /// **前提が崩れたことを知る唯一の手掛かりが運用中のサーバに残らない。**
    ///
    /// # 入力は「上界だけが破れている」状態にすること
    ///
    /// 実トークンが多いだけでコンテキストには余裕がある（400 + 1024 < 4096）。
    /// **超過側の文言が出ていないことも併せて主張する**——両方破れる入力だと、
    /// `log_check` の2つの `if` の述語を取り違えても文言は出てしまう
    /// （実測: `overruns_the_context` → `upper_bound_broken` の変異が緑を通った
    /// / 11c-2 のレビュー指摘）。
    ///
    /// **否定側（超過の文言が出ないこと）の前に、それが現に出せることを確かめる。**
    /// 超過側の行が丸ごと出せなくなっていると、この否定は何も見ていない。
    #[test]
    fn a_broken_upper_bound_is_visible_under_the_default_log_filter() {
        let check = UsageCheck::new(120, 400, 1_024, CONTEXT);
        assert!(
            check.upper_bound_broken() && !check.overruns_the_context(),
            "前提: 上界だけが破れている入力であること"
        );

        // **前提: 超過側の文言は現に出せる。**
        let overrunning = UsageCheck::new(4_000, 4_000, 1_024, CONTEXT);
        let verbose = captured_with_filter(VERBOSE_FILTER, || {
            log_check(overrunning, Estimate::UpperBound)
        });
        assert!(
            verbose.contains(OVERRUN_MARKER),
            "前提: 超過側の文言は現に出せること: {verbose:?}"
        );

        let output = captured_under_default_filter(|| {
            log_check(check, Estimate::UpperBound);
        });

        // **深刻度も3つの値も、上界の破れを告げる行の中で見る**（作業単位 B-2）。
        //
        // **⚠ いまこの捕捉に囮は無い**（B-2-2 / レビュー指摘 R-3）。
        // `log_check` は同じ呼び出しで最大3件（上界・超過・比率）を出すが、
        // **比率は `debug!` なので既定フィルタに載らず**、超過側は上の前提
        // （`!overruns_the_context()`）が塞いでいる。**2026-08-26 に捕捉を実測して
        // 1行しか無いことを確かめた**——出力全体で見ても結果は変わらない。
        // B-2 でここに書いた「比率の行の `estimated_tokens=` でも満たされる」は**誤り**だった。
        //
        // それでも行に絞ってあるのは、**この経路に既定フィルタへ載るイベントが
        // 1本増えた瞬間、出力全体で見る形が空振りになる**から。
        let broken = line_containing(&output, UPPER_BOUND_MARKER);
        assert!(
            broken.contains("WARN"),
            "深刻度は warn であること: {broken:?}"
        );
        // **切り分けに要る3つの値**（見積もり・実トークン・想定コンテキスト）。
        //
        // **欄はトークンとして突き合わせる**（作業単位 B-12）。
        // ここが `contains` だった間、**`actual_tokens` を 10 倍にする変異が
        // `cargo test --lib` を 1122 件全緑で通り抜けていた**
        // （`actual_tokens=4000` は `actual_tokens=400` を含む）。
        // **同じ変異を `tests/server_chat.rs` は赤にしていた**——
        // つまり `src/` 側が `tests/` 側より緩い、という逆転が起きていた。
        assert!(
            has_field(broken, "estimated_tokens=120"),
            "見積もりを載せること: {broken:?}"
        );
        assert!(
            has_field(broken, "actual_tokens=400"),
            "実トークン数を載せること: {broken:?}"
        );
        assert!(
            has_field(broken, "context_tokens=4096"),
            "想定コンテキスト長を載せること: {broken:?}"
        );
        // **文言と述語の結線を固定する。** ここが緩いと、超過していない
        // リクエストで「コンテキスト長を超えました」が鳴り、運用者は
        // `PERSONACORE_CONTEXT_TOKENS` を疑って空振りする。
        // **否定は出力全体で見る**（どの行にも無いことを主張したい）。
        assert!(
            !output.contains(OVERRUN_MARKER),
            "超えていないのに超過の文言を出さないこと: {output:?}"
        );
    }

    /// **想定コンテキストの超過も既定のログ設定で見える。**
    ///
    /// # 入力は「超過だけが起きている」状態にすること
    ///
    /// 見積もり（4000）は実トークン（4000）を覆っているので上界は破れていない。
    /// **これがまさに検出したいケース**（見積もりは正しいのに、生成上限を足すと
    /// コンテキストに収まらない）で、しかも上界も同時に破れている入力だと
    /// 述語の取り違えを区別できない。
    ///
    /// **否定側（上界の文言が出ないこと）の前に、それが現に出せることを確かめる。**
    #[test]
    fn a_context_overrun_is_visible_under_the_default_log_filter() {
        let check = UsageCheck::new(4_000, 4_000, 1_024, CONTEXT);
        assert!(
            check.overruns_the_context() && !check.upper_bound_broken(),
            "前提: 超過だけが起きている入力であること"
        );

        // **前提: 上界側の文言は現に出せる。**
        let broken = UsageCheck::new(120, 400, 1_024, CONTEXT);
        let verbose =
            captured_with_filter(VERBOSE_FILTER, || log_check(broken, Estimate::UpperBound));
        assert!(
            verbose.contains(UPPER_BOUND_MARKER),
            "前提: 上界側の文言は現に出せること: {verbose:?}"
        );

        let output = captured_under_default_filter(|| {
            log_check(check, Estimate::UpperBound);
        });

        // **深刻度も2つの値も、超過を告げる行の中で見る**（作業単位 B-2）。
        let overrun = line_containing(&output, OVERRUN_MARKER);
        assert!(
            overrun.contains("WARN"),
            "深刻度は warn であること: {overrun:?}"
        );
        assert!(
            has_field(overrun, "max_tokens=1024") && has_field(overrun, "context_tokens=4096"),
            "何と何を比べて超えたのかが分かること: {overrun:?}"
        );
        // **逆向きの結線も固定する。** 述語を取り違えると、超過している
        // （検出したい）リクエストでこちらの警告が丸ごと消える。
        // **否定は出力全体で見る。**
        assert!(
            !output.contains(UPPER_BOUND_MARKER),
            "破れていないのに上界の文言を出さないこと: {output:?}"
        );
    }

    /// **正常なら既定のログ設定には何も出さない。ただし比率は `debug` で残る。**
    ///
    /// 比は `debug`。ここが `warn` に上がると会話1ターンごとに鳴り続け、
    /// 本当に前提が崩れたときの警告が埋もれる（docs/06-roadmap.md の既知の問題）。
    ///
    /// # 「出ないこと」の前に「出ること」を確かめる
    ///
    /// 否定だけを主張すると、**イベントが丸ごと消えても緑のまま**になる。
    /// 実際に2つの壊れ方がある:
    ///
    /// * 比率の `debug!` を削除しても、否定側は「何も出なかった」ので緑
    ///   （ADR-0013 の「実測を貯めてから判断する」材料が静かに消える）。
    /// * `tracing` の callsite 単位のキャッシュ（interest）は
    ///   **最初に評価した subscriber の判断を焼き付ける**ため、他のテストが
    ///   先に既定フィルタで同じ行を通すと `never` に落ちうる。
    ///
    /// 先に `trace` フィルタで**イベント自体が出ている**ことを固定しておけば、
    /// どちらも赤で気付ける（`llm::response` の
    /// `an_unreadable_usage_stays_out_of_the_default_log_filter` と同じ形）。
    #[test]
    fn a_healthy_crosscheck_stays_out_of_the_default_log_filter() {
        let healthy = UsageCheck::new(400, 120, 1_024, CONTEXT);
        assert!(
            !healthy.upper_bound_broken() && !healthy.overruns_the_context(),
            "前提: どちらの警告も鳴らない入力であること"
        );

        // **まず「記録自体はしている」ことを確かめる。**
        let verbose =
            captured_with_filter(VERBOSE_FILTER, || log_check(healthy, Estimate::UpperBound));
        // **比率の行の中で見る**（作業単位 B-2）。
        //
        // **⚠ いまこの捕捉に囮は無い**（B-2-2 / レビュー指摘 R-3）。
        // `log_check` は最大3件出すが、**この入力はどちらの警告も鳴らない**
        // （すぐ上の前提がそう固定している）ので、`trace` で捕っても比率の1行だけ。
        // **2026-08-26 に実測して確かめた。**
        // B-2 でここに書いた「警告側の行の `estimated_tokens=` でも満たされる」は**誤り**だった。
        //
        // 行に絞ってあるのは、**この捕捉にイベントが1本増えた瞬間に
        // 出力全体で見る形が空振りになる**から。
        let ratio = line_containing(&verbose, RATIO_MARKER);
        // **`Some(30)` は `Debug` だが空白を含まない**ので、そのままトークンになる
        // （`contains` だと `percent=Some(300)` にも当たる / 作業単位 B-12）。
        assert!(
            has_field(ratio, "percent=Some(30)"),
            "前提: 実トークンが見積もりの何 % かを載せること: {ratio:?}"
        );
        assert!(
            has_field(ratio, "estimated_tokens=400") && has_field(ratio, "actual_tokens=120"),
            "前提: 比率の元になった2つの値も載せること: {ratio:?}"
        );

        let output = captured_under_default_filter(|| log_check(healthy, Estimate::UpperBound));

        assert!(
            output.is_empty(),
            "正常時は既定の設定で無音であること: {output:?}"
        );
    }

    /// **`usage` を返さない上流で警告を鳴らさない。**
    ///
    /// 欠落は「答え合わせできない」であって異常ではない。ここで鳴らすと
    /// vLLM / Ollama などに差し替えた瞬間、毎リクエスト警告が流れる。
    ///
    /// **否定の前に肯定を置く理由は上のテストと同じ**（イベントが丸ごと
    /// 消えても「何も出なかった」で緑になる形にしない）。
    #[test]
    fn a_missing_usage_never_warns() {
        // **プロンプト組立は観測窓の外で済ませる。** `prompt` 層のログ
        // （記憶を落とした等）を `report` の出力と取り違えないため。
        let plan = plan();

        // **まず「答え合わせできなかったこと自体は記録する」を固定する。**
        let verbose = captured_with_filter(VERBOSE_FILTER, || {
            report(
                &plan,
                Estimate::UpperBound,
                budget(),
                TokenBudget::DEFAULT_RESPONSE_TOKENS,
                None,
            );
        });
        assert!(
            verbose.contains(MISSING_MARKER),
            "前提: 欠落は `debug` で残すこと: {verbose:?}"
        );

        let output = captured_under_default_filter(|| {
            report(
                &plan,
                Estimate::UpperBound,
                budget(),
                TokenBudget::DEFAULT_RESPONSE_TOKENS,
                None,
            );
        });

        assert!(
            output.is_empty(),
            "usage の欠落で警告を出さないこと: {output:?}"
        );
    }

    /// `report` は**答え合わせの結果をそのまま**ログへ流す（配線の確認）。
    #[test]
    fn report_logs_the_crosscheck_of_the_given_plan() {
        let plan = plan();

        let output = captured_under_default_filter(|| {
            report(
                &plan,
                Estimate::UpperBound,
                budget(),
                TokenBudget::DEFAULT_RESPONSE_TOKENS,
                Some(TokenUsage {
                    prompt_tokens: u32::MAX,
                }),
            );
        });

        assert!(
            output.contains(UPPER_BOUND_MARKER),
            "usage があれば実際に判定して報告すること: {output:?}"
        );
    }

    /// **計画の和が上界でない回（数えた計画）は、上界の判定だけを外し、超過の判定は残す**
    /// （作業単位 B-60d-1 のレビュー 要修正1 / ADR-0052 §推奨5 の4行目「今と同じ」）。
    ///
    /// 入力は**両方が破れる**もの（見積もり 120 < 実 4000、4000 ＋ 1024 > 4096）。
    /// 同じ入力を `UpperBound` で回すと2行とも出ることを対照に置く（片方を消す変異が、どちらの側でも見える）。
    #[test]
    fn a_planning_estimate_skips_only_the_upper_bound() {
        let both = UsageCheck::new(120, 4_000, 1_024, CONTEXT);
        assert!(
            both.upper_bound_broken() && both.overruns_the_context(),
            "前提: 両方が破れる入力であること"
        );

        let bound = captured_under_default_filter(|| log_check(both, Estimate::UpperBound));
        assert!(
            bound.contains(UPPER_BOUND_MARKER) && bound.contains(OVERRUN_MARKER),
            "対照: 上界の計画では2行とも出ること: {bound:?}"
        );

        let planning = captured_under_default_filter(|| log_check(both, Estimate::Planning));
        assert!(
            !planning.contains(UPPER_BOUND_MARKER),
            "数えた計画の和は上界ではないので、上界の破れを言わないこと: {planning:?}"
        );
        let overrun = line_containing(&planning, OVERRUN_MARKER);
        assert!(
            overrun.contains("WARN") && has_field(overrun, "actual_tokens=4000"),
            "超過は数えた計画でも言うこと: {overrun:?}"
        );
        let verbose = captured_with_filter(VERBOSE_FILTER, || log_check(both, Estimate::Planning));
        assert!(
            verbose.contains(RATIO_MARKER),
            "照合の debug 行も毎回出すこと: {verbose:?}"
        );
    }
}
