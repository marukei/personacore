//! **性格ステータスの軸名が応答に漏れたことの検出**（作業単位 0011c / ADR-0018）。
//!
//! **I/O を持たない純粋関数のみ。** 外部通信も追加のクエリも足さない
//! ——見ているのは文字列の照合だけ（ADR-0018「検出は安い」）。
//!
//! # 何を検出するか
//!
//! **プロンプトに実際に載った軸名**が、応答の**地の文**／**台詞**に現れたか。
//! [`ParsedResponse`] の分解結果をそのまま材料にする。
//!
//! * **地の文と台詞は別の欄で数える。** 地の文の方が
//!   [ADR-0012](../../docs/adr/0012-stat-section-directive.md) の (A) の本体で、
//!   台詞内は違う壊れ方（ADR-0018）。
//! * **値 0 の軸は見ない。** プロンプトから軸ごと落ちている（CLAUDE.md 絶対制約）ので
//!   モデルはその軸名を読んでいない。判定は
//!   [`StatAlloc::appears_in_prompt`] を通し、**`prompt` 側の `filter` と同じ関数**にしてある
//!   ——片方だけ条件が動くと、ここが「読まれていない軸」を数え始める。
//! * **数値は検出しない。** `5` が性格値の漏れか「5分待って」かを出力から決める方法が無い
//!   （ADR-0018「できないことを見張るふりをしない」）。
//!
//! # 何をしないか
//!
//! **記録のためだけの判定で、応答には一切影響させない**（ADR-0018）。
//! 落とさない・書き換えない・エラーにしない。返すのは**軸名の一覧**だけで、
//! **応答本文は返さない**（呼び出し側がそのままログへ出せる形にするため）。
//!
//! # 精度について、doc に書いてよい範囲
//!
//! **これは門ではなくログである。** ADR-0018 が誤検出も見逃しも受け入れると宣言している。
//!
//! * **誤検出する。** 「共感」という軸名のキャラクターで「共感してくれてありがとう」と
//!   台詞に出れば1件と数える。短い軸名ほど当たりやすい。
//! * **見逃す。** 「面白い口調で」のように**軸名を使わずに同じ欠陥**が起きる形は捕まえられない。
//!   数えているのは**軸名の一致**であって「(A) そのもの」ではない。
//!   **0 件を「起きていない」と読まないこと。**
//!
//! # フォールバックした回
//!
//! 崩れた回は生出力が丸ごと地の文に入る（ADR-0007）ので、
//! **`「」` の中にあった軸名も「地の文の漏れ」として数える。**
//! 分解できていない回に台詞と地の文の区別は無いため、これは避けられない
//! ——`outcome` が同じ行に載っている（`server::turn`）ので、集計側で切り分けられる。

use crate::domain::StatAlloc;

use super::ParsedResponse;

/// 漏れた軸名。**地の文と台詞で別の欄**（ADR-0018）。
///
/// 借りているのは**渡された `StatAlloc` の軸名**（＝運用者が書いた値）だけで、
/// **応答本文は一切保持しない。** そのままログへ出せる。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AxisLeaks<'a> {
    narration: Vec<&'a str>,
    dialogue: Vec<&'a str>,
}

impl<'a> AxisLeaks<'a> {
    /// **地の文**に現れた軸名。渡された順を保つ。
    pub fn narration(&self) -> &[&'a str] {
        &self.narration
    }

    /// **台詞**に現れた軸名。渡された順を保つ。
    pub fn dialogue(&self) -> &[&'a str] {
        &self.dialogue
    }

    /// どちらの欄にも1件も無いか。
    pub fn is_empty(&self) -> bool {
        self.narration.is_empty() && self.dialogue.is_empty()
    }
}

/// 応答に軸名が漏れたかを調べる。**純粋関数**（I/O も時刻も乱数も見ない）。
///
/// **どんな入力でも panic せず、`Result` も返さない。** これは記録のための判定で、
/// 応答を止める材料ではない（ADR-0018 / CLAUDE.md「パーサは絶対に panic しない」）。
pub fn detect_axis_leaks<'a>(parsed: &ParsedResponse, stats: &'a [StatAlloc]) -> AxisLeaks<'a> {
    AxisLeaks {
        narration: leaked_axes(parsed.narration.as_deref(), stats),
        dialogue: leaked_axes(parsed.dialogue.as_deref(), stats),
    }
}

