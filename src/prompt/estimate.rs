//! トークン数の**見積もり**。
//!
//! # これは見積もりであって正確なトークン化ではない
//!
//! 正確に数えるにはモデルのトークナイザ（Mistral の SentencePiece 語彙）が要るが、
//! そのために依存クレートを増やしていない。**代わりに、見積もりを「この1関数」に閉じ込めてある。**
//! 予算配分のロジック（`super::budget::plan`）はコストを `u32` で受け取るので、
//! 費用の出どころが変わっても変更を必要としない。
//!
//! # 上流が数えた値は、ここに入れない（ADR-0052 §見張り番の扱い / 作業単位 B-60c）
//!
//! > かつてここは「後でトークナイザを入れるときに触るのはここだけ」と書いていた。
//! > **その形では成り立たない**（ADR-0052 §背景）—— 上流に数えさせるのは I/O で、`prompt` は I/O を持たない。
//!
//! **この関数は、どのトークナイザでも成り立つバイト数の上界を返し続ける**（ADR-0052 §決定1 の (i)）。
//! 上流が数えた値は**別の型・別の名前**（`super::CountedParts`）が持ち、`super::build_prompt` は
//! 表に無い部品だけをこの関数で数える。**数えた値をこの関数の中へ入れないこと** ——
//! 入れると上界の名前で上界でない値が返り、この下の見張り番がすべて別の主張になる。
//!
//! # 過小評価しない
//!
//! 危険なのは**過小評価**の側だけ。少なく見積もるとプロンプトが上流の
//! コンテキスト長を超える。**LM Studio では前方から黙って切り捨てられる**（ADR-0013。
//! 最初に消えるのはシステムプロンプト＝ペルソナ・性格・記憶）。エラーは出ず応答も返るため、
//! **「性格を変えても応答が変わらない」という形だけが現れる。**
//! llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
//! プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
//! 多く見積もった場合の害は「記憶や履歴が必要より早く削られる」ことなので、
//! **両側の誤差を対称に扱わず、常に多めに倒す。**
//!
//! # 採用した上界とその根拠
//!
//! **UTF-8 の 1 バイトにつき 1 トークン**として数える。
//!
//! Mistral 系（tekken）を含む現行のトークナイザは**バイトレベル BPE** で、
//! 語彙に無い列はバイトへフォールバックする。**どのトークンも最低 1 バイトを消費する**ので、
//! 「UTF-8 バイト数」は真のトークン数の**証明できる上界**になる
//! （等号はすべてのバイトが単独トークンへ分解された最悪ケース）。
//! 見積もりに要るのは平均ではなく上界なので、これを採る。
//!
//! ## 文字数ベースの係数を採らなかった理由
//!
//! 「ASCII は N 文字/トークン」という係数は、**圧縮しづらい入力クラスで簡単に破れる。**
//!
//! | 入力 | 3文字/トークン | 2文字/トークン | 実コスト |
//! |---|---|---|---|
//! | `a b c d …`（空白区切りの単字 39文字） | 13 | 20 | 約 20 |
//! | UUID（36文字） | 12 | 18 | 18〜20 |
//! | `𠮷𩸽𠀋`（3文字 / 12バイト） | — | — | 最大 24 |
//!
//! `" a"` `" b"` はどの BPE でも 1 トークンで、**3文字/トークンを満たす語彙は実在しない。**
//! 2文字/トークンにしても、UUID・16進・base64 のような**ランダム寄りの ASCII** では
//! 余裕が 0 か負になる。しかもこれは「利用者がログを貼る」ときに現実に来る文字列クラスで、
//! 3,000 文字貼られれば誤差はそのまま数百トークンになる。
//!
//! 非 ASCII 側も同じ。**アストラル面の文字は 1 文字 = 4 バイト**なので、
//! 文字数に定数を掛ける方式では最悪 4 トークン/文字を取りこぼす
//! （`𠮷` のような JIS 外字は人名で現実に来る）。
//!
//! ## 代償
//!
//! 英文は実測比で約 4 倍、日本語散文は約 2〜3 倍に見積もる。記憶と履歴は必要より早く
//! 削られるが、**溢れる方向より安全**（上の「過小評価しない」を参照）。
//! 正確に数える側は ADR-0052 が受け持つ —— 上流に部品を数えさせ、その値を `super::CountedParts` で
//! `super::build_prompt` へ渡す（**この関数は変えない**。数えた値は上界ではなく、安全は送る前の封が持つ）。
//! 日本語では窓の約2割しか使っていなかった（origin#56 の実測。ADR-0052 §背景）。
//!
//! ## 実測との突き合わせ
//!
//! 上界の主張が現実の LM Studio でも成り立つかは、
//! `tests/prompt_estimate_calibration.rs` が `usage.prompt_tokens` と突き合わせる。
//! **`#[ignore]` 付きなので通常の `cargo test` では走らない**（LM Studio は常時稼働ではない）。
//! 手順はそのファイルの doc を参照。
//!
//! なお、メッセージ 1 件ごとに乗る役割名・区切りトークンは本文の長さに依らないため、
//! `MESSAGE_OVERHEAD_TOKENS` として別に加算する。

