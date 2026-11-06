//! **集計キーの綴りが、ログの欄として読み戻せる形かどうか**の判定。
//! **テスト専用**（`#[cfg(test)]`。`testlog` と同じ理由で、フィーチャの下へ出さない）。
//!
//! # なぜ独立したモジュールなのか（作業単位 B-40）
//!
//! この判定は **作業単位 B-39 が `src/parse/mod.rs` の私有として書いた。**
//! あのとき持ち主は1つ（`parse::ParseOutcome::label`）しか無く、
//! doc にこう書いてあった——
//! **「2つ目の持ち主が現れた時が引き上げ時。その持ち主は `src/domain/drift.rs` である」。**
//!
//! **B-40 がその2つ目。** `domain::drift` には
//! `DriftOutcome` / `DriftDiscardReason` / `DriftDelta` / `DriftForm` /
//! `DriftNote` / `DriftScoring` / `StatDriftMode` の**7つ**の集計キーが並び、
//! **どれもログの欄の値として載る**のに、形の検査は `!label.is_empty()` だけだった。
//!
//! **⚠ 着手時の作業単位は「6つ」と数えていて、`StatDriftMode` が抜けていた**
//! （B-40 の是正で足した。`D-6` の「写しから1列だけが抜ける」形）。
//! **どの型が並んでいるかを読むなら、この doc ではなく
//! `domain::drift::tests::every_aggregation_key` の中身である。**
//!
//! **⚠ そこが「権威」になるのは、`every_aggregation_key_is_readable_back_as_a_log_field` の
//! 2つの押さえと揃ったときだけ**——**列の総数**（`keys.len()`）が
//! 「列が丸ごと増える／減る」を止め、**持ち主ごとの綴りの突き合わせ**が
//! 「その列がその型の値から作られていること」を止める。
//! **B-40 の着手時は件数しか無く、`DriftForm` の列を `DriftDelta::ALL` から作る変異が
//! 1940 全緑・exit 0 を通った**（長さが一致する型どうしは件数では区別できない。
//! 是正で塞いだ）。
//!
//! **引き上げの理由は1つだけ——写しを2つにしないため**（CLAUDE.md
//! 「同じ発見を2箇所に書かない。片方を直しても、もう片方が黙って古びる」）。
//! **判定の本体も、その権威になる2本の対照も、リポジトリ全体でここに1つずつしか無い。**
//!
//! # なぜ `testlog` でも `testlog/assert.rs` でもないのか
//!
//! * **`testlog/assert.rs` は読み手側の道具箱**で、`tests/common/mod.rs` が
//!   `#[path]` で**同じファイルを読む**。あそこの `SHARED_HELPERS` の門は
//!   **「`tests/**` がこの定義を写したら赤」**という意味である。
//!   **⚠ 置いただけなら、門は何も言わない**——門は `SHARED_HELPERS` に
//!   **並べた綴りしか見ない**（`testlog` の `defines_a_shared_helper`）。
//!   **偽の主張になるのは、綴りを `SHARED_HELPERS` へ足したとき**で、
//!   そのとき門は「`tests/**` が一度も使わない述語の写しが戻った」と言うことになる。
//!   **置かない結論は変わらないが、理由はこちらである。**
//! * **`testlog` そのもの**も「`tests/` と共有するための場所」である（あちらの doc）。
//!   **ここは書き手側の主張**——「`label()` が返してよい綴り」の規約であって、
//!   ログを読む道具ではない。
//!
//! # なぜ `domain` でも `parse` でもないのか
//!
//! **どちらか一方に置くと、もう一方が他方の層を借りることになる**
//! （`parse::ParseOutcome` は `domain` の型ではない）。
//! そして**「logfmt の欄として使える形か」はドメインの不変条件ではない**
//! ——この規約が守っているのは**値の意味**ではなく**観測のされ方**である。
//! 本番コードは一切呼ばない（`#[cfg(test)]` なので呼べない）ので、
//! `domain` の公開面へ出す理由も無い。
//!
//! # ⚠ **写しが2つに戻ることを止める見張り番は無い**（B-40 で実測）
//!
//! `parse` 側の `use` を**弱い写しの定義**（空でないことだけ）へ差し替えると、
//! **1940 全緑・exit 0** だった。**守っているのはこの doc だけ**である。
//! `testlog` の `SHARED_HELPERS` / `SHARED_BODIES` に相当する走査は置いていない
//! ——**置かない判断を司令塔が下した**（B-40 の是正）。理由は2つ。
//!
//! * **写しが戻っても、判定の広がりの権威はこのファイルの2本のまま生きている。**
//!   壊れるのは「`parse` 側だけが緩む」であって、
//!   「判定そのものが空白しか見ていない状態へ戻る」ではない。
//! * **`SHARED_HELPERS` は綴りを列挙する形**なので、
//!   **名前を変えた写しは原理的に捕まらない**（あちらの doc の表）。
//!
//! # 使う側
//!
//! * `parse::tests::labels_are_distinct_and_carry_no_input`
//! * `domain::drift::tests::every_aggregation_key_is_readable_back_as_a_log_field`
//!
//! **どちらも受け持つのは「広げすぎていないこと」の側だけ**である
//! ——**現存する値はすべて前提を満たしている**ので、判定を広げても
//! あの2本は一度もその枝に入らない（CLAUDE.md `13c-3a`）。
//! **広げたことの権威は、このファイルの2本の対照。**