/// 1つの本文に現れた軸名を集める。**渡された順を保つ。**
///
/// 判定は素の部分文字列一致（`str::contains`）。正規化も大文字小文字の畳み込みもしない
/// ——軸名は運用者が書いた値をそのまま `StatAxis` にした文字列で、
/// プロンプトにも**おおむね**その綴りで載る（`prompt::sanitize_character`）。
///
/// # 綴りが一致しない軸名がある
///
/// `sanitize_character` は軸名を `SafeText::sanitize` に通すので、
/// **制御トークンの形**（`[…]` / `<…>` で**中身が ASCII の英数字・`_`・`/`・`|` だけ**）を
/// 含む軸名は、閉じ括弧の手前に空白が1つ入った形で載る
/// ——実測で `[BATTLE]` は `[BATTLE ]`、`HP<max>` は `HP<max >` になる
/// （`prompt::safe::neutralize_control_tokens`）。
/// **そういう軸名の漏れはここでは数えられない**（探すのは `[BATTLE]` の綴りのままなので）。
///
/// ADR-0018 が受け入れた「見逃し」の一種で、**運用上の影響は ASCII 本体の形だけ**
/// ——中身が非 ASCII の `[好み]` 型は崩れないので、綴りは一致する（実測）。
fn leaked_axes<'a>(text: Option<&str>, stats: &'a [StatAlloc]) -> Vec<&'a str> {
    let Some(text) = text else {
        return Vec::new();
    };

    stats
        .iter()
        // **`prompt` 側の `filter` と同じ述語。** 載らなかった軸は見ない（ADR-0018）。
        .filter(|alloc| alloc.appears_in_prompt())
        .map(|alloc| alloc.axis.as_str())
        .filter(|axis| text.contains(axis))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{StatAxis, StatScale, StatValue};
    use crate::parse::parse_response;

    /// フィクスチャの尺度。**既定と違う値**（`tests/common` の `fixture_scale` と同じ理由）。
    fn fixture_scale() -> StatScale {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        assert_ne!(
            scale,
            StatScale::DEFAULT,
            "フィクスチャの尺度は既定と違う値であること"
        );
        scale
    }

    fn value(raw: u32) -> StatValue {
        fixture_scale()
            .value(raw)
            .expect("尺度に収まる値であること")
    }

    fn alloc(axis: &str, raw: u32) -> StatAlloc {
        StatAlloc {
            axis: StatAxis::parse(axis).expect("正当な軸名であること"),
            value: value(raw),
        }
    }

    /// **地の文に漏れる軸。** 台詞用・0 の軸・漏れない軸と**別の文字列**。
    const NARRATION_AXIS: &str = "ユーモア";

    /// **台詞に漏れる軸。** 地の文用と別の文字列（同じだと欄の取り違えが検出できない）。
    const DIALOGUE_AXIS: &str = "ロジック";

    /// **値 0 の軸**（プロンプトに載らない＝検出の対象外）。
    const ZERO_AXIS: &str = "パッション";

    /// **どこにも漏れない軸。** 「常に全軸を返す」実装を検出するために要る。
    const CLEAN_AXIS: &str = "勇気";

    /// 応答に出るが軸名ではない語。**これを軸と間違えないこと**の material。
    const NOT_AN_AXIS: &str = "コーヒー";

    /// 軸の一覧。**先頭は漏れない軸、末尾は漏れる軸**にしてある。
    ///
    /// * **先頭** ——「常に先頭の軸を返す」「先頭の軸しか見ない」実装を緑にしない。
    /// * **末尾** ——**値 0 の軸を末尾に置かないこと。** 末尾が検出対象外だと、
    ///   「一覧の最後の軸を見ない」実装（`stats[..len - 1]`）と
    ///   「述語で落とした」結果が**同じ**になる。実測で、0 の軸を末尾に置いていた頃は
    ///   その変異が **839/839 緑**で通り抜けた（CLAUDE.md「最頻の穴」の位置版）。
    fn stats() -> Vec<StatAlloc> {
        assert_fixture_axes_are_distinct();
        let allocs = vec![
            alloc(CLEAN_AXIS, 4),
            // **値 0 の軸は中間に置く**（末尾に置かない理由は上）。
            alloc(ZERO_AXIS, 0),
            alloc(NARRATION_AXIS, 5),
            alloc(DIALOGUE_AXIS, 2),
        ];
        assert_the_last_axis_is_inspected_and_leaks(&allocs);
        allocs
    }

    /// **一覧の末尾が「検出の対象で、しかも実際に漏れる軸」であること。**
    ///
    /// 値ではなく**位置**についての前提なので、`assert_ne!` を軸名に置くだけでは足りない。
    fn assert_the_last_axis_is_inspected_and_leaks(allocs: &[StatAlloc]) {
        let last = allocs.last().expect("軸が1本以上あること");
        assert!(
            last.appears_in_prompt(),
            "末尾に値 0 の軸を置かないこと（末尾を見ない実装が緑になる）: {last:?}"
        );
        assert_eq!(
            last.axis.as_str(),
            DIALOGUE_AXIS,
            "末尾は実際に漏れる軸であること（漏れない軸だと同じく区別が付かない）"
        );
        assert!(
            LEAKS_IN_DIALOGUE.contains(DIALOGUE_AXIS),
            "前提: 末尾の軸が現に応答へ出る材料があること"
        );
    }

    /// **フィクスチャの軸名が互いに別の文字列であること。**
    ///
    /// 同値にすると欄の取り違えも「先頭しか見ない」も検出できなくなる
    /// （CLAUDE.md「最頻の穴」）。
    fn assert_fixture_axes_are_distinct() {
        let names = [
            NARRATION_AXIS,
            DIALOGUE_AXIS,
            ZERO_AXIS,
            CLEAN_AXIS,
            NOT_AN_AXIS,
        ];
        for (index, left) in names.iter().enumerate() {
            for right in &names[index + 1..] {
                assert_ne!(left, right, "フィクスチャの語は互いに別であること");
            }
            assert!(
                !left.is_empty(),
                "空の軸名は `StatAxis::parse` が拒否する（前提）"
            );
        }
    }

    /// **地の文にだけ軸名がある応答。** 台詞には別の軸も他の軸名も入っていない。
    const LEAKS_IN_NARRATION: &str =
        "（コーヒーを淹れながら、ユーモアたっぷりな口調で笑っている。）「おかえりなさい」";

    /// **台詞にだけ軸名がある応答。** 地の文には軸名が1つも無い。
    const LEAKS_IN_DIALOGUE: &str = "彼女はカップを置いた。「ロジックだけじゃ足りないよ」";

    /// **両方に別々の軸名がある応答。**
    const LEAKS_IN_BOTH: &str =
        "（ユーモアたっぷりな口調で。）「ロジックだけじゃ足りないよ」 彼女は笑った。";

    /// **軸名が1つも無い応答。**
    const CLEAN_REPLY: &str = "（コーヒーを淹れながら。）「おかえりなさい」";

    /// **値 0 の軸名が地の文に出る応答。** 検出してはいけない。
    const LEAKS_ONLY_A_ZERO_AXIS: &str = "（パッションを込めて話しかけてくる。）「ただいま」";

    /// 崩れた応答（`「` が閉じない）。**生出力が丸ごと地の文になる。**
    const BROKEN_WITH_AXIS: &str = "彼女は笑った。「ユーモアたっぷりに話すね";

    /// **地の文の軸名は地の文の欄で数える。** 台詞の欄は空のまま。
    #[test]
    fn an_axis_name_in_the_narration_is_counted_in_the_narration_field() {
        let stats = stats();
        let parsed = parse_response(LEAKS_IN_NARRATION);
        // 前提: 材料が実際に地の文と台詞へ割れていること。
        assert!(
            parsed
                .narration
                .as_deref()
                .is_some_and(|narration| narration.contains(NARRATION_AXIS)),
            "前提: 軸名が地の文側にあること: {parsed:?}"
        );
        assert!(
            parsed
                .dialogue
                .as_deref()
                .is_some_and(|dialogue| !dialogue.contains(NARRATION_AXIS)),
            "前提: 台詞側には無いこと: {parsed:?}"
        );

        let leaks = detect_axis_leaks(&parsed, &stats);

        assert_eq!(leaks.narration(), [NARRATION_AXIS]);
        assert_eq!(
            leaks.dialogue(),
            [] as [&str; 0],
            "地の文の漏れを台詞の欄に入れないこと"
        );
        assert!(!leaks.is_empty());
    }

    /// **台詞の軸名は台詞の欄で数える。** 地の文の欄は空のまま。
    #[test]
    fn an_axis_name_in_the_dialogue_is_counted_in_the_dialogue_field() {
        let stats = stats();
        let parsed = parse_response(LEAKS_IN_DIALOGUE);
        assert!(
            parsed
                .dialogue
                .as_deref()
                .is_some_and(|dialogue| dialogue.contains(DIALOGUE_AXIS)),
            "前提: 軸名が台詞側にあること: {parsed:?}"
        );
        assert!(
            parsed
                .narration
                .as_deref()
                .is_some_and(|narration| !narration.contains(DIALOGUE_AXIS)),
            "前提: 地の文側には無いこと: {parsed:?}"
        );

        let leaks = detect_axis_leaks(&parsed, &stats);

        assert_eq!(leaks.dialogue(), [DIALOGUE_AXIS]);
        assert_eq!(
            leaks.narration(),
            [] as [&str; 0],
            "台詞の漏れを地の文の欄に入れないこと"
        );
        assert!(!leaks.is_empty());
    }

    /// **両方に出たら両方の欄に載る。** しかも**別々の軸名**として。
    #[test]
    fn a_leak_in_both_places_is_recorded_in_both_fields() {
        assert_ne!(
            NARRATION_AXIS, DIALOGUE_AXIS,
            "前提: 2つの欄が別の値で埋まること"
        );

        let stats = stats();
        let leaks = detect_axis_leaks(&parse_response(LEAKS_IN_BOTH), &stats);

        assert_eq!(leaks.narration(), [NARRATION_AXIS]);
        assert_eq!(leaks.dialogue(), [DIALOGUE_AXIS]);
    }

    /// **値 0 の軸は、その軸名が地の文にあっても検出しない。**
    ///
    /// CLAUDE.md の絶対制約（0 の軸はプロンプトに載せない）と**対**の見張り番。
    /// 載っていない軸をここで数えると、**モデルが読んでいない語の一致**を
    /// 漏れとして記録することになる（ADR-0018）。
    #[test]
    fn a_zero_valued_axis_is_never_counted() {
        let stats = stats();
        let parsed = parse_response(LEAKS_ONLY_A_ZERO_AXIS);
        assert!(
            parsed
                .narration
                .as_deref()
                .is_some_and(|narration| narration.contains(ZERO_AXIS)),
            "前提: 0 の軸名が現に地の文にあること（無いから検出されないのではない）: {parsed:?}"
        );
        assert!(
            stats
                .iter()
                .any(|alloc| alloc.axis.as_str() == ZERO_AXIS && alloc.value.get() == 0),
            "前提: その軸が値 0 で渡っていること"
        );

        let leaks = detect_axis_leaks(&parsed, &stats);

        assert!(
            leaks.is_empty(),
            "プロンプトに載らなかった軸を数えないこと: {leaks:?}"
        );
    }

    /// **判定の条件は `prompt` の `filter` と同じ関数**（`StatAlloc::appears_in_prompt`）。
    ///
    /// 片方だけ条件が動いたら気づけるように、**同じ述語**を通していることを直接見る。
    #[test]
    fn only_the_axes_that_appear_in_the_prompt_are_inspected() {
        let leaking = alloc(ZERO_AXIS, 0);
        assert!(
            !leaking.appears_in_prompt(),
            "値 0 の軸はプロンプトに載らない（`prompt` の filter と同じ条件）"
        );
        assert!(alloc(ZERO_AXIS, 1).appears_in_prompt());

        // 述語が真を返す軸だけが検出される。
        let stats = vec![leaking];
        let leaks = detect_axis_leaks(&parse_response(LEAKS_ONLY_A_ZERO_AXIS), &stats);
        assert!(leaks.is_empty(), "{leaks:?}");
    }

    /// **漏れが無い回は両方の欄が空。** ただし「常に空」ではないことは他の本が見ている。
    #[test]
    fn a_clean_response_reports_no_leak() {
        let stats = stats();
        let parsed = parse_response(CLEAN_REPLY);
        assert!(
            parsed
                .narration
                .as_deref()
                .is_some_and(|narration| narration.contains(NOT_AN_AXIS)),
            "前提: 地の文に語があること（空だから漏れないのではない）: {parsed:?}"
        );

        let leaks = detect_axis_leaks(&parsed, &stats);

        assert!(leaks.is_empty(), "{leaks:?}");
        assert_eq!(leaks.narration(), [] as [&str; 0]);
        assert_eq!(leaks.dialogue(), [] as [&str; 0]);
    }

    /// **漏れた軸を全部返す。** 先頭の1本で打ち切らない。
    #[test]
    fn every_leaking_axis_is_reported_not_just_the_first() {
        const BOTH_AXES_IN_NARRATION: &str =
            "（ユーモアたっぷりな口調で、ロジックも交えて話している。）";

        let stats = stats();
        let parsed = parse_response(BOTH_AXES_IN_NARRATION);
        let leaks = detect_axis_leaks(&parsed, &stats);

        assert_eq!(
            leaks.narration(),
            [NARRATION_AXIS, DIALOGUE_AXIS],
            "漏れた軸を1本で打ち切らないこと（渡された順で全部）"
        );
        assert_eq!(leaks.dialogue(), [] as [&str; 0]);
    }

    /// **崩れた回でも判定する。** 生出力が丸ごと地の文なので、地の文の欄で数える。
    #[test]
    fn a_fallback_response_is_still_inspected() {
        let stats = stats();
        let parsed = parse_response(BROKEN_WITH_AXIS);
        assert!(
            parsed.outcome.fell_back(),
            "前提: この応答は崩れとして扱われること: {parsed:?}"
        );
        assert_eq!(
            parsed.narration.as_deref(),
            Some(BROKEN_WITH_AXIS),
            "前提: フォールバックは生出力を丸ごと地の文にする"
        );

        let leaks = detect_axis_leaks(&parsed, &stats);

        assert_eq!(
            leaks.narration(),
            [NARRATION_AXIS],
            "崩れた回を判定の対象から外さないこと"
        );
        assert_eq!(leaks.dialogue(), [] as [&str; 0]);
    }

    /// **軸が1つも無いキャラクターでも動く**（空を返すだけ / panic しない）。
    #[test]
    fn a_character_without_stats_never_leaks() {
        let leaks = detect_axis_leaks(&parse_response(LEAKS_IN_BOTH), &[]);

        assert!(leaks.is_empty(), "{leaks:?}");
    }

    /// **どんな入力でも panic しない**（絶対制約）。
    ///
    /// 空・空白・制御文字・壊れた括弧・巨大入力を通す。**戻り値は問わない。**
    #[test]
    fn detection_never_panics_on_degenerate_input() {
        let stats = stats();
        let long = "ユーモア".repeat(5_000);
        let cases = [
            "",
            " ",
            "\u{0000}\u{FEFF}",
            "」」」",
            "「",
            "「ユーモア",
            "ユーモア」",
            long.as_str(),
        ];

        for case in cases {
            let parsed = parse_response(case);
            let _ = detect_axis_leaks(&parsed, &stats);
            let _ = detect_axis_leaks(&parsed, &[]);
        }
    }

    /// **応答本文を持ち帰らない。** 返るのは軸名だけ。
    ///
    /// 呼び出し側（`server::turn`）はこれをそのままログへ出すので、
    /// ここに本文が混ざると**運用ログが会話の写しになる。**
    #[test]
    fn the_result_carries_axis_names_only_never_the_response_text() {
        let stats = stats();
        let leaks = detect_axis_leaks(&parse_response(LEAKS_IN_BOTH), &stats);

        let axes: Vec<&str> = stats.iter().map(|alloc| alloc.axis.as_str()).collect();
        for leaked in leaks.narration().iter().chain(leaks.dialogue()) {
            assert!(
                axes.contains(leaked),
                "渡された軸名以外を返さないこと: {leaked:?}"
            );
        }
        // 応答本文の断片（軸名ではない語）が混ざらないこと。
        let rendered = format!("{leaks:?}");
        assert!(
            !rendered.contains("おかえり") && !rendered.contains("彼女"),
            "応答本文を持ち帰らないこと: {rendered}"
        );
    }
}