/// メッセージ1件あたりの固定オーバーヘッド。
///
/// チャットテンプレートの役割名・区切りトークンの分。本文の長さに依らず乗るので、
/// 短い発言が並ぶ履歴ではこちらが支配的になる。**履歴の見積もりから外さないこと。**
///
/// # この数字だけは証明で支えられていない
///
/// 本文側の見積もりには「UTF-8 バイト数はバイトレベル BPE の上界」という根拠があるが、
/// **テンプレート側は実測して転記した数字でしかない。** ここが過小だと、履歴40件で
/// 160 トークンの過小計上になり、LM Studio では前方から黙って切り捨てられる
/// ＝**最初に消えるのはシステムプロンプト**という、このモジュールが最も恐れている
/// 壊れ方をする（ADR-0013 / このファイル冒頭の「過小評価しない」を参照）。
/// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。
/// ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
///
/// 実測との突き合わせは
/// `tests/prompt_estimate_calibration.rs` の
/// `the_message_overhead_constant_is_not_below_the_measured_one` が行う。
/// **既定のシステム前置きを持つモデルへ切り替えたら、必ず測り直すこと。**
///
/// # 何で測ったか
///
/// **`ministral-3-14b-instruct-2512`（LM Studio）のチャットテンプレート。**
/// **綴りを機械可読で持っているのは `crate::measured` の一覧**で、
/// **起動時に上流が応答するモデルと突き合わせて、ずれていたら報告する**
/// （ADR-0033 §決定1 / 作業単位 E-2）。**この行の綴りを変えるなら、あちらも一緒に直すこと**
/// （`every_measured_constant_is_where_the_registry_says_it_is` が食い違いで落ちる）。
pub const MESSAGE_OVERHEAD_TOKENS: u32 = 8;