/// 集計キーの綴りが**ログの欄として読み戻せない**理由（作業単位 B-39 / B-40 で引き上げ）。
///
/// **`bool` にしないのは、囮ごとに「どの主張で鳴ったか」を固定するため。**
/// `bool` だと、3つの主張のどれか1つが常に鳴る変異と区別が付かない
/// （`prompt::safe` の `SpellingFault` と同じ作法。**あちらとは出所が別**で、
/// あちらが見ているのは `SafeText::sanitize` が触る集合である）。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeyFault {
    /// 空。`キー=` だけになり、**欄が在ることと値が在ることが区別できない。**
    Empty,
    /// **ASCII の図形文字でない**（空白・制御文字・非 ASCII）。
    ///
    /// 空白は欄の**トークンの区切り**で、そこで値が切れる。
    /// 制御文字は**ログの上で目に見えない。**
    NotGraphicAscii(char),
    /// **欄そのものの区切り**（`=` と `\u{22}`）。
    ///
    /// `=` はキーと値の境目、`\u{22}` は `Debug` が書く文字列の囲みである。
    Separator(char),
}

/// 集計キーの綴りが**ログの欄として読み戻せない**点を1つ返す（作業単位 B-39）。
///
/// **走査本体を引数で受け取る形にしてある**ので、
/// **実物のラベルと、テストが植えた綴りの両方を同じ判定に通せる**
/// （CLAUDE.md / `15b`。実物だけを見る検査は
/// 「1件も無いから緑」と「そもそも探せていないから緑」を区別できない）。
///
/// # ⚠ 判定の順番は**答えを変える**（**書いてから2回測って、2回書き直した**）
///
/// **B-39 の着手時の doc**: 「順番に意味がある。先に図形文字を見ないと、
/// 空白と制御文字が [`KeyFault::Separator`] の枝へ落ちる」——**誤り。**
/// 空白は `=` でも `\u{22}` でもないので、その枝には落ちない。
///
/// **1回目の書き直し**: 「`=` と `\u{22}` はどちらも ASCII の図形文字なので、
/// 2つの主張が受け持つ集合は互いに素。順番に意味は無い」——**これも誤り。**
/// **「文字の集合が互いに素」と「綴りの集合が互いに素」は別の主張である。**
/// **1つの綴りが両方に触れれば、答えは変わる。**
///
/// | 綴り | いまの順 | 入れ替えた順 |
/// |---|---|---|
/// | `"a=b c"`（区切りと空白を両方持つ） | `NotGraphicAscii(' ')` | `Separator('=')` |
///
/// **B-39 のレビューの実測**: 入れ替えのみ = 1905 全緑 ／ 囮を足すのみ = 1905 全緑 ／
/// **両方 = 1 赤。** **等価に見えていたのは、囮が
/// 「1つの綴りにつき違反1つ」しか持っていなかったから。**
/// **[`KeyFault`] を `bool` にしなかった理由が
/// 「どの主張で鳴ったかを名前で固定する」ことなので、名前の入れ替えはこの判定の中心にある。**
/// **見張っているのは [`the_aggregation_key_check_finds_each_fault`] の `"a=b c"` の行1本。**
///
/// # ⚠ 読み手が読める綴りも落とす。**2種類あり、理由が別**
///
/// `is_ascii_graphic()` は、`crate::testlog` の欄の道具が**読み戻せる**綴りまで落とす。
///
/// | 落とすもの | 読み手はどうか | なぜ落とすか |
/// |---|---|---|
/// | **空白でない ASCII 制御文字**（`\u{7F}` など） | **読める**（空白ではないのでトークンが切れない / B-39 で `\u{7F}` を実測） | **ログの上で目に見えない**（CLAUDE.md「見えない文字」） |
/// | **非 ASCII**（`構造化` など） | **読めるし、目にも見える** | **集計キーは ASCII に揃える**という規約。**B-39 より前からの継承**で、`parse::ParseOutcome::label` の doc の「`tracing` のフィールドやメトリクスのラベルにそのまま使える」がその前提 |
///
/// **2種類とも、実物の集計キーでは観測できない**（`parse` の6値も
/// `domain::drift` の値も、すべて ASCII の図形文字）。
/// **権威は [`the_aggregation_key_check_finds_each_fault`] の囮だけ。**
/// **厳しい側の偽陽性はうるさく落ちるだけ**で、逆向きは黙って通る。
///
/// [`the_aggregation_key_check_finds_each_fault`]:
///     tests::the_aggregation_key_check_finds_each_fault
pub(crate) fn fault_in_aggregation_key(label: &str) -> Option<KeyFault> {
    if label.is_empty() {
        return Some(KeyFault::Empty);
    }
    // ここが B-39 より前は `label.is_ascii() && !label.contains(' ')` だった。
    if let Some(character) = label.chars().find(|c| !c.is_ascii_graphic()) {
        return Some(KeyFault::NotGraphicAscii(character));
    }
    label
        .chars()
        .find(|c| *c == '=' || *c == '\u{22}')
        .map(KeyFault::Separator)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判定が、外れ方ごとに実際に鳴ること**（植えた綴りの対照 / 作業単位 B-39）。
    ///
    /// 実物のラベルだけを見る検査は、**「1件も無いから緑」と
    /// 「そもそも探せていないから緑」を区別できない**（CLAUDE.md / `15b`）。
    /// **実物の値はすべて前提を満たしている**ので、判定を広げても
    /// 使う側（`parse` / `domain::drift`）は一度もその枝に入らない。
    ///
    /// **⚠ 囮は判定から組み立てず、そのまま書く。** 引くと、
    /// **判定を壊す変異が囮も一緒に動かして緑になる。**
    ///
    /// **⚠ 拒む側だけでは足りない**（`#[should_panic]` と同じ形）——
    /// **「常に鳴る」判定を、下の「拒まない側」が止める。**
    ///
    /// **⚠ 「1つの綴りにつき違反1つ」の囮だけでは足りない**（B-39 の是正1）——
    /// **2つの主張の順番を入れ替える変異が 1905 全緑を通る。**
    /// **混在の囮（`"a=b c"`）が、順番を見ている唯一の見張り番。**
    /// 理由は [`fault_in_aggregation_key`] の doc の表。
    #[test]
    fn the_aggregation_key_check_finds_each_fault() {
        // 1. 鳴る側。**見えない文字は `\u` の綴りで書く**（CLAUDE.md）。
        for (spelling, expected) in [
            ("", KeyFault::Empty),
            // 空白（B-39 より前もここだけは止めていた）。
            ("a\u{20}b", KeyFault::NotGraphicAscii('\u{20}')),
            // **B-39 で広がった分。** 空白を見るだけの版は、ここを1つ残らず素通りする。
            ("a\u{09}b", KeyFault::NotGraphicAscii('\u{09}')),
            ("a\u{0A}b", KeyFault::NotGraphicAscii('\u{0A}')),
            ("a\u{0D}b", KeyFault::NotGraphicAscii('\u{0D}')),
            ("a\u{7F}b", KeyFault::NotGraphicAscii('\u{7F}')),
            // 非 ASCII（B-39 より前も止めていた）。
            ("構造化", KeyFault::NotGraphicAscii('構')),
            // **B-39 の本題。** 集計キーで実際に効く区切りは `=` と `\u{22}`。
            ("a=b", KeyFault::Separator('=')),
            ("\u{22}x\u{22}", KeyFault::Separator('\u{22}')),
            ("fallback\u{3D}unclosed", KeyFault::Separator('=')),
            // **⚠ 1つの綴りが2つの主張に触れる囮**（B-39 の是正1）。
            // **これが無いと、2つの主張の順番を入れ替える変異が全緑を通る。**
            ("a=b c", KeyFault::NotGraphicAscii('\u{20}')),
        ] {
            assert_eq!(
                fault_in_aggregation_key(spelling),
                Some(expected),
                "植えた綴りの外れ方が違う: {spelling:?}"
            );
        }

        // 2. 拒まない側。**「常に鳴る」判定はここで止まる。**
        //    実物のラベルは使わない（実物は使う側の検査が別に見ている）。
        for spelling in ["x", "a_b", "a-b", "a.b:1", "[a]", "+1", "a,b"] {
            assert_eq!(
                fault_in_aggregation_key(spelling),
                None,
                "正当な綴りを拒んでいる: {spelling:?}"
            );
        }
    }

    /// **判定が拒む綴りは、ログの欄の読み手が実際に読み違える**（作業単位 B-39）。
    ///
    /// 「危ない」と書くだけでは、CLAUDE.md が言う
    /// 「doc が、守られていない場所に『守った』と書く」の逆向きになる。
    /// **壊れるところを、`crate::testlog` の道具に実際に通して見せる。**
    ///
    /// # 植えた欄で測る理由
    ///
    /// 使う側の集計キーは**閉じた値の集合**（`parse::ParseOutcome` も
    /// `domain::drift` の6つの型も）なので、**本番の入口からは
    /// 「判定が効く状態」を作れない**（CLAUDE.md `13c-3a`）。
    /// `server::turn` / `server::drift` が出す欄の**形だけ**を写して植える。
    /// **`parse` も `domain` も本番コードは1行もログを出さない**
    /// ——ここで `tracing` を鳴らすのはテストである。
    ///
    /// # ⚠ 主張は必ず**行に切ってから**当てる
    ///
    /// `crate::testlog` の `has_field` の doc が
    /// 「主張の本体は必ず行に切ってから渡す」と書いている（24-2 / B-1-2）。
    /// **B-39 の是正前のここは、丸ごとの出力に3箇所当てていた** ——
    /// **このテストの主題そのものが「壊れたキーは1イベントを2行に割る」**なので、
    /// **いちばん行に切るべき場所で切っていなかった**（B-39 のレビュー指摘）。
    ///
    /// # 実測（B-39 / この機・`tracing-subscriber` の `fmt` 層）
    ///
    /// | 植えた綴り | 起きること |
    /// |---|---|
    /// | `a\u{09}b` | 欄が**空白で切れ**、読み手は `a` を**値の全部として**読む |
    /// | `a\u{0A}b` | **1つのイベントが2行に割れ**、`session_id` が別の行へ行く |
    /// | `a=b` | **誰も書いていないキー**（`outcome=a`）が在ることになる |
    /// | `\u{22}x\u{22}` | `Debug` が書いた文字列と**同じバイト列**になる |
    ///
    /// # ⚠ `a=b` は「読めなくなる」のではない
    ///
    /// **トークン一致の読み手は `a=b` を読み戻せる**（実測）。
    /// 壊れるのは**キーの側**で、`=` を含む値は
    /// 「キーが伸びた別の欄」（`crate::testlog` の `has_field` の doc の表 / E-2 の形）と
    /// **バイト単位で同じものになる。**
    #[test]
    fn a_key_the_check_rejects_is_misread_by_the_field_readers() {
        use crate::testlog::{
            captured_under_default_filter, has_field, has_key, line_containing, quoted_field,
        };

        /// 植えた綴りを**本番と同じ形の欄**に載せて、捕捉した出力を返す。
        fn planted(label: &str) -> String {
            captured_under_default_filter(|| {
                tracing::info!(
                    outcome = %label,
                    session_id = 42,
                    "PLANTED_FOR_B39"
                );
            })
        }

        // 0. 対照。**正当な綴りなら、下の4つの読み違えは1つも起きない。**
        let sound = planted("structured");
        assert_eq!(sound.lines().count(), 1, "{sound:?}");
        let sound_line = line_containing(&sound, "PLANTED_FOR_B39");
        assert!(has_field(sound_line, "outcome=structured"), "{sound:?}");
        assert!(!has_field(sound_line, "outcome=a"), "{sound:?}");
        assert!(!has_key(sound_line, "outcome=a"), "{sound:?}");
        assert_eq!(quoted_field(sound_line, "outcome"), None, "{sound:?}");
        assert!(has_field(sound_line, "session_id=42"), "{sound:?}");

        // 1. 空白（TAB）。**値が切れ、`a` が値の全部として読める。**
        let tab = planted("a\u{09}b");
        assert!(fault_in_aggregation_key("a\u{09}b").is_some());
        assert_eq!(tab.lines().count(), 1, "TAB は行を割らない: {tab:?}");
        let tab_line = line_containing(&tab, "PLANTED_FOR_B39");
        assert!(
            !has_field(tab_line, "outcome=a\u{09}b"),
            "植えた値を丸ごとは読み戻せないこと: {tab:?}"
        );
        assert!(
            has_field(tab_line, "outcome=a"),
            "切れた前半が値の全部として読めてしまうこと: {tab:?}"
        );

        // 2. 改行。**1つのイベントが2行に割れ、帰属が壊れる。**
        let newline = planted("a\u{0A}b");
        assert!(fault_in_aggregation_key("a\u{0A}b").is_some());
        assert_eq!(newline.lines().count(), 2, "{newline:?}");
        let head = line_containing(&newline, "PLANTED_FOR_B39");
        assert!(has_field(head, "outcome=a"), "{newline:?}");
        assert!(
            !has_field(head, "session_id=42"),
            "同じ行から欄が落ちること（帰属が壊れる形）: {newline:?}"
        );

        // 3. `=`。**誰も書いていないキーが在ることになる。**
        let equals = planted("a=b");
        assert!(fault_in_aggregation_key("a=b").is_some());
        assert_eq!(equals.lines().count(), 1, "{equals:?}");
        let equals_line = line_containing(&equals, "PLANTED_FOR_B39");
        assert!(
            has_field(equals_line, "outcome=a=b"),
            "トークン一致では読み戻せる（ここは壊れない）: {equals:?}"
        );
        assert!(
            has_key(equals_line, "outcome=a"),
            "誰も書いていないキーが在ることになる: {equals:?}"
        );

        // 4. `\u{22}`。**`Debug` が書いた文字列と同じバイト列になる。**
        let quoted = planted("\u{22}x\u{22}");
        assert!(fault_in_aggregation_key("\u{22}x\u{22}").is_some());
        assert_eq!(quoted.lines().count(), 1, "{quoted:?}");
        let quoted_line = line_containing(&quoted, "PLANTED_FOR_B39");
        assert_eq!(
            quoted_field(quoted_line, "outcome"),
            Some("x"),
            "`Display` で書いた値が `Debug` の文字列として読めてしまうこと: {quoted:?}"
        );
    }
}