/// 文字列のトークン数を**保守的に**見積もる。
///
/// 戻り値は常に UTF-8 バイト数以上（＝真のトークン数の上界）。
/// 決して panic せず、極端に長い入力でも飽和して `u32::MAX` に留まる。
pub fn estimate_tokens(text: &str) -> u32 {
    // `usize` が `u32` に収まらない長さ（4GB 超）は飽和させる。
    // ここで `u32::MAX` に張り付いても、予算配分は飽和加算なので panic しない。
    u32::try_from(text.len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 見積もりが下回ってはいけない値（＝真のトークン数の上界）。
    ///
    /// バイトレベル BPE の1トークンは最低1バイトを消費するので、これが上界。
    fn upper_bound(text: &str) -> u32 {
        u32::try_from(text.len()).expect("テストの入力は u32 に収まること")
    }

    /// 代表的な入力クラス。**圧縮しづらい側を意図的に集めてある。**
    const SAMPLES: [&str; 12] = [
        "",
        "a",
        "The quick brown fox jumps over the lazy dog.",
        // 空白区切りの単字。`" a"` はどの BPE でも1トークンなので、
        // 「N文字/トークン（N >= 2）」の係数が最初に破れる形。
        "a b c d e f g h i j k l m n o p q r s t",
        // UUID。16進の乱雑な列は語彙に乗らずバイトへ落ちる。
        "f47ac10b-58cc-4372-a567-0e02b2c3d479",
        // base64 風。
        "aGVsbG8gd29ybGQhIHRoaXMgaXMgYSB0ZXN0Lg==",
        "猫が好き",
        "落ち着いた口調で話す。記憶は事実として扱うこと。",
        // アストラル面（1文字 = 4バイト）。人名で現実に来る。
        "𠮷𩸽𠀋",
        "🙂🙃🙂",
        "①②③Ω±§",
        "混在 mixed 𠮷 42",
    ];

    /// 空文字は 0 トークン。
    #[test]
    fn empty_text_costs_nothing() {
        assert_eq!(estimate_tokens(""), 0);
    }

    /// **どんな入力でも、見積もりは UTF-8 バイト数を下回らない。**
    ///
    /// これが `estimate_tokens` の唯一の約束。バイトレベル BPE の上限が
    /// 1トークン/バイトである以上、バイト数は**証明できる上界**であり、
    /// ここを下回った瞬間に「過小評価しない」という宣言が成り立たなくなる。
    ///
    /// **係数を緩める変異（ASCII を N 文字/トークンにする、非 ASCII を
    /// 文字数×定数にする）は、必ずこのテストで落ちる。**
    #[test]
    fn estimate_is_never_below_the_utf8_byte_length() {
        for text in SAMPLES {
            assert!(
                estimate_tokens(text) >= upper_bound(text),
                "上界を下回った: {text:?} → {} < {}",
                estimate_tokens(text),
                upper_bound(text)
            );
        }
    }

    /// 上と同じ性質を、**擬似乱数で生成した文字列**に対しても確かめる。
    ///
    /// 決定的な線形合同法で回すので、失敗したら同じ入力で再現できる。
    #[test]
    fn the_byte_length_bound_holds_for_generated_text() {
        // 面ごとに代表を取る（BMP / アストラル面 / ASCII 記号）。
        let alphabet: Vec<char> = " aZ0!-\u{00E9}\u{3042}\u{732B}\u{FF21}\u{20BB7}\u{1F600}"
            .chars()
            .collect();
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut text = String::new();

        for _ in 0..20_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let pick = (state >> 33) as usize % alphabet.len();
            text.push(alphabet[pick]);

            assert!(
                estimate_tokens(&text) >= upper_bound(&text),
                "上界を下回った: {text:?}"
            );
        }
    }

    /// **アストラル面は 1 文字 = 4 バイト。** 文字数×2 では取りこぼす。
    #[test]
    fn astral_plane_characters_cost_four_tokens_each() {
        let text = "𠮷𩸽𠀋";
        assert_eq!(text.chars().count(), 3);
        assert_eq!(text.len(), 12);
        assert!(
            estimate_tokens(text) >= 12,
            "1文字4バイトを見落とした: {}",
            estimate_tokens(text)
        );
    }

    /// **空白区切りの単字は圧縮できない。** `" a"` はどの BPE でも1トークン。
    #[test]
    fn space_separated_single_letters_are_not_compressible() {
        let text = "a b c d e f g h i j k l m n o p q r s t";
        // 単字20個 ＋ 区切りの空白19個。空白は後続の文字と1トークンに畳まれるので
        // 実コストは 20 前後まで上がる。3文字/トークン（13）では確実に足りない。
        assert!(
            estimate_tokens(text) >= 20,
            "空白区切りの単字を過小評価した: {}",
            estimate_tokens(text)
        );
    }

    /// **UUID は語彙に乗らない。** 36文字で 18〜20 トークンに達する。
    #[test]
    fn a_uuid_is_not_underestimated() {
        let text = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
        assert_eq!(text.len(), 36);
        assert!(
            estimate_tokens(text) >= 20,
            "UUID を過小評価した: {}",
            estimate_tokens(text)
        );
    }

    /// 日本語は 1 文字 1 トークンを上回る見積もりにする。
    #[test]
    fn japanese_estimate_is_conservative() {
        let text = "猫".repeat(200);
        assert!(
            estimate_tokens(&text) > 200,
            "日本語を1文字1トークンで数えないこと: {}",
            estimate_tokens(&text)
        );
    }

    /// 文字を足して見積もりが減らない（単調非減少）。
    #[test]
    fn estimate_never_decreases_when_text_grows() {
        let mut text = String::new();
        let mut previous = estimate_tokens(&text);
        for character in "abcあ猫 zピ𠮷".chars() {
            text.push(character);
            let current = estimate_tokens(&text);
            assert!(current >= previous, "text = {text:?}");
            previous = current;
        }
    }

    /// 連結した文字列の見積もりは、部分の見積もりの和を下回らない。
    ///
    /// 下回ると「枠ごとに見積もって足す」`super::build_prompt` の計算が
    /// 全体より小さくなり、予算配分が壊れる。
    #[test]
    fn estimate_is_superadditive_over_concatenation() {
        for left in SAMPLES {
            for right in SAMPLES {
                let joined = format!("{left}{right}");
                assert!(
                    estimate_tokens(&joined)
                        >= estimate_tokens(left).saturating_add(estimate_tokens(right)),
                    "連結で見積もりが減った: {left:?} + {right:?}"
                );
            }
        }
    }

    /// 極端に長い入力でも panic しない。
    #[test]
    fn very_long_text_does_not_panic() {
        let text = "あ".repeat(500_000);
        assert_eq!(estimate_tokens(&text), 1_500_000);
    }

    /// オーバーヘッドは 0 ではない（履歴の件数が効くようにする）。
    ///
    /// 短い発言が並ぶ履歴では本文よりこちらが支配的になるので、0 にすると
    /// 「50件の『はい』」が予算をまったく食わない見積もりになる。
    ///
    /// **これは下限の確認でしかない。** 値が実測に足りているか（`8 → 1` の変異）は
    /// 上流のトークナイザにしか答えられないので、
    /// `tests/prompt_estimate_calibration.rs` の
    /// `the_message_overhead_constant_is_not_below_the_measured_one` が受け持つ。
    #[test]
    fn message_overhead_is_not_zero() {
        const { assert!(MESSAGE_OVERHEAD_TOKENS > 0) };
    }
}
