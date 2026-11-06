//! **性格ステータスの変容**（[ADR-0024](../../docs/adr/0024-stat-drift.md) / 作業単位 16a）の
//! ドメイン型と**適用規則**。
//!
//! **I/O を持たない純粋関数のみ**を置く（CLAUDE.md 3節）。
//!
//! # なぜ `parse` でも `store` でもなく `domain` なのか
//!
//! ここに置いてあるのは**語彙**（向き・破棄の理由）と**規則**（許可リスト・対象外・飽和）で、
//! そのどれもが**文字列の分解にも SQL にも依存しない。**
//! `parse::drift` は「サブLLM の生出力を [`DriftProposal`] にする」だけ、
//! `prompt::drift` は「依頼を組む」だけで、**両者は互いを知らない**
//! （`extraction` の対と同じ形。共有する定義はこの `domain` にある）。
//!
//! # 規則（ADR-0024）
//!
//! | 節 | 規則 | ここでの実装 |
//! |---|---|---|
//! | §1 | 軸は**保存済みの軸名との完全一致**だけ受ける | [`apply_drift`] の `find` |
//! | §3 | 1ターン**最大1軸・±1** | [`DriftDelta`] が2値しか持たない |
//! | §5 | 端では**飽和させ、飽和を数える**（クランプしない） | [`DriftOutcome::Saturated`] |
//! | §6 | 崩れたら破棄。**理由ごとに数える** | [`DriftDiscardReason`] |
//!
//! > **§4（`appears_in_prompt()` が真の軸だけ動かせる）は外れた**
//! > （[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §決定4 / §決定6）。
//! > **値 0 の軸も変容の依頼に載せる**ようになったので、
//! > 「モデルが読んでいない軸を動かさない」は**載せることで満たされる。**
//! > 門を残すと**ゼロから育てるキャラクターが原理的に動かせない。**
//!
//! # **下端は `0`**（ADR-0027 §決定5 / 作業単位 16e-b）
//!
//! [`DRIFT_FLOOR`] は `0`。**`0` から `-1` が飽和**になる（負の値は作らない）。
//!
//! **`1` だった頃の見張り番 `drift_never_produces_a_zero_value` は役目を終えた。**
//! いまは2本に割れている——**`0` へ落ちられること**
//! （`drift_can_now_reach_zero_at_the_bottom`）と、
//! **`0` から戻れること**（`a_zero_valued_axis_can_now_be_raised_from_the_floor_up`）。
//! **片方だけでは、`0` が片道になっていることに気付けない。**
//!
//! > **⚠ 開けられたのは §決定2 の減衰が一緒に入ったからである**（ADR-0027 §決定5）。
//! > `0` は片道ではなくなった（§決定4 で値 0 の軸も依頼に載る）ので、
//! > 「変容が軸を本文プロンプトから消す」ことを塞ぐ必要が無くなった。

use super::stat::{StatAlloc, StatAxis, StatValue};

/// **変容の下端。`0`**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md)
/// §決定5 / 作業単位 16e-b）。**`0` から `-1` は飽和**で、負の値は作らない。
///
/// # なぜ `1` から `0` へ開けられたのか
///
/// **`0` が片道でなくなったから。** §決定4 で**値 0 の軸も変容の依頼に載る**ようになり、
/// §決定6 で「1本でも軸が定義されていれば依頼を作る」に門が変わった。
/// つまり `0` へ落ちた軸も**次のターンから採点の対象**で、
/// 上げの提案が来れば戻ってくる。
///
/// `1` を下端にしていた理由は「**変容だけで軸を本文プロンプトから消せる**経路を作らない」
/// （`StatAlloc::appears_in_prompt` が偽になる）ことだった。
/// **消えても戻れるなら、塞ぐ理由が無い。**
///
/// # ⚠ **減衰（§決定2）と一緒でなければ開けられなかった**
///
/// 16e-a の時点では、`single` × `on` で**モデル自身の `-1` がそのまま適用されていた**
/// （実測で m13 は 4/72、腕F は 12/36 の下げ）。**そこで開けると、
/// 1回の提案だけで軸が本文プロンプトから消える。**
/// 16e-b で「累積が費用に達したら1段」（§決定7）に変わり、
/// **1回の提案では届かない**——低い値ほど下げの費用が高い（§9）ので、
/// `0` の手前は特に硬い。
///
/// # 見張り番は2本に割れている
///
/// * **`0` へ落ちられること** —— `drift_can_now_reach_zero_at_the_bottom`
/// * **`0` から戻れること** —— [`a_zero_valued_axis_can_now_be_raised_from_the_floor_up`]
///
/// **1本にまとめない。** まとめると、`0` が片道（落ちたきり／上がったきり）に
/// なっている状態が緑を通る。
pub const DRIFT_FLOOR: u32 = 0;

/// **「動いていない」の綴り**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12）。
///
/// [`DriftDelta::sign`] と同じ役割で、**綴りの正はここ**。
/// [`DriftForm::All`] の依頼が手本として示す形（`- 項目名 0`）と、
/// `parse::drift` が受ける形が、この1つの定数を基準にする。
///
/// **`Option<DriftDelta>` の `None` 側の綴り**なので `DriftDelta` の中には置けない
/// ——あの型は2値しか持たない（幅を型で ±1 に固定している）。
///
/// 見張り番は `prompt::drift` の
/// `the_examples_in_the_all_axis_request_parse_back_into_a_scorecard`。
pub const NO_CHANGE_SIGN: &str = "0";

/// 変容の向き。**幅は型で ±1 に固定してある**（ADR-0024 §3）。
///
/// 数値（`i32` など）を持たせない。持たせた瞬間に `+2` を**表現できてしまい**、
/// 「どのくらい強い出来事か」の判断を小型モデルに委ねる道が開く
/// （ADR-0024 が「幅を可変にする」を却下した理由そのもの）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftDelta {
    /// 1段上げる。
    Up,
    /// 1段下げる。
    Down,
}

impl DriftDelta {
    /// すべての向き。**集計と網羅テストの走査に使う。**
    pub const ALL: [Self; 2] = [Self::Up, Self::Down];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない
    /// （`parse::extraction::DiscardReason::label` と同じ扱い）。
    ///
    /// ADR-0024 の「危険 2. 一方向へ流れる」は、この2つの比で見る。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    /// **依頼プロンプトが手本として示す綴り**（`+1` / `-1`）。
    ///
    /// `Subject::extraction_word` と同じ役割——**綴りの正はここ**で、
    /// `prompt::drift` の手本も `parse::drift` が受ける形もこれを基準にする。
    /// 直書きが2箇所に分かれると、片方だけ動いたときに
    /// **モデルが手本どおりに書くほど破棄される。**
    /// 見張り番は `prompt::drift` の
    /// `the_examples_in_the_request_parse_back_into_a_suggestion`。
    pub const fn sign(self) -> &'static str {
        match self {
            Self::Up => "+1",
            Self::Down => "-1",
        }
    }
}

/// 変容の提案。**軸名は [`StatAxis::parse`] を通っているだけ**で、
/// **保存済みの軸との照合はまだ済んでいない**（ADR-0024 §1 の許可リストは [`apply_drift`]）。
///
/// **LLM の出力は信頼できない入力**（CLAUDE.md 絶対制約）。この型は
/// 「軸名として正当な形をしている」までしか主張しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftSuggestion {
    /// 提案された軸名（**照合前**）。
    pub axis: StatAxis,
    /// 向き。
    pub delta: DriftDelta,
}

/// 捨てた理由。**1つの数に潰さない**（ADR-0024 §6）。
///
/// 潰すと「モデルが形式を守れない」「列挙する癖がある」「知らない軸を作る」
/// 「語らないと決めた軸を動かそうとする」が区別できず、
/// **プロンプトを直すべきかモデルを替えるべきかの判断材料にならない。**
///
/// **飽和はここに無い**（提案は正しく、適用できないだけ）。
/// [`DriftOutcome::Saturated`] として別に数える。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftDiscardReason {
    /// 1行の形に合わない（ADR-0024 §6「形式が読めない」）。
    ///
    /// 幅が `±1` でない（`+2`）ものも、軸名として読めないものもここに入る。
    /// **`parse::drift` の doc に受ける集合と捨てる集合が書いてある。**
    Malformed,
    /// 提案が2行以上あった（ADR-0024 §3）。**先頭だけを採らない。**
    ///
    /// **1軸だけを尋ねる形（[`DriftForm::Single`]）のときだけ出る。**
    /// 全軸採点（[`DriftForm::All`]）では N 行が正常なので、
    /// 行数の食い違いは [`Self::MissingAxis`] / [`Self::DuplicateAxis`] /
    /// [`Self::UnknownAxis`] のどれかとして数える。
    MultipleLines,
    /// 保存済みの軸名と完全一致しなかった（ADR-0024 §1 の許可リスト）。
    ///
    /// **モデルが軸を作った。** 打ち手は「並んでいる名前だけを書け」の指示の側。
    UnknownAxis,
    /// **全軸採点で、依頼に載せた軸の行が無かった**
    /// （[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12）。
    ///
    /// # **`0` と採点されたのとは別物である**
    ///
    /// ADR-0027 §12 が名指ししている区別そのもの——
    /// **行が無いのは形式の崩れであって 0 ではない。**
    /// 混ぜると、**モデルが列挙を途中でやめた回が「動いていない」として集計に入り**、
    /// §決定2（選ばれなかった軸が薄れる）の入力が静かに水増しされる。
    ///
    /// 打ち手は「**全部書け**」の指示の側である。
    ///
    /// # ⚠ **「応答が途中で切れた」はもうここに来ない**（作業単位 B-1）
    ///
    /// 上限で打ち切られた回は [`Self::Truncated`] へ分けた
    /// ——**打ち手が「上限を上げる」で正反対**だから。
    /// **ただし上流が停止理由を申告しなかった回**は、いまもここへ落ちうる
    /// （その状態は `server::drift` が別に `warn` で残す）。
    MissingAxis,
    /// **全軸採点で、同じ軸の行が2つ以上あった。**
    ///
    /// **`MultipleLines` と混ぜない。** あちらは「1行のはずが複数行来た」で、
    /// こちらは「N 行のうち同じ名前が重なった」。
    /// 打ち手は「同じ項目を2回書かない」の指示の側。
    DuplicateAxis,
    /// **応答がトークン上限で打ち切られたので、丸ごと捨てた**
    /// （[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §打ち切り /
    /// 作業単位 B-1）。
    ///
    /// # **崩れているとは限らない。そこが危ない**
    ///
    /// 1軸を尋ねる形は**2行以上を丸ごと捨てる**（[`Self::MultipleLines`]）。
    /// **モデルが2行書いた回で、打ち切りが2行目を丸ごと消すと、
    /// 受け取る側には「1行の正当な提案」しか届かない**——
    /// **捨てるはずの回がそのまま値を動かし、ログは正常な `applied` に見える。**
    ///
    /// **何行あったのかが分からない以上、残った1行を信じる根拠が無い。**
    /// だから**行を選ばず丸ごと捨てる**（抽出の「最後の1行だけ捨てる」とは
    /// **意図的に違う判断**。理由は ADR-0027）。
    ///
    /// # [`Self::Malformed`] にも [`Self::MissingAxis`] にも混ぜない
    ///
    /// **打ち手が正反対。** 崩れなら**プロンプトを直す**、
    /// 行が足りないなら**「全部書け」の指示**、打ち切りなら**上限を上げる**側である。
    /// 混ぜると、**上限が現実に合っていないというシグナルが読めなくなる**
    /// （`parse::extraction::DiscardReason::Truncated` と同じ理屈）。
    Truncated,
}

impl DriftDiscardReason {
    /// すべての理由。**集計の走査に使う。**
    pub const ALL: [Self; 6] = [
        Self::Malformed,
        Self::MultipleLines,
        Self::UnknownAxis,
        Self::MissingAxis,
        Self::DuplicateAxis,
        Self::Truncated,
    ];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::MultipleLines => "multiple_lines",
            Self::UnknownAxis => "unknown_axis",
            Self::MissingAxis => "missing_axis",
            Self::DuplicateAxis => "duplicate_axis",
            Self::Truncated => "truncated",
        }
    }

    /// **許可リストで落ちた理由か**（= [`apply_drift`] が出しうる理由か）。
    ///
    /// 残りは分解の段階（`parse::drift`）でしか出ない。
    /// 見張り番は `apply_drift_only_reports_the_allowlist_reasons`。
    ///
    /// # **`NotInPrompt` は消えた**（ADR-0027 §決定4 / §決定6）
    ///
    /// 値 0 の軸も変容の依頼に載せるようになったので、
    /// 「保存済みだがプロンプトに載っていない軸」という状態が**存在しなくなった。**
    /// 到達不能な理由を残すと、**ログに常に 0 の欄が並ぶだけ**でなく、
    /// 「§4 の門がまだ在る」と読める（CLAUDE.md「死んだ検査」）。
    pub const fn from_allowlist(self) -> bool {
        matches!(self, Self::UnknownAxis)
    }
}

/// **分解の結果**（`parse::drift` が返す）。
///
/// 「提案が1つ」「提案なし」「壊れ」の3つしかない（ADR-0024 §3 / §6）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftProposal {
    /// 提案が1つ。
    Suggested(DriftSuggestion),
    /// 提案なし（「なし」と書かれた、または何も書かれなかった）。**正常な答え。**
    Nothing,
    /// 捨てた。
    Discarded(DriftDiscardReason),
}

/// **依頼の形**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12）。**2値。**
///
/// | 値 | 尋ね方 | 応答 |
/// |---|---|---|
/// | [`Self::Single`] | 動いた項目を**1つだけ** | 1行、または「なし」 |
/// | [`Self::All`]（**既定**） | 全項目を**1つずつ採点** | **項目の数と同じ行数**（`0` を書ける） |
///
/// # なぜ切り替えられる形にするのか
///
/// **同じ台本で両方を測るため**（ADR-0027 §12 の「決めること」）。
/// docs/05-development.md「属性を実験の切り替えスイッチとして使う」と同じ作法。
///
/// # ⚠ **既定は 2026-08-25 に [`Self::All`] へ移した**（作業単位 19）
///
/// **測って決まった**（ADR-0027 §決定8 / 16d-2・16i・16j・16l・16n）。
/// [`Self::Single`] は**上げ偏りが 94% で、値が分化しない**
/// ——全軸が同じ向きへ流れて、最後は上端に並ぶ。
/// [`Self::All`] は**`0` を書けるので票が立たない回がある**ぶん、
/// 上げと下げの両方が出て、着地点がばらける（16j の5条件を満たす唯一の形）。
///
/// **`Single` は「測定と比べるために残してある形」**であって、
/// 明示したときだけ効く。
///
/// # なぜ `domain` にあるのか（`server::config` ではなく）
///
/// [`StatDriftMode`] と同じ理由——この型は**依頼と分解の両方が従う取り決め**であって、
/// 環境変数の綴りの話ではない。綴り（`single` / `all`）を読むのは `server::config`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftForm {
    /// 動いた項目を1つだけ尋ねる（ADR-0024 §3）。**16j で既定から降りた。**
    Single,
    /// **既定**（作業単位 19）。**全軸採点**（ADR-0027 §12 → §決定8 / 16i・16j）。
    All,
}

impl DriftForm {
    /// すべての値。**集計と網羅テストの走査に使う。**
    pub const ALL: [Self; 2] = [Self::Single, Self::All];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    ///
    /// `server::config` が受け付ける綴りと同じ語にしてある
    /// ——**ログに出た語をそのまま環境変数へ書き戻せる**（[`StatDriftMode::label`] と同じ作法）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::All => "all",
        }
    }
}

/// **依頼に足す「書き方の注記」**
/// （[ADR-0029](../../docs/adr/0029-language-as-a-matched-bundle.md) §訂正 / 作業単位 16m）。**3値。**
///
/// | 値 | 依頼に足す文 |
/// |---|---|
/// | [`Self::None`]（既定 / 腕 X） | **何も足さない。** m20 / m21 で測った依頼そのもの |
/// | [`Self::Copy`]（腕 Y） | **複写の指示**——軸名は上に書かれている字をそのまま写す |
/// | [`Self::Language`]（腕 Z） | **言語の指示**——日本語（常用漢字）で書く |
///
/// # ⚠ **これは機能ではなく、測定の器具である**
///
/// ADR-0029 §決定5 は**保留**の状態にある。「言語の指示は効かない」という推論を
/// 支える測定が1つも無かったので、**3本の腕を実機で比べて決める。**
/// **測って決まったら、この型は「片方に固定して消える」か
/// 「正式な決定として残る」かのどちらか**になる。
///
/// # 文面は3つに閉じている（運用者の自由文にしない）
///
/// ADR-0029 §決定4 と却下欄のとおり、**依頼の文面は測って決めた結果物**である。
/// 自由文を入れられる形にすると、**測っていない文面に黙って置き換わる経路**ができる。
/// 綴り（`none` / `copy` / `language`）を読むのは `server::config`、
/// **文面を持つのは `prompt::drift`** で、この型は**束を1つ選ぶ値**でしかない。
///
/// # なぜ `domain` にあるのか
///
/// [`DriftForm`] と同じ理由——依頼の組立が従う取り決めであって、
/// 環境変数の綴りの話ではない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftNote {
    /// **既定（腕 X）。** 注記を足さない。**依頼は1バイトも変わらない。**
    None,
    /// **複写の指示（腕 Y）。** 「そのまま写す」——言語ではなく作業を指す。
    Copy,
    /// **言語の指示（腕 Z）。** ADR-0029 が問うているのはこちら。
    Language,
}

impl DriftNote {
    /// すべての値。**集計と網羅テストの走査に使う。**
    pub const ALL: [Self; 3] = [Self::None, Self::Copy, Self::Language];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    ///
    /// `server::config` が受け付ける綴りと同じ語にしてある
    /// ——**ログに出た語をそのまま `PERSONACORE_DRIFT_NOTE` へ書き戻せる。**
    /// **3本の腕を並べて読むための語**なので、別名を用意しない。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Copy => "copy",
            Self::Language => "language",
        }
    }
}

/// **全軸採点の1軸ぶんの結果**（ADR-0027 §12）。
///
/// # **「0 と採点された」と「行が無かった」は、この型では区別しない**
///
/// **区別は [`DriftScoring`] の側で付いている。** 行が1本でも欠けたら
/// 採点表そのものが [`DriftDiscardReason::MissingAxis`] として捨てられるので、
/// **[`DriftScoring::Scored`] に入っている `None` は必ず「モデルが 0 と書いた」である。**
/// ここに「無かった」を表す第3の状態を作ると、
/// **その値が集計へ流れる経路**ができてしまう（ADR-0027 §12 が名指しする混同）。
///
/// # フィールドは private
///
/// 軸名は**依頼に載せた保存済みの綴り**（照合済み）で、
/// モデルが書いた文字列ではない。公開すると、
/// **照合していない軸名を採点結果として持ち回れる**（[`DriftVote`] と同じ判断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftScore {
    axis: StatAxis,
    delta: Option<DriftDelta>,
}

impl DriftScore {
    /// **分解の内部経路。** `crate` の外からは作れない。
    ///
    /// `parse::drift` が**依頼に載せた軸の綴り**と、
    /// その軸に対してモデルが書いた向き（`None` = `0`）から組む。
    pub(crate) const fn new(axis: StatAxis, delta: Option<DriftDelta>) -> Self {
        Self { axis, delta }
    }

    /// 採点された軸。**保存済みの綴り。**
    #[must_use]
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 向き。**`None` は「モデルが `0` と書いた」**（「行が無かった」ではない）。
    #[must_use]
    pub const fn delta(&self) -> Option<DriftDelta> {
        self.delta
    }

    /// **`0` でなかった採点だけが票になる**（ADR-0026 §決定3 / ADR-0027 §決定1）。
    ///
    /// # `0` を票にしない理由は、重みの数え方そのものである
    ///
    /// ADR-0027 §12 の危険3——**全軸採点では毎ターン全軸が採点される**ので、
    /// `0` まで票にすると「その軸が何回票になったか」が全軸で同じになり、
    /// **希少度の重みが一律に戻る。** 数えるのは **`0` でなかった回数**。
    ///
    /// [`DriftOutcome::vote`] と同じく、**軸名は保存済みの綴り**
    /// （[`DriftScore`] は依頼に載せた綴りしか持たない）。
    #[must_use]
    pub fn vote(&self) -> Option<DriftVote> {
        self.delta.map(|delta| DriftVote {
            axis: self.axis.clone(),
            delta,
        })
    }
}

/// **全軸採点の分解の結果**（ADR-0027 §12）。[`DriftProposal`] の全軸版。
///
/// # 崩れたら**丸ごと**捨てる（`parse::drift` と同じ作法）
///
/// 読めた行だけを拾うと、**読めなかった軸が「0 と採点された」ことになる。**
/// それは ADR-0027 §12 が「必ず区別すること」と書いた当のものなので、
/// **部分的に成功した採点表という状態を作らない。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftScoring {
    /// 依頼に載せた**全軸ぶん**の採点。**並びは依頼に載せた順。**
    Scored(Vec<DriftScore>),
    /// 捨てた。
    Discarded(DriftDiscardReason),
}

/// **採点表が読めた回の集計キー**（[`DriftScoring::label`]）。
///
/// **[`DriftOutcome::label`] のどれとも重ならない綴り**にしてある
/// ——同じログの**結末の欄**に、依頼の形によってどちらかが載るため
/// （欄の綴りをここに書かない。`server::drift` の
/// `the_drift_fields_never_collide_with_another_module` が走査で落とす）。
///
/// # なぜ `applied` を使い回さないのか
///
/// 全軸採点には「適用された1軸」が無い。**読めた採点表そのもの**が結果である。
/// `applied` を流用すると、**`single` の「提案が通った」と区別できなくなり**、
/// 依頼の形を変えた測定どうしを並べられない。
pub const DRIFT_SCORED_LABEL: &str = "scored";

impl DriftScoring {
    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない
    /// （[`DriftOutcome::label`] と同じ扱い）。
    ///
    /// 破棄の場合は [`DriftDiscardReason::label`] をそのまま返す——
    /// **破棄の理由と結末が1つの語彙**になる。
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Scored(_) => DRIFT_SCORED_LABEL,
            Self::Discarded(reason) => reason.label(),
        }
    }
}

/// **適用できた変容。** どの軸を、いくつからいくつへ動かすか。
///
/// `before` を持つのは ADR-0024 §7 のため——`store` の更新は
/// `WHERE character_id = ? AND axis = ? AND value = ?` で
/// **現在値を条件に含める**ので、読んだ時点の値が要る。
///
/// # **フィールドは private。この値は SQL へ渡る**
///
/// ADR-0024 §1 は「SQL へは**照合を通った `StatAxis` だけ**がプレースホルダで渡る」と
/// 決めている。フィールドが公開されていると、
/// `DriftChange { axis: モデルが書いたままの軸名, .. }` を呼び出し側が普通に組み立てられ、
/// **許可リストを迂回した値が `store` に届く経路**ができる。
///
/// # ⚠ **`ValidTag` と同じ強さだと読まないこと**
///
/// 守れているのは「**このモジュールの外からは作れない**」までである。
/// [`apply_drift`] はモジュール内にあるので、**同じモジュール内（テストを含む）では
/// 構造体リテラルで組める。** ADR-0006 の `ValidTag`（辞書引き以外の構築手段を
/// 型から消す）とは強さが違う。**「表現できなくした」とは書かない**
/// ——CLAUDE.md「型で表現できなくしたと書く前に、表現できてしまう実装を実際に書いてみる」。
///
/// # [`DriftSuggestion`] を同じ扱いにしない理由
///
/// あちらは `parse::drift`（**別のモジュール**）が作るので、private にすると
/// 公開のコンストラクタが要る。そして**あの型は何も主張していない**
/// ——「軸名として正当な形をしている」までで、照合前の untrusted な値である。
/// **構築を絞っても不変条件が1つも増えない**ので、鍵を掛ける意味が無い。
/// 実際に守るのは [`apply_drift`] の許可リストで、そこを通った値だけがこの型になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftChange {
    /// 動かす軸。**保存済みの綴り**（提案側の綴りではない）。
    axis: StatAxis,
    /// 動かす前の値。**`store` の更新条件に使う。**
    before: StatValue,
    /// 動かした後の値。**尺度による検証を通っている**（`StatScale::value`）。
    after: StatValue,
}

impl DriftChange {
    /// **帳簿（`domain::drift_ledger`）が1段以上動かしたときの構築経路。**
    ///
    /// `domain` の中からしか呼べない（`apply_drift` と同じ強さ）。
    /// 渡す軸名は**帳簿が持っている保存済みの綴り**で、
    /// `before` / `after` は**同じ尺度**の値である（帳簿は `StatValue` を持ち回る）。
    ///
    /// # ここだけ「1段」ではない
    ///
    /// [`apply_drift`] は必ず1段だが、帳簿は**費用が安い領域で2段以上動く**
    /// （ADR-0027 §9）。`before` は**そのターンの開始時の値**で、
    /// `store` の `WHERE value = ?` に渡るのはこちらである。
    pub(in crate::domain) const fn stepped(
        axis: StatAxis,
        before: StatValue,
        after: StatValue,
    ) -> Self {
        Self {
            axis,
            before,
            after,
        }
    }

    /// 動かす軸。**許可リストの照合を通った、保存済みの綴り**（ADR-0024 §1）。
    ///
    /// `store` の `UPDATE ... WHERE axis = ?` にプレースホルダで渡る値。
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 動かす前の値。**`store` の更新条件**（`AND value = ?` / ADR-0024 §7）に使う。
    pub const fn before(&self) -> StatValue {
        self.before
    }

    /// 動かした後の値。**尺度による検証を通っている**（`StatScale::value`）。
    pub const fn after(&self) -> StatValue {
        self.after
    }

    /// 動いた向き。**保持せずに前後の値から導く。**
    ///
    /// `DriftDelta` を持たせると「向きと値が食い違った `DriftChange`」を
    /// 表現できてしまう（**写しは片方だけ古びる** / CLAUDE.md）。
    ///
    /// **`before == after` の値は、このモジュールの外からは作れない**
    /// （フィールドが private で [`apply_drift`] が唯一の構築経路）。
    /// **モジュールの中では作れる**ので、`apply_drift` が必ず1段動かすことを
    /// 言い切っているのは型ではなく見張り番
    /// `an_applied_change_always_moves_by_exactly_one_step` の方である。
    pub const fn delta(&self) -> DriftDelta {
        if self.after.get() > self.before.get() {
            DriftDelta::Up
        } else {
            DriftDelta::Down
        }
    }
}

/// **適用の結果。** ADR-0024 が数えると決めた種別がそのまま並ぶ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftOutcome {
    /// 値が動く。
    Applied(DriftChange),
    /// 提案が無かった。**正常な答え**（ADR-0024 の「危険 3. 提案が出ない」で見る側）。
    Nothing,
    /// 端で動かせない。**破棄ではない**（ADR-0024 §5）。
    ///
    /// クランプして「動いた」と記録すると、
    /// 「毎ターン上げようとしているが上端で止まっている」と「提案が出ていない」が
    /// **区別できなくなる。**
    Saturated {
        /// 動かせなかった軸（**保存済みの綴り**）。
        axis: StatAxis,
        /// 提案された向き。
        delta: DriftDelta,
    },
    /// 捨てた。
    Discarded(DriftDiscardReason),
}

impl DriftOutcome {
    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    ///
    /// 破棄の場合は [`DriftDiscardReason::label`] をそのまま返す——
    /// ログの1列に落とすときに、**破棄の理由と結末が1つの語彙**になる。
    /// 語が重ならないことは `every_outcome_label_is_distinct` が見張る。
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Applied(_) => "applied",
            Self::Nothing => "nothing",
            Self::Saturated { .. } => "saturated",
            Self::Discarded(reason) => reason.label(),
        }
    }

    /// **票になる結末だけが `Some`**（ADR-0026 §決定3）。
    ///
    /// | 結末 | 票 | なぜ |
    /// |---|---|---|
    /// | [`Self::Applied`] | **なる** | 許可リストと §4 の門を通った提案そのもの |
    /// | [`Self::Saturated`] | **なる** | 提案は正しく、適用できないだけ（[`DriftVote`] の doc） |
    /// | [`Self::Nothing`] | ならない | 提案が無い |
    /// | [`Self::Discarded`] | ならない | **門を通っていない。** 破棄はログのまま |
    ///
    /// # `match` で書いてある理由
    ///
    /// **結末が増えたらコンパイルが通らない。** `if let` で `Applied` だけを拾う形にすると、
    /// 新しい結末が黙って「票にならない」側へ落ちる
    /// （CLAUDE.md「`match` なので値が増えたらコンパイルが通らない」）。
    ///
    /// # 向きは**提案どおり**（適用できたかどうかとは別）
    ///
    /// [`Self::Applied`] の向きは [`DriftChange::delta`] が前後の値から導く。
    /// 保持した向きを別に持たないのは、**写しが片方だけ古びる**のを避けるため。
    #[must_use]
    pub fn vote(&self) -> Option<DriftVote> {
        match self {
            Self::Applied(change) => Some(DriftVote {
                axis: change.axis().clone(),
                delta: change.delta(),
            }),
            Self::Saturated { axis, delta } => Some(DriftVote {
                axis: axis.clone(),
                delta: *delta,
            }),
            Self::Nothing | Self::Discarded(_) => None,
        }
    }
}

/// **提案を保存済みの性格ステータスに突き合わせる**（ADR-0024 §1 / §4 / §5）。**純粋関数。**
///
/// 1. 提案が無い・すでに壊れているなら、そのまま結末に写す。
/// 2. **保存済みの軸名との完全一致**で探す。無ければ [`DriftDiscardReason::UnknownAxis`]。
/// 3. 端なら [`DriftOutcome::Saturated`]。上端は尺度、下端は [`DRIFT_FLOOR`]。
/// 4. それ以外は [`DriftOutcome::Applied`]。
///
/// # **§4 の門（`appears_in_prompt()`）は外れた**（ADR-0027 §決定4 / §決定6）
///
/// 値 0 の軸も**変容の依頼には載せる**ようになったので、
/// 「モデルが読んでいない軸を、モデルの判断で動かさない」は
/// **載せることで満たされる**（ADR-0027 §決定4）。
/// 門を残すと、**全部ゼロから育てるキャラクターが原理的に動かせない**
/// ——上げの提案が毎回 `not_in_prompt` で捨てられる。
///
/// **本文プロンプトの側は何も変わっていない**（CLAUDE.md 絶対制約）。
/// 落とすのは `prompt::sanitize_character` の `filter` で、
/// 変えたのは**変容の依頼がその述語を見るかどうか**だけである。
///
/// # 尺度は**その軸が覚えているもの**を使う
///
/// [`StatValue`] は検証に使った尺度を覚えている（ADR-0015）。ここで別途
/// 尺度を受け取ると、`before` と `after` が**別の尺度の値**になる形を作れてしまう。
/// 「その尺度がそのキャラクターのものか」は型では守れない（`StatValue` の doc）ので、
/// **せめて前後で同じものを使う。**
pub fn apply_drift(allocs: &[StatAlloc], proposal: DriftProposal) -> DriftOutcome {
    let suggestion = match proposal {
        DriftProposal::Suggested(suggestion) => suggestion,
        DriftProposal::Nothing => return DriftOutcome::Nothing,
        DriftProposal::Discarded(reason) => return DriftOutcome::Discarded(reason),
    };

    // **完全一致だけ**（ADR-0024 §1）。前方一致にすると、モデルが軸名の一部を
    // 書いただけで別の軸が動く。
    let Some(alloc) = allocs
        .iter()
        .find(|candidate| candidate.axis == suggestion.axis)
    else {
        return DriftOutcome::Discarded(DriftDiscardReason::UnknownAxis);
    };

    // **軸名は保存済みの綴りを持ち回る**（照合を通った型だけを先へ渡す）。
    //
    // **これは今のところ「条件付きの等価変異」の位置**（CLAUDE.md 第5の分類）。
    // `suggestion.axis` に差し替えても **1221 passed / 0 failed** で通る
    // （実測 / 2026-08-23 / 変異19）——`StatAxis` の等価は正規化後の文字列の
    // 完全一致なので、上の門を抜けた時点で**両者はバイト列として同一**である。
    //
    // **無条件の等価ではない。** 寄りかかっているのは「照合が完全一致であること」
    // ただ1点で、前方一致に緩める変異（変異4）を入れると**両者は別物になる。**
    // その変異は `an_axis_that_is_not_stored_is_discarded` が赤にするので、
    // **等価性を支えている不変条件の側に見張り番が立っている。**
    // 保存済みの綴りを書いておくのは、その不変条件が緩んだ日に
    // **安全な側へ倒れる**ようにするため。
    let saturated = || DriftOutcome::Saturated {
        axis: alloc.axis.clone(),
        delta: suggestion.delta,
    };

    let scale = alloc.value.scale();
    let before = alloc.value.get();
    let raw = match suggestion.delta {
        // **クランプしない**（ADR-0024 §5）。
        DriftDelta::Up if before >= scale.get() => return saturated(),
        // **`== DRIFT_FLOOR` に縮めない。** いまは下端が `u32` の最小値なので
        // clippy の言うとおり同値だが、**下端を上げた日に「下端より下」が素通りする。**
        // `#[expect]` にしてあるので、**下端が動いて lint が鳴らなくなったら
        // この属性の側が警告になる**（気付ける形で残す）。
        #[expect(
            clippy::absurd_extreme_comparisons,
            reason = "下端が u32 の最小値でなくなった日に、この属性が警告になって気付ける"
        )]
        DriftDelta::Down if before <= DRIFT_FLOOR => return saturated(),
        DriftDelta::Up => before.saturating_add(1),
        DriftDelta::Down => before.saturating_sub(1),
    };

    // **尺度を通さずに値を作らない**（`StatScale::value` が唯一のコンストラクタ）。
    // 上の門を通っている以上ここは必ず `Ok` だが、`expect` を置かずに
    // **安全側（動かさない）へ倒す**（CLAUDE.md 絶対制約）。
    let Ok(after) = scale.value(raw) else {
        return saturated();
    };

    DriftOutcome::Applied(DriftChange {
        axis: alloc.axis.clone(),
        before: alloc.value,
        after,
    })
}

/// **1ターンぶんの票**（[ADR-0026](../../docs/adr/0026-drift-by-accumulation.md) §決定3）。
///
/// 「どの軸を、どちらへ動かそうとしたか」だけを持つ。**値は持たない**——
/// 票は蓄積されて後から効くので、記録した時点の値は
/// **適用する時点では既に古い**（ADR-0026 §決定2 の「溜めて消費する」）。
///
/// # **票になるのは「門を通った提案」だけ**（[`DriftOutcome::vote`]）
///
/// 軸名は**保存済みの綴り**（許可リストと §4 の門を通っている）で、
/// SQL へはプレースホルダで渡る。**分解に失敗した回は票にならない。**
///
/// # 飽和も票になる
///
/// 上端で `+1` を出し続けている状態は、票の側から見れば
/// **「上げようとし続けている」という情報**である。
/// 記録の時点で落とすと、蓄積（16e）はその傾向を二度と見られない
/// ——**飽和の判定は適用の時点で行う。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftVote {
    /// 票の入った軸。**保存済みの綴り。**
    axis: StatAxis,
    /// 向き。
    delta: DriftDelta,
}

impl DriftVote {
    /// 票の入った軸。**許可リストの照合を通った、保存済みの綴り**（ADR-0024 §1）。
    #[must_use]
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 向き。**提案されたとおり**（適用できたかどうかとは別）。
    #[must_use]
    pub const fn delta(&self) -> DriftDelta {
        self.delta
    }

    /// **DB から読み戻す内部経路。** `crate` の外からは呼べない。
    ///
    /// **`store` の読み出しが検証を通した後にだけ使う**
    /// （`StatAxis::parse` と向きの綴りの照合。`Store::recent_drift_votes`）。
    /// 公開すると、**照合していない軸名を票として持ち回れる。**
    pub(crate) const fn from_row(axis: StatAxis, delta: DriftDelta) -> Self {
        Self { axis, delta }
    }
}

/// **変容の走らせ方**（[ADR-0026](../../docs/adr/0026-drift-by-accumulation.md) §決定1）。**3値。**
///
/// | 値 | 判定を走らせるか | 票を記録するか | **`stat_alloc` を動かすか** |
/// |---|---|---|---|
/// | [`Self::Off`]（既定） | しない | しない | **しない** |
/// | [`Self::Shadow`] | する | **する** | **しない** |
/// | [`Self::On`] | する | する | する |
///
/// # なぜ `domain` にあるのか（`server::config` ではなく）
///
/// **[`Self::applying`] が [`DriftApply`] の唯一の発行元**で、その [`DriftApply`] は
/// `Store::apply_stat_drift` の引数に要る。つまりこの型は
/// **「値を動かしてよい」という規則そのもの**であって、環境変数の綴りの話ではない。
/// 綴り（`on` / `shadow` / `1` / `yes` …）を読むのは `server::config` の側で、
/// あちらは**この型を作るだけ**である（`Handle::parse` と HTTP 層の関係と同じ）。
///
/// # `bool` を2つ持たない
///
/// 「走らせるか」と「動かすか」を別々の `bool` にすると、
/// **`(走らせない, 動かす)` が表現できてしまう。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatDriftMode {
    /// **既定。** 判定そのものを走らせない（ADR-0024 §8）。
    Off,
    /// **記録専用**（ADR-0026 §決定1）。判定して票を残すが、**値は動かさない。**
    ///
    /// このモードでは**プロンプトが動かないので決定性が保たれ**、
    /// 同じ会話を2回投げて比べられる（ADR-0024 §8 の ⚠ が消える）。
    Shadow,
    /// 判定し、票を残し、**値も動かす**（16b の従来動作）。
    On,
}

impl StatDriftMode {
    /// すべての値。**集計と網羅テストの走査に使う。**
    pub const ALL: [Self; 3] = [Self::Off, Self::Shadow, Self::On];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    ///
    /// `server::config` が受け付ける綴りは**もっと広い**（`1` / `true` / `yes` …）が、
    /// ログに出る正はここ。**どの綴りで起動しても、記録には同じ語が載る。**
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::On => "on",
        }
    }

    /// **判定を走らせるか。** `Off` 以外。
    ///
    /// 呼び出し側（`server::extract`）はこれが偽なら**上流を呼ばない**——
    /// 呼んでから結果を捨てる形にすると、`Off` でも GPU を使う。
    #[must_use]
    pub const fn runs(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// **値を動かしてよいという許可。** [`Self::On`] だけが `Some` を返す。
    ///
    /// これが `Store::apply_stat_drift` の唯一の入場券である（[`DriftApply`] の doc）。
    #[must_use]
    pub fn applying(self) -> Option<DriftApply> {
        matches!(self, Self::On).then_some(DriftApply(()))
    }

    /// **キャラクターに保存する綴り**を読む（[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md) §決定3）。
    ///
    /// # 受けるのは [`Self::label`] の3つだけ
    ///
    /// 環境変数（`server::config`）は `0` / `true` / `yes` などの別名も受けるが、
    /// **ここは受けない。** 管理 API と `character.stat_drift_mode` 列の読み出しが通る入口で、
    /// 別名を受けると**保存された値の綴りが割れる**（同じ走らせ方が DB に2通りで残る）。
    ///
    /// **前後の空白も大文字も畳まない。** 畳むと `" on"` が保存されうる
    /// （保存するのは [`Self::label`] の綴りであって入力ではないが、
    /// 読み出しの側で畳むと、CHECK を外して書かれた行が黙って読めてしまう）。
    ///
    /// **綴りの正は [`Self::label`]。写しを作らない**——比較の相手を `ALL` の
    /// `label()` から取るので、ここに文字列は1つも無い。
    ///
    /// # Errors
    ///
    /// 3つの綴りのどれとも完全一致しなければ [`StatDriftModeError::Unknown`]。
    pub fn parse(raw: &str) -> Result<Self, StatDriftModeError> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.label() == raw)
            .ok_or(StatDriftModeError::Unknown)
    }
}

/// [`StatDriftMode::parse`] が受け付けなかった理由。
///
/// **`Display` は定数文字列**（入力を反射しない / 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StatDriftModeError {
    /// 3つの綴り（[`StatDriftMode::label`]）のどれでもない。
    #[error("変容の走らせ方の綴りが不正です")]
    Unknown,
}

/// **効いている走らせ方がどこから来たか**
/// （[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md) §決定2 / §決定4）。
///
/// 変容の応答（`GET .../drift`）と、変容を走らせる記録の欄に載る。
/// **測定を既存のキャラクターで回したときに、環境変数が効かなかったことを
/// 後から追えるように**（ADR-0045 §結果の ⚠）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriftModeSource {
    /// キャラクターに書いた値（`character.stat_drift_mode` が NULL でない）。
    Character,
    /// 起動時の既定（`PERSONACORE_STAT_DRIFT`。無ければ off）。**キャラクターは未設定。**
    Default,
}

impl DriftModeSource {
    /// すべての値。**集計と網羅テストの走査に使う。**
    pub const ALL: [Self; 2] = [Self::Character, Self::Default];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Default => "default",
        }
    }
}

/// **そのターンに効く走らせ方**と、その出どころ（ADR-0045 §決定2）。
///
/// # 規則は1つ —— **キャラクターの値があればそれ、無ければ起動時の既定**
///
/// **環境変数はキャラクターの値に勝たない**（ADR-0045 の却下欄:
/// 画面で「動かす」にしたのに動かない、が起きる）。
///
/// # ターンごとに決める
///
/// 呼ぶのは次の2箇所で、**どちらも、そのリクエストで引いたキャラクターの行から決める**:
///
/// * **変容を走らせる側** —— キャラクターの行は**チャット要求の入口**（`server::chat`）で引き、
///   解決するのは**応答を保存した後**の `server::extract::Extraction::start`。
///   **行は入口で引いた写しなので、応答を返している最中に来た `PATCH` はそのターンには効かず、
///   次のターンから効く。**
/// * **変容の応答** —— `server::admin::get_drift`（読んだその時の行）。
///
/// どちらの場合も起こし直しは要らない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedDriftMode {
    mode: StatDriftMode,
    source: DriftModeSource,
}

impl ResolvedDriftMode {
    /// キャラクターの値 `chosen` と起動時の既定 `default` から、効く走らせ方を決める。
    #[must_use]
    pub const fn resolve(chosen: Option<StatDriftMode>, default: StatDriftMode) -> Self {
        match chosen {
            Some(mode) => Self {
                mode,
                source: DriftModeSource::Character,
            },
            None => Self {
                mode: default,
                source: DriftModeSource::Default,
            },
        }
    }

    /// 効いている走らせ方。
    #[must_use]
    pub const fn mode(self) -> StatDriftMode {
        self.mode
    }

    /// その出どころ。
    #[must_use]
    pub const fn source(self) -> DriftModeSource {
        self.source
    }
}

/// **`stat_alloc` を動かしてよい**という許可証（ADR-0026 §決定1）。
///
/// `Store::apply_stat_drift` はこの値を**引数として要求する**ので、
/// **持っていない呼び出し側は、書き込みの文をそもそも呼べない。**
///
/// # ⚠ **`ValidTag` と同じ強さだと読まないこと**
///
/// 守れているのは「**このモジュールの外からは作れない**」までである
/// （フィールドが private / [`DriftChange`] と同じ強さ）。
/// [`StatDriftMode::applying`] はこのモジュールの中にあるので、
/// **同じモジュールの中（テストを含む）では構造体リテラルで組める。**
///
/// # ⚠ **「shadow が値を動かさない」を、この型が言い切っているのではない**（16d のレビューで実測）
///
/// **[`StatDriftMode::On`] と書けば、どこからでも許可証が取れる。**
/// 呼び出し側を `mode.applying()` から `StatDriftMode::On.applying()` へ変える変異は
/// **コンパイルが通り**、下の `only_the_applying_mode_hands_out_a_permit` は
/// **緑のまま**だった。この型が消しているのは
/// **「許可証を持たずに書き込みの文を呼ぶ」形だけ**である。
///
/// **権威は振る舞いを見る2本**——
/// `server::drift::tests::the_shadow_mode_never_touches_the_stored_values` と
/// `tests/server_stat_drift.rs::the_shadow_mode_records_a_vote_through_the_real_bind`。
///
/// > **⚠ 統合の1本が権威になったのは 2026-08-25 から**（作業単位 19 のレビュー）。
/// > **`shadow` の腕だけが既定の `K` で回っていて、既定の `K` では `on` でも1往復では動かない**
/// > ——**「値が動かない」がモードではなくつまみの結果**になっていた。
/// > **両腕を同じ安い `K` に揃えて直した**（実測: 直す前は緑・直した後は赤）。
///
/// # `Copy` にしない
///
/// 書き込み1回につき1枚。複製できると、**1枚の許可で何度でも書く**形が作れる。
#[derive(Debug)]
pub struct DriftApply(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::stat::{StatScale, fixture_scale};

    /// フィクスチャの尺度。**既定（10）と違う値**（`fixture_scale` = 7）。
    ///
    /// 既定を使うと「渡された尺度を無視して既定で判定している」実装が緑を通る
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    fn scale() -> StatScale {
        let scale = fixture_scale();
        assert_ne!(scale, StatScale::DEFAULT, "既定と違う尺度で測ること");
        scale
    }

    fn axis(raw: &str) -> StatAxis {
        StatAxis::parse(raw).expect("正当な軸名であること")
    }

    fn alloc(name: &str, value: u32) -> StatAlloc {
        StatAlloc {
            axis: axis(name),
            value: scale().value(value).expect("尺度に収まる値であること"),
        }
    }

    /// 保存済みの性格ステータス。**軸名も値も全部違う値にする。**
    ///
    /// 境界の4点（上端 / 上端−1 / **下端 0** / 1）が1本ずつ入っている。
    ///
    /// # **下端の軸と「プロンプトに載らない軸」は同じ1本になった**（ADR-0027 §決定5）
    ///
    /// [`DRIFT_FLOOR`] が `0` になったので、
    /// 「下端ちょうど」と「値 0（本文プロンプトに載らない）」が**同じ状態**である。
    /// 別々の軸に割り当てると値が重複し、**取り違えが検出できなくなる**
    /// （下の `assert_ne!` が落ちる）。
    ///
    /// # 入口で退化を潰す
    ///
    /// 2つの値を同値にすると、その取り違えが検出できない（CLAUDE.md）。
    /// 軸名・値のどちらも重複が無いことをここで言い切る。
    fn allocs() -> Vec<StatAlloc> {
        let top = scale().get();
        let stats = vec![
            alloc("パッション", 4),
            alloc("ロジック", 2),
            alloc("共感", top),
            alloc("好奇心", top - 1),
            // **下端＋1**（門が1段ずれていないことを見る側）。
            alloc("静けさ", DRIFT_FLOOR + 1),
            // **下端ちょうど。同時に「本文プロンプトに載らない軸」**。
            alloc("ユーモア", DRIFT_FLOOR),
        ];

        for (index, left) in stats.iter().enumerate() {
            for right in &stats[index + 1..] {
                assert_ne!(left.axis, right.axis, "軸名が重複している");
                assert_ne!(
                    left.value.get(),
                    right.value.get(),
                    "値が重複している（取り違えが検出できなくなる）"
                );
            }
        }
        assert!(
            stats.iter().any(|stat| !stat.appears_in_prompt()),
            "値 0 の軸が1本あること（§4 の分岐に入る状態を作る）"
        );
        assert_eq!(
            top, 7,
            "前提: 境界の4点が別々の軸に割り当てられる尺度であること"
        );

        stats
    }

    /// 提案を1つ作る。
    fn suggest(name: &str, delta: DriftDelta) -> DriftProposal {
        DriftProposal::Suggested(DriftSuggestion {
            axis: axis(name),
            delta,
        })
    }

    fn applied(name: &str, delta: DriftDelta) -> DriftChange {
        match apply_drift(&allocs(), suggest(name, delta)) {
            DriftOutcome::Applied(change) => change,
            other => panic!("適用されること: {name} {delta:?} -> {other:?}"),
        }
    }

    /// **適用は必ず 1 段だけ動かす**（ADR-0024 §3）。
    ///
    /// 幅の固定を緩める変異（`+2`）も、`before == after` にする変異も、ここで赤くなる。
    #[test]
    fn an_applied_change_always_moves_by_exactly_one_step() {
        let up = applied("パッション", DriftDelta::Up);
        assert_eq!(up.before().get(), 4, "読んだ値をそのまま持つこと");
        assert_eq!(up.after().get(), 5);
        assert_eq!(up.delta(), DriftDelta::Up);

        let down = applied("パッション", DriftDelta::Down);
        assert_eq!(down.before().get(), 4);
        assert_eq!(down.after().get(), 3);
        assert_eq!(down.delta(), DriftDelta::Down);

        // **どの軸のどちらの向きでも、動くときは必ず 1 段。**
        for stat in allocs() {
            for delta in DriftDelta::ALL {
                if let DriftOutcome::Applied(change) =
                    apply_drift(&allocs(), suggest(stat.axis.as_str(), delta))
                {
                    let before = i64::from(change.before().get());
                    let after = i64::from(change.after().get());
                    assert_eq!(
                        (after - before).abs(),
                        1,
                        "1 段以外の幅で動いた: {} {delta:?} {before} -> {after}",
                        stat.axis
                    );
                    assert_eq!(change.delta(), delta, "向きが提案と違う: {}", stat.axis);
                }
            }
        }
    }

    /// **上端では飽和する。クランプして「動いた」と記録しない**（ADR-0024 §5）。
    ///
    /// 境界は1段単位で見る——**上端ちょうど**と**上端−1**の両方。
    #[test]
    fn the_top_of_the_scale_saturates_instead_of_clamping() {
        let top = scale().get();

        assert_eq!(
            apply_drift(&allocs(), suggest("共感", DriftDelta::Up)),
            DriftOutcome::Saturated {
                axis: axis("共感"),
                delta: DriftDelta::Up,
            },
            "上端ちょうどで +1 は飽和すること"
        );

        // **上端−1 からは動ける**（飽和の門が1段ずれていないこと）。
        let curious = applied("好奇心", DriftDelta::Up);
        assert_eq!(curious.before().get(), top - 1);
        assert_eq!(curious.after().get(), top);

        // 上端からでも下げる側は動く（向きを取り違える変異を赤にする）。
        let empathy = applied("共感", DriftDelta::Down);
        assert_eq!(empathy.before().get(), top);
        assert_eq!(empathy.after().get(), top - 1);
    }

    /// **下端は 0。`0` から下は飽和する**（ADR-0027 §決定5）。
    ///
    /// 境界は1段単位で見る——**下端ちょうど**と**下端＋1**の両方。
    ///
    /// # ⚠ 以前は「下端は 1」の見張り番だった
    ///
    /// `the_floor_is_one_so_drift_never_reaches_zero` を置き換えたもの
    /// （16e-b / 司令塔の採否済み）。**`0` へ落ちられることは
    /// `drift_can_reach_zero_but_never_goes_below` が網羅で見る**ので、
    /// ここが持つのは**境界が1段ずれていないこと**である。
    #[test]
    fn the_floor_is_zero_and_only_zero_saturates() {
        assert_eq!(
            apply_drift(&allocs(), suggest("ユーモア", DriftDelta::Down)),
            DriftOutcome::Saturated {
                axis: axis("ユーモア"),
                delta: DriftDelta::Down,
            },
            "0 から -1 は飽和すること（負の値を作らない）"
        );

        // **下端＋1 からは下げられる**（門が1段ずれていないこと）。
        let quiet = applied("静けさ", DriftDelta::Down);
        assert_eq!(quiet.before().get(), DRIFT_FLOOR + 1);
        assert_eq!(quiet.after().get(), DRIFT_FLOOR);
        assert_eq!(quiet.after().get(), 0, "下端の値そのものを直書きで押さえる");

        // 下端からでも上げる側は動く（**`0` は片道ではない**）。
        let humour = applied("ユーモア", DriftDelta::Up);
        assert_eq!(humour.before().get(), DRIFT_FLOOR);
        assert_eq!(humour.after().get(), 1);
    }

    /// **`0` へは落ちられる。その下へは落ちない**（ADR-0027 §決定5）。
    ///
    /// 尺度の全域を1段ずつ走る**網羅**。
    ///
    /// # ⚠ **`drift_never_produces_a_zero_value` を置き換えたもの**（16e-b / 司令塔の採否済み）
    ///
    /// 下端が `1` だった頃は「適用結果が `0` になることは無い」が守るものだった。
    /// **下端が `0` になった**（ADR-0027 §決定5）ので、その主張はもう仕様ではない。
    ///
    /// # ここが持つのは「落ちられること」だけ
    ///
    /// **「`0` から戻れること」は別の見張り番**
    /// （[`a_zero_valued_axis_can_now_be_raised_from_the_floor_up`]）。
    /// **1本にまとめない**——まとめると、`0` が片道（落ちたきり）になっている状態が
    /// 緑を通る。§決定5 が下端を開けられた根拠は「片道ではない」ことそのものである。
    #[test]
    fn drift_can_reach_zero_but_never_goes_below() {
        let scale = scale();
        let mut reached_zero = false;
        for raw in 0..=scale.get() {
            let stats = vec![alloc("パッション", raw)];
            for delta in DriftDelta::ALL {
                let outcome = apply_drift(&stats, suggest("パッション", delta));
                if let DriftOutcome::Applied(change) = &outcome {
                    // **`0` は正当な着地点**だが、その下は無い（`u32` なので回り込みも見る）。
                    assert!(
                        change.after().get() <= scale.get(),
                        "変容が尺度の外へ出た: {raw} {delta:?} -> {}",
                        change.after().get()
                    );
                    if change.after().get() == 0 {
                        reached_zero = true;
                        assert_eq!(raw, 1, "0 へ落ちられるのは 1 からだけ");
                        assert_eq!(delta, DriftDelta::Down);
                    }
                    assert!(
                        change.after().scale() == scale && change.before().scale() == scale,
                        "前後で同じ尺度を使うこと: {outcome:?}"
                    );
                }
                if raw == 0 && delta == DriftDelta::Down {
                    assert_eq!(
                        outcome,
                        DriftOutcome::Saturated {
                            axis: axis("パッション"),
                            delta,
                        },
                        "値 0 の軸を下げようとしたら飽和すること（負の値を作らない）"
                    );
                }
            }
        }
        assert!(
            reached_zero,
            "尺度の全域を走っても 0 に着地しなかった（下端がまだ 1 のまま）"
        );
    }

    /// **保存済みの軸名と完全一致しないものは捨てる**（ADR-0024 §1 の許可リスト）。
    ///
    /// 前方一致・後方一致・部分一致に緩める変異は、ここで赤くなる。
    #[test]
    fn an_axis_that_is_not_stored_is_discarded() {
        for name in [
            "パッ",         // 前方一致
            "パッション力", // 後方に足した
            "ッショ",       // 部分一致
            "情熱",         // 別の語
            "passion",      // 綴りが違う
            "パツション",   // 1文字違い
        ] {
            for delta in DriftDelta::ALL {
                assert_eq!(
                    apply_drift(&allocs(), suggest(name, delta)),
                    DriftOutcome::Discarded(DriftDiscardReason::UnknownAxis),
                    "保存されていない軸が通った: {name} {delta:?}"
                );
            }
        }

        // **前後の空白だけの違いは同じ軸**（`StatAxis::parse` が正規化する）。
        assert!(matches!(
            apply_drift(&allocs(), suggest(" パッション ", DriftDelta::Up)),
            DriftOutcome::Applied(_)
        ));
    }

    /// **値 0 の軸は、保存されていれば上げられる**（ADR-0027 §決定4 / §決定6）。
    ///
    /// # ここが「ゼロ始まりのロールプレイ」の全体を支えている
    ///
    /// ADR-0027 §11 の実測——**中間から始めると均され、ゼロから始めると分化する。**
    /// §4 の門が残っていると、**全部ゼロのキャラクターは提案が毎回捨てられ、
    /// 一度も動かない。** その壊れ方は「提案は出ているのに値が動かない」なので、
    /// **ログの `outcome=` を見て初めて分かる**（本文には何も出ない）。
    ///
    /// # 下げる側は飽和のまま
    ///
    /// [`DRIFT_FLOOR`] は `1` なので `0` からは下がらない。
    /// **破棄ではなく飽和**——提案そのものは正しく、端で動けないだけである。
    #[test]
    fn a_zero_valued_axis_can_now_be_raised_from_the_floor_up() {
        let stats = allocs();
        let humour = stats
            .iter()
            .find(|stat| stat.axis == axis("ユーモア"))
            .expect("前提: 値 0 の軸が保存されていること");
        assert!(
            !humour.appears_in_prompt(),
            "前提: その軸が本文プロンプトには載らない軸であること"
        );

        let outcome = apply_drift(&stats, suggest("ユーモア", DriftDelta::Up));
        let DriftOutcome::Applied(change) = &outcome else {
            panic!("値 0 の軸が上げられなかった: {outcome:?}");
        };
        assert_eq!(change.axis(), &axis("ユーモア"));
        assert_eq!(change.before().get(), 0, "読んだ値をそのまま持つこと");
        assert_eq!(change.after().get(), 1, "1 段だけ上がること");

        assert_eq!(
            apply_drift(&stats, suggest("ユーモア", DriftDelta::Down)),
            DriftOutcome::Saturated {
                axis: axis("ユーモア"),
                delta: DriftDelta::Down,
            },
            "0 からは下がらない（負の値を作らない）"
        );
    }

    /// **「なし」の回・飽和の回・適用の回・破棄の回が、同じ入力集合の中に混在する。**
    ///
    /// CLAUDE.md「フィクスチャが『その分岐が効く状態』に一度も入らない」への備え。
    /// 分岐ごとに別のテストへ切ると、**同じ保存状態でどれか1つしか起きない**形の
    /// フィクスチャになりやすく、結末を1つに畳む変異（例: 何でも `Nothing`）が
    /// 「そのテストだけ赤」で済んでしまう。
    #[test]
    fn every_outcome_shows_up_in_one_pass_over_the_same_stats() {
        let stats = allocs();
        let cases: [(DriftProposal, &str); 6] = [
            (DriftProposal::Nothing, "nothing"),
            (suggest("パッション", DriftDelta::Up), "applied"),
            (suggest("共感", DriftDelta::Up), "saturated"),
            // **下端＋1 からは `0` まで落ちられる**（ADR-0027 §決定5）。
            (suggest("静けさ", DriftDelta::Down), "applied"),
            // **値 0 の軸も動く**（ADR-0027 §決定4 / §決定6）。下げようとすれば飽和する。
            (suggest("ユーモア", DriftDelta::Down), "saturated"),
            (suggest("情熱", DriftDelta::Up), "unknown_axis"),
        ];

        let mut seen: Vec<&str> = Vec::new();
        for (proposal, expected) in cases {
            let outcome = apply_drift(&stats, proposal);
            assert_eq!(outcome.label(), expected, "結末が違う: {outcome:?}");
            seen.push(outcome.label());
        }

        for label in ["nothing", "applied", "saturated", "unknown_axis"] {
            assert!(
                seen.contains(&label),
                "この一巡で {label} が一度も出ていない"
            );
        }
    }

    /// 壊れた分解結果は、理由を保ったまま結末に写る（付け替えない）。
    #[test]
    fn a_discarded_proposal_keeps_its_reason() {
        for reason in DriftDiscardReason::ALL {
            assert_eq!(
                apply_drift(&allocs(), DriftProposal::Discarded(reason)),
                DriftOutcome::Discarded(reason),
                "破棄の理由が付け替えられた: {}",
                reason.label()
            );
        }
    }

    /// **提案が無ければ何も起きない。**
    #[test]
    fn no_proposal_means_no_change() {
        assert_eq!(
            apply_drift(&allocs(), DriftProposal::Nothing),
            DriftOutcome::Nothing
        );
        // 軸が1本も無くても panic しない。
        assert_eq!(
            apply_drift(&[], suggest("パッション", DriftDelta::Up)),
            DriftOutcome::Discarded(DriftDiscardReason::UnknownAxis)
        );
    }

    /// **[`apply_drift`] が出す破棄の理由は許可リスト由来の2つだけ。**
    ///
    /// 分解の段階の理由（`Malformed` / `MultipleLines`）をここで作り始めると、
    /// **どの層で落ちたのかがログから読めなくなる。**
    /// （素通しの `Discarded` は入力の理由をそのまま返すだけなので、ここでは提案だけを回す。）
    #[test]
    fn apply_drift_only_reports_the_allowlist_reasons() {
        let stats = allocs();
        for name in ["パッション", "ユーモア", "情熱", "共感", "静けさ", ""] {
            for delta in DriftDelta::ALL {
                let Ok(parsed) = StatAxis::parse(name) else {
                    continue;
                };
                let outcome = apply_drift(
                    &stats,
                    DriftProposal::Suggested(DriftSuggestion {
                        axis: parsed,
                        delta,
                    }),
                );
                if let DriftOutcome::Discarded(reason) = outcome {
                    assert!(
                        reason.from_allowlist(),
                        "分解の段階の理由を適用側が作った: {} ({name})",
                        reason.label()
                    );
                }
            }
        }
    }

    /// **`DriftDiscardReason::ALL` はすべての値をちょうど1回ずつ並べ、集計キーの綴りも固定する。**
    ///
    /// `match` で書いてあるので、**値が増えたらこのテストがコンパイルできなくなる**
    /// （`ALL` に足し忘れたまま集計から漏れる経路を塞ぐ）。
    /// **減らす方向**は `ALL.len()` と、各値が `ALL` にちょうど1つあることが赤にする
    /// ——`NotInPrompt` を落とす変異が緑を通った（実測 / 2026-08-23 / レビュー指摘3）。
    ///
    /// 綴りを直書きするのは、`every_outcome_label_is_distinct` が
    /// **重ならないことしか見ない**ため。`"not_in_prompt"` → `"nip"` の改名が
    /// そちらを素通りする（同じく実測）。**集計キーはログの列名として外から読まれる値**で、
    /// 黙って変わると過去の測定値と突き合わせられなくなる。
    /// 綴りは `parse::extraction::DiscardReason::label` と同じ作法。
    #[test]
    fn all_lists_every_discard_reason_exactly_once() {
        for reason in DriftDiscardReason::ALL {
            let expected = match reason {
                DriftDiscardReason::Malformed => "malformed",
                DriftDiscardReason::MultipleLines => "multiple_lines",
                DriftDiscardReason::UnknownAxis => "unknown_axis",
                DriftDiscardReason::MissingAxis => "missing_axis",
                DriftDiscardReason::DuplicateAxis => "duplicate_axis",
                DriftDiscardReason::Truncated => "truncated",
            };
            assert_eq!(reason.label(), expected, "集計キーの綴りが変わった");
            assert_eq!(
                DriftDiscardReason::ALL
                    .iter()
                    .filter(|candidate| **candidate == reason)
                    .count(),
                1,
                "{} が ALL にちょうど1つ並んでいない",
                reason.label()
            );
        }
        assert_eq!(
            DriftDiscardReason::ALL.len(),
            6,
            "ADR-0024 §6 / ADR-0027 §12 / ADR-0027 §打ち切り の理由は6つ"
        );
    }

    /// **`DriftDelta::ALL` は両方の向きをちょうど1回ずつ並べ、綴りも固定する。**
    ///
    /// # ここが欠けると、指標が壊れたことに気付けない
    ///
    /// ADR-0024 の「危険 2. 一方向へ流れる」は **`+1` と `-1` の比**で見る。
    /// `ALL` から `Down` が落ちると、走査で数える側は**常に「全部 up」**になり、
    /// **本当に一方向へ流れている状態と、指標が壊れている状態が区別できない。**
    /// `[Self::Up]` だけにする変異が緑を通った（実測 / 2026-08-23 / レビュー指摘3）。
    ///
    /// `sign()` も一緒に固定する——こちらは**プロンプトの手本の綴りの正**で、
    /// `prompt::drift` の `the_examples_in_the_request_parse_back_into_a_suggestion` が
    /// この値と手本を突き合わせている。
    #[test]
    fn all_lists_every_delta_exactly_once() {
        for delta in DriftDelta::ALL {
            let (label, sign) = match delta {
                DriftDelta::Up => ("up", "+1"),
                DriftDelta::Down => ("down", "-1"),
            };
            assert_eq!(delta.label(), label, "集計キーの綴りが変わった");
            assert_eq!(delta.sign(), sign, "手本の符号の綴りが変わった");
            assert_eq!(
                DriftDelta::ALL
                    .iter()
                    .filter(|candidate| **candidate == delta)
                    .count(),
                1,
                "{label} が ALL にちょうど1つ並んでいない"
            );
        }
        assert_eq!(DriftDelta::ALL.len(), 2, "向きは上げと下げの2つ");
    }

    /// **結末の集計キーの綴りも固定する。**
    ///
    /// `label()` は破棄のときだけ [`DriftDiscardReason::label`] へ委ねるので、
    /// **委譲を止めて定数を返す変異**（例: 全部 `"discarded"`）は
    /// `every_outcome_label_is_distinct` を素通りしうる（重なりが増えないため）。
    #[test]
    fn every_outcome_label_is_spelled_the_way_the_logs_expect() {
        assert_eq!(DriftOutcome::Nothing.label(), "nothing");
        assert_eq!(
            DriftOutcome::Saturated {
                axis: axis("共感"),
                delta: DriftDelta::Up,
            }
            .label(),
            "saturated"
        );
        assert_eq!(
            apply_drift(&allocs(), suggest("パッション", DriftDelta::Up)).label(),
            "applied"
        );
        for reason in DriftDiscardReason::ALL {
            assert_eq!(
                DriftOutcome::Discarded(reason).label(),
                reason.label(),
                "破棄の結末は理由の綴りをそのまま使うこと"
            );
        }
    }

    /// **集計キーが互いに区別できる。** 1つでも重なると、ログの数が別の意味と混ざる。
    #[test]
    fn every_outcome_label_is_distinct() {
        let mut labels: Vec<&str> = DriftDiscardReason::ALL
            .into_iter()
            .map(DriftDiscardReason::label)
            .collect();
        labels.push(DriftOutcome::Nothing.label());
        labels.push(
            DriftOutcome::Saturated {
                axis: axis("共感"),
                delta: DriftDelta::Up,
            }
            .label(),
        );
        labels.push(apply_drift(&allocs(), suggest("パッション", DriftDelta::Up)).label());
        // **全軸採点の結末も同じ欄に載る**（ADR-0027 §12 / 作業単位 16e-b）。
        // 依頼の形によって `DriftOutcome::label` と `DriftScoring::label` の
        // どちらかが結末の欄に出るので、**2つの語彙を跨いで重ならないこと**が要る。
        labels.push(DriftScoring::Scored(Vec::new()).label());

        let total = labels.len();
        for label in &labels {
            assert!(!label.is_empty(), "空の集計キーがある");
        }
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), total, "集計キーが重なっている: {labels:?}");

        // 向きの集計キーも区別できること（`+1` と `-1` の比を見るため）。
        assert_ne!(DriftDelta::Up.label(), DriftDelta::Down.label());
        assert_ne!(DriftDelta::Up.sign(), DriftDelta::Down.sign());
    }

    /// **どの理由が許可リスト由来かの印が、実際の分類と合っている。**
    #[test]
    fn the_allowlist_marker_matches_the_reasons_apply_can_produce() {
        assert!(DriftDiscardReason::UnknownAxis.from_allowlist());
        assert!(!DriftDiscardReason::Malformed.from_allowlist());
        assert!(!DriftDiscardReason::MultipleLines.from_allowlist());
        // **全軸採点の理由は分解の段階でしか出ない**（ADR-0027 §12）。
        assert!(!DriftDiscardReason::MissingAxis.from_allowlist());
        assert!(!DriftDiscardReason::DuplicateAxis.from_allowlist());
    }

    /// **`DriftForm::ALL` は両方の形をちょうど1回ずつ並べ、綴りも固定する。**
    ///
    /// `match` で書いてあるので、**形が増えたらコンパイルが通らない。**
    /// 綴りは `server::config` が受け付ける語と同じで、**ログに出た語を
    /// そのまま環境変数へ書き戻せる**ことを `server::config` 側の
    /// `every_drift_form_label_is_a_spelling_the_parser_accepts` が突き合わせる。
    #[test]
    fn all_lists_every_drift_form_exactly_once() {
        for form in DriftForm::ALL {
            let expected = match form {
                DriftForm::Single => "single",
                DriftForm::All => "all",
            };
            assert_eq!(form.label(), expected, "集計キーの綴りが変わった");
            assert_eq!(
                DriftForm::ALL
                    .iter()
                    .filter(|candidate| **candidate == form)
                    .count(),
                1,
                "{expected} が ALL にちょうど1つ並んでいない"
            );
        }
        assert_eq!(DriftForm::ALL.len(), 2, "依頼の形は2値");
        assert_ne!(
            DriftForm::Single.label(),
            DriftForm::All.label(),
            "形の集計キーが重なっている"
        );
        // **「既定は `All`」の権威は `server::config`** にある
        // （`the_drift_form_defaults_to_the_all_axis_request`）。
        // ここは語彙だけを固定する。
    }

    /// **`DriftNote::ALL` は3本の腕をちょうど1回ずつ並べ、綴りも固定する。**
    ///
    /// `match` で書いてあるので、**腕が増えたらコンパイルが通らない**
    /// （[`all_lists_every_drift_form_exactly_once`] と同じ形）。
    ///
    /// 綴りは `server::config` が受け付ける語と同じで、
    /// **測定のログに出た語をそのまま `PERSONACORE_DRIFT_NOTE` へ書き戻せる**ことは
    /// あちらの `every_drift_note_label_is_a_spelling_the_parser_accepts` が突き合わせる。
    #[test]
    fn all_lists_every_drift_note_exactly_once() {
        for note in DriftNote::ALL {
            let expected = match note {
                DriftNote::None => "none",
                DriftNote::Copy => "copy",
                DriftNote::Language => "language",
            };
            assert_eq!(note.label(), expected, "集計キーの綴りが変わった");
            assert_eq!(
                DriftNote::ALL
                    .iter()
                    .filter(|candidate| **candidate == note)
                    .count(),
                1,
                "{expected} が ALL にちょうど1つ並んでいない"
            );
        }
        assert_eq!(DriftNote::ALL.len(), 3, "腕は3本");

        // **綴りが1つも重ならない**（重なると、3本の腕がログで区別できない）。
        for (index, left) in DriftNote::ALL.iter().enumerate() {
            for right in &DriftNote::ALL[index + 1..] {
                assert_ne!(left.label(), right.label(), "腕の集計キーが重なっている");
            }
        }
        // **「既定は `None`」の権威は `server::config`** にある
        // （`the_drift_note_defaults_to_no_note`）。ここは語彙だけを固定する。
    }

    /// **採点表は「0 と採点された軸」を持てるが、「行が無かった軸」は持てない。**
    ///
    /// ADR-0027 §12 が名指しする区別。**行が欠けたら採点表そのものが作られず**、
    /// [`DriftScoring::Discarded`] になる（分解の側の見張り番は `parse::drift`）。
    /// ここで見るのは**型としてそうなっている**こと——
    /// [`DriftScore::delta`] の `None` に「無かった」の意味を持たせない。
    #[test]
    fn a_zero_score_carries_its_axis_and_is_not_an_absence() {
        let zero = DriftScore::new(axis("ユーモア"), None);
        let moved = DriftScore::new(axis("パッション"), Some(DriftDelta::Up));

        assert_eq!(zero.axis(), &axis("ユーモア"), "採点表が軸を取り違えた");
        assert_eq!(zero.delta(), None, "0 は向きを持たない");
        assert_eq!(moved.delta(), Some(DriftDelta::Up));
        assert_ne!(zero, moved);

        // **欠けは採点表の外側で表す。** 同じ軸の「0」と「行が無い」が別の値になる。
        let scored = DriftScoring::Scored(vec![zero.clone()]);
        let missing = DriftScoring::Discarded(DriftDiscardReason::MissingAxis);
        assert_ne!(scored, missing);
    }

    // ---------------------------------------------------------------------
    // 走らせ方と許可証（ADR-0026 §決定1）
    // ---------------------------------------------------------------------

    /// **許可証を出すのは `On` だけ。**
    ///
    /// # ⚠ **これは「唯一の権威」ではない**（16d のレビューで実測）
    ///
    /// 当初この doc は「`server::drift` の側には分岐が無いので、この見張り番が
    /// 唯一の権威」と書いていた。**誤りだった。**
    /// **`StatDriftMode::On.applying()` はどこからでも呼べる**ので、
    /// 呼び出し側を `mode.applying()` からそう書き換える変異は
    /// **コンパイルが通り、このテストは緑のまま通る。**
    ///
    /// **赤くなったのは振る舞いを見る2本だけ**だった——
    /// `server::drift::tests::the_shadow_mode_never_touches_the_stored_values` と
    /// `tests/server_stat_drift.rs::the_shadow_mode_records_a_vote_through_the_real_bind`。
    /// **記録専用モードで値が動かないことの権威はそちらにある。**
    ///
    /// > **⚠ 統合の1本が権威になったのは 2026-08-25 から**（作業単位 19 のレビュー）。
    /// > **`shadow` の腕だけが既定の `K` で回っていて、既定の `K` では `on` でも
    /// > 1往復では動かない**ため、**「値が動かない」がモードではなくつまみの結果**だった。
    /// > **同じ変異で統合の1本は緑を通っていた。** 両腕を同じ安い `K` に揃えて直した。
    ///
    /// # それでもここが要る
    ///
    /// ここが守るのは「**`applying` の分け方そのもの**」で、
    /// 上の2本は「**その分け方が現に使われていること**」を見る。
    /// 両方が無いと、片方を緩める変異がもう片方の期待値を道連れにする。
    ///
    /// 実害: 記録専用モードは「値が動かないから決定性が保たれる」ことが存在理由で、
    /// ここが `Some` を返すと**測定の前提が黙って崩れる**
    /// ——票は同じように溜まるので、集計だけ見ても気付けない。
    #[test]
    fn only_the_applying_mode_hands_out_a_permit() {
        assert!(
            StatDriftMode::On.applying().is_some(),
            "on は値を動かせること"
        );
        assert!(
            StatDriftMode::Shadow.applying().is_none(),
            "記録専用モードは値を動かせないこと"
        );
        assert!(
            StatDriftMode::Off.applying().is_none(),
            "off は値を動かせないこと"
        );
    }

    /// **判定を走らせるのは `Off` 以外。**
    ///
    /// `applying()` と**別の分け方**であることが要点——
    /// 「走らせる」と「動かす」が同じ集合になると、記録専用モードが消える。
    #[test]
    fn the_recording_modes_run_but_only_one_of_them_applies() {
        assert!(!StatDriftMode::Off.runs(), "off は走らせないこと");
        assert!(StatDriftMode::Shadow.runs(), "記録専用は走らせること");
        assert!(StatDriftMode::On.runs(), "on は走らせること");

        // **2つの分け方が同じ集合にならない。** 同じなら3値である意味が無い。
        let runs: Vec<bool> = StatDriftMode::ALL.iter().map(|mode| mode.runs()).collect();
        let applies: Vec<bool> = StatDriftMode::ALL
            .iter()
            .map(|mode| mode.applying().is_some())
            .collect();
        assert_ne!(
            runs, applies,
            "「走らせる」と「動かす」が同じ分け方になっている（記録専用が消えている）"
        );
    }

    /// **`ALL` は3つの値をちょうど1回ずつ並べ、綴りも固定する。**
    ///
    /// 綴りは**ログにも環境変数にも出る**（`server::config` の
    /// `every_mode_label_is_a_spelling_the_parser_accepts` が書き戻せることを見張る）。
    #[test]
    fn all_lists_every_mode_exactly_once() {
        for mode in StatDriftMode::ALL {
            let label = match mode {
                StatDriftMode::Off => "off",
                StatDriftMode::Shadow => "shadow",
                StatDriftMode::On => "on",
            };
            assert_eq!(mode.label(), label, "集計キーの綴りが変わった");
            assert_eq!(
                StatDriftMode::ALL
                    .iter()
                    .filter(|candidate| **candidate == mode)
                    .count(),
                1,
                "{label} が ALL にちょうど1つ並んでいない"
            );
        }
        assert_eq!(StatDriftMode::ALL.len(), 3, "走らせ方は3値");
    }

    // ---------------------------------------------------------------------
    // キャラクターごとの走らせ方（ADR-0045 / 作業単位 A-8）
    // ---------------------------------------------------------------------

    /// **保存する綴りは3つだけを受け、環境変数の別名は受けない**（ADR-0045 §決定3）。
    ///
    /// **期待値は直書き**（`label()` から作ると、綴りを入れ替える変異が
    /// 期待値も一緒に入れ替えて緑のまま）。
    ///
    /// 拒む側には**環境変数が受ける別名を全部**並べる（`server::config` の
    /// `STAT_DRIFT_ON` / `STAT_DRIFT_OFF` の綴り。**あちらの定数は呼ばない**——
    /// 呼ぶと、あちらが語を減らしたときにこちらの主張も一緒に縮む）。
    #[test]
    fn a_stored_drift_mode_accepts_only_the_three_labels() {
        assert_eq!(StatDriftMode::parse("off"), Ok(StatDriftMode::Off));
        assert_eq!(StatDriftMode::parse("shadow"), Ok(StatDriftMode::Shadow));
        assert_eq!(StatDriftMode::parse("on"), Ok(StatDriftMode::On));

        for alias in [
            // 環境変数だけが受ける別名（空文字列を含む）。
            "0", "1", "true", "false", "yes", "no", "",
            // 大文字・前後の空白は畳まない。
            "ON", "Off", "SHADOW", " on", "on ", "shadow\n", "\toff",
            // 似ているが違う語。
            "onn", "shadows", "of",
        ] {
            assert_eq!(
                StatDriftMode::parse(alias),
                Err(StatDriftModeError::Unknown),
                "{alias:?} を受けないこと"
            );
        }
    }

    /// **拒否の `Display` は定数で、入力を反射しない**（絶対制約）。
    #[test]
    fn a_rejected_drift_mode_does_not_echo_the_input() {
        let planted = "planted-mode-spelling";
        let err = StatDriftMode::parse(planted).expect_err("受けないこと");
        let shown = err.to_string();
        assert!(!shown.contains(planted), "{shown}");
        assert_eq!(shown, "変容の走らせ方の綴りが不正です");
    }

    /// **解決の表**（キャラクターの値 × 起動時の既定 / ADR-0045 §決定1・§決定2）。
    ///
    /// # 12 マスを1本で固定する
    ///
    /// **期待値は綴りの直書き**（`label()` からも `resolve` からも作らない）。
    /// 腕の選び方の要点は2つ:
    ///
    /// * **既定と違う値で回る腕**（`off` 既定 × `on` / `on` 既定 × `off`）——
    ///   ここが無いと「常に既定を返す」実装が、キャラクターの値が既定と
    ///   同じマスでは区別できない。
    /// * **キャラクターの値と既定が一致するマスでも出どころは `character`**——
    ///   値だけ見ると「一致するなら既定扱い」の実装が通り抜ける。
    #[test]
    fn the_resolution_table_of_character_value_and_default() {
        use StatDriftMode::{Off, On, Shadow};
        let table: [(Option<StatDriftMode>, StatDriftMode, (&str, &str)); 12] = [
            // 未設定は既定に従う。
            (None, Off, ("off", "default")),
            (None, Shadow, ("shadow", "default")),
            (None, On, ("on", "default")),
            // キャラクターの値が勝つ。
            (Some(Off), Off, ("off", "character")),
            (Some(Off), Shadow, ("off", "character")),
            (Some(Off), On, ("off", "character")),
            (Some(Shadow), Off, ("shadow", "character")),
            (Some(Shadow), Shadow, ("shadow", "character")),
            (Some(Shadow), On, ("shadow", "character")),
            (Some(On), Off, ("on", "character")),
            (Some(On), Shadow, ("on", "character")),
            (Some(On), On, ("on", "character")),
        ];

        for (chosen, default, expected) in table {
            let resolved = ResolvedDriftMode::resolve(chosen, default);
            assert_eq!(
                (resolved.mode().label(), resolved.source().label()),
                expected,
                "キャラクター = {chosen:?} / 既定 = {default:?}"
            );
        }
    }

    /// **出どころの綴りを固定する**（変容の応答とログの欄に載る）。
    ///
    /// **名前に記録の欄名を含めない**——`server::drift` の
    /// `the_drift_fields_never_collide_with_another_module` が他のモジュールの綴りを落とす
    /// （A-8 の着手時に実際に踏んだ）。
    #[test]
    fn all_lists_every_mode_source_exactly_once() {
        for source in DriftModeSource::ALL {
            let expected = match source {
                DriftModeSource::Character => "character",
                DriftModeSource::Default => "default",
            };
            assert_eq!(source.label(), expected, "集計キーの綴りが変わった");
            assert_eq!(
                DriftModeSource::ALL
                    .iter()
                    .filter(|candidate| **candidate == source)
                    .count(),
                1,
                "{expected} が ALL にちょうど1つ並んでいない"
            );
        }
        assert_eq!(DriftModeSource::ALL.len(), 2, "出どころは2値");
    }

    // ---------------------------------------------------------------------
    // 票（ADR-0026 §決定3）
    // ---------------------------------------------------------------------

    /// **適用できた結末は、提案どおりの向きの票になる。**
    ///
    /// # 「載っているか」ではなく「どれのものか」を見る
    ///
    /// 軸も向きも**2通りずつ**回す。1組だけで測ると、
    /// **軸を決め打ちにする実装も、向きを決め打ちにする実装も緑を通る**
    /// （CLAUDE.md「抜けるのはいつも帰属」）。
    #[test]
    fn an_applied_outcome_votes_for_the_axis_and_direction_that_were_proposed() {
        let cases = [
            ("パッション", DriftDelta::Up),
            ("パッション", DriftDelta::Down),
            ("ロジック", DriftDelta::Up),
            ("ロジック", DriftDelta::Down),
        ];
        for (name, delta) in cases {
            let outcome = apply_drift(&allocs(), suggest(name, delta));
            assert!(
                matches!(outcome, DriftOutcome::Applied(_)),
                "前提: 適用できる提案であること: {name} {delta:?} / {outcome:?}"
            );

            let vote = outcome.vote().expect("票になること");
            assert_eq!(vote.axis().as_str(), name, "提案された軸の票になること");
            assert_eq!(vote.delta(), delta, "提案された向きの票になること");
        }
    }

    /// **飽和した結末も票になる**（ADR-0026 §決定3）。
    ///
    /// 記録の時点で落とすと、「上げようとし続けている」という情報が票から消える。
    /// **上端への `+1` と下端（`0`）への `-1` の両方**で測る。
    #[test]
    fn a_saturated_outcome_still_becomes_a_vote() {
        let cases = [("共感", DriftDelta::Up), ("ユーモア", DriftDelta::Down)];
        for (name, delta) in cases {
            let outcome = apply_drift(&allocs(), suggest(name, delta));
            assert!(
                matches!(outcome, DriftOutcome::Saturated { .. }),
                "前提: 飽和する提案であること: {name} {delta:?} / {outcome:?}"
            );

            let vote = outcome.vote().expect("飽和も票になること");
            assert_eq!(vote.axis().as_str(), name);
            assert_eq!(vote.delta(), delta, "提案された向きのまま残ること");
        }
    }

    /// **提案なしと破棄は票にならない**（ADR-0026 §決定3）。
    ///
    /// 破棄は「門を通っていない」——**票に入れると、
    /// モデルが作った軸名や、語らないと決めた軸が蓄積に混ざる。**
    /// **4種すべて**を回す（1種だけだと、理由で分岐する実装が緑を通る）。
    #[test]
    fn nothing_and_every_discard_reason_never_become_a_vote() {
        assert!(
            DriftOutcome::Nothing.vote().is_none(),
            "提案なしは票にならないこと"
        );
        for reason in DriftDiscardReason::ALL {
            assert!(
                DriftOutcome::Discarded(reason).vote().is_none(),
                "{} は票にならないこと",
                reason.label()
            );
        }
    }

    /// **票になる結末と、ならない結末が食い違わない。**
    ///
    /// 「票になるのは `applied` と `saturated` だけ」を、
    /// **結末の集計キーの側から**言い直したもの。
    /// 片方だけを緩める変異が、もう片方の期待値を動かさない形にしてある。
    #[test]
    fn only_the_two_outcomes_that_passed_the_gate_become_votes() {
        let voting = [
            apply_drift(&allocs(), suggest("パッション", DriftDelta::Up)),
            apply_drift(&allocs(), suggest("共感", DriftDelta::Up)),
        ];
        let silent = [
            DriftOutcome::Nothing,
            DriftOutcome::Discarded(DriftDiscardReason::Malformed),
            DriftOutcome::Discarded(DriftDiscardReason::MultipleLines),
            DriftOutcome::Discarded(DriftDiscardReason::UnknownAxis),
            DriftOutcome::Discarded(DriftDiscardReason::MissingAxis),
            DriftOutcome::Discarded(DriftDiscardReason::DuplicateAxis),
        ];

        let voting_labels: Vec<&str> = voting.iter().map(DriftOutcome::label).collect();
        assert_eq!(
            voting_labels,
            vec!["applied", "saturated"],
            "前提: 票になる側の結末が2種そろっていること"
        );
        for outcome in &voting {
            assert!(
                outcome.vote().is_some(),
                "{} は票になること",
                outcome.label()
            );
        }
        for outcome in &silent {
            assert!(
                outcome.vote().is_none(),
                "{} は票にならないこと",
                outcome.label()
            );
        }
    }

    /// **`0` でなかった採点だけが票になる**（ADR-0027 §決定1 / §12 の危険3）。
    ///
    /// # `0` まで票にすると、希少度の重みが一律に戻る
    ///
    /// 全軸採点では**毎ターン全軸が採点される**ので、
    /// 「票になった回数」が全軸で同じになる。ADR が名指しで
    /// 「数えるのは `0` でなかった回数」と書いた当のものである。
    ///
    /// **上げと下げの両方**を回す（片方だけだと向きを決め打つ実装が通る）。
    #[test]
    fn only_a_score_that_was_not_zero_becomes_a_vote() {
        for delta in DriftDelta::ALL {
            let scored = DriftScore::new(axis("パッション"), Some(delta));
            let vote = scored.vote().expect("0 でない採点は票になること");
            assert_eq!(vote.axis(), &axis("パッション"), "軸の帰属が違う");
            assert_eq!(vote.delta(), delta, "向きが提案どおりでない");
        }

        assert!(
            DriftScore::new(axis("ユーモア"), None).vote().is_none(),
            "0 と採点された軸が票になった"
        );
    }

    /// **採点表の集計キーは、結末の語彙と重ならない綴りを名乗る。**
    ///
    /// 綴りそのものは `every_outcome_label_is_distinct` が跨いで見張るが、
    /// **どの語であるか**はここが直書きで押さえる
    /// （重ならない別の語へ改名する変異は、あちらを素通りする）。
    #[test]
    fn a_read_scorecard_says_it_was_scored() {
        assert_eq!(DriftScoring::Scored(Vec::new()).label(), "scored");
        assert_eq!(DRIFT_SCORED_LABEL, "scored");
        for reason in DriftDiscardReason::ALL {
            assert_eq!(
                DriftScoring::Discarded(reason).label(),
                reason.label(),
                "捨てた採点表は理由の綴りをそのまま使うこと"
            );
        }
    }

    // ---------------------------------------------------------------------
    // 集計キーとして読み戻せる形（作業単位 B-40）
    // ---------------------------------------------------------------------

    use crate::aggkey::fault_in_aggregation_key;

    /// **結末のすべての値**（作業単位 B-40）。
    ///
    /// 破棄は**理由の数だけ**並べる——[`DriftOutcome::label`] は破棄のとき
    /// [`DriftDiscardReason::label`] へ委ねるので、綴りが理由ごとに違う。
    ///
    /// **⚠ 手書きの配列である。** ここから**値を落とす／2回並べる**のは
    /// [`every_aggregation_key_list_covers_its_type_exactly_once`] が
    /// 添字の突き合わせで赤にする（B-39 の `ALL_OUTCOMES` と同じ作法）。
    /// **⚠ 逆向き——値を増やしてここへ並べ忘れる——は赤にならない。**
    /// 止めるのは [`outcome_index`] の `match` の**コンパイル**だけである
    /// （実測と、できない理由は
    /// [`every_aggregation_key_list_covers_its_type_exactly_once`] の doc の表）。
    ///
    /// [`every_aggregation_key_list_covers_its_type_exactly_once`]:
    ///     every_aggregation_key_list_covers_its_type_exactly_once
    fn all_outcomes() -> Vec<DriftOutcome> {
        let mut outcomes = vec![
            DriftOutcome::Applied(applied("パッション", DriftDelta::Up)),
            DriftOutcome::Nothing,
            DriftOutcome::Saturated {
                axis: axis("共感"),
                delta: DriftDelta::Up,
            },
        ];
        outcomes.extend(DriftDiscardReason::ALL.map(DriftOutcome::Discarded));
        outcomes
    }

    /// **採点表のすべての値**（作業単位 B-40）。破棄は理由の数だけ並べる。
    ///
    /// `Scored` の綴りは [`DRIFT_SCORED_LABEL`]。**依頼の形によって
    /// [`DriftOutcome::label`] と [`DriftScoring::label`] のどちらかが
    /// 同じ欄に載る**ので、どちらも同じ検査を通す。
    fn all_scorings() -> Vec<DriftScoring> {
        let mut scorings = vec![DriftScoring::Scored(Vec::new())];
        scorings.extend(DriftDiscardReason::ALL.map(DriftScoring::Discarded));
        scorings
    }

    /// 結末の添字。**網羅の `match`。値が増えたらここでコンパイルが止まる。**
    fn outcome_index(outcome: &DriftOutcome) -> usize {
        match outcome {
            DriftOutcome::Applied(_) => 0,
            DriftOutcome::Nothing => 1,
            DriftOutcome::Saturated { .. } => 2,
            DriftOutcome::Discarded(reason) => 3 + discard_reason_index(*reason),
        }
    }

    /// 採点表の添字。**網羅の `match`。**
    fn scoring_index(scoring: &DriftScoring) -> usize {
        match scoring {
            DriftScoring::Scored(_) => 0,
            DriftScoring::Discarded(reason) => 1 + discard_reason_index(*reason),
        }
    }

    /// 破棄の理由の添字。**網羅の `match`。**
    fn discard_reason_index(reason: DriftDiscardReason) -> usize {
        match reason {
            DriftDiscardReason::Malformed => 0,
            DriftDiscardReason::MultipleLines => 1,
            DriftDiscardReason::UnknownAxis => 2,
            DriftDiscardReason::MissingAxis => 3,
            DriftDiscardReason::DuplicateAxis => 4,
            DriftDiscardReason::Truncated => 5,
        }
    }

    /// 向きの添字。**網羅の `match`。**
    fn delta_index(delta: DriftDelta) -> usize {
        match delta {
            DriftDelta::Up => 0,
            DriftDelta::Down => 1,
        }
    }

    /// 依頼の形の添字。**網羅の `match`。**
    fn form_index(form: DriftForm) -> usize {
        match form {
            DriftForm::Single => 0,
            DriftForm::All => 1,
        }
    }

    /// 注記の添字。**網羅の `match`。**
    fn note_index(note: DriftNote) -> usize {
        match note {
            DriftNote::None => 0,
            DriftNote::Copy => 1,
            DriftNote::Language => 2,
        }
    }

    /// 走らせ方の添字。**網羅の `match`。**
    fn mode_index(mode: StatDriftMode) -> usize {
        match mode {
            StatDriftMode::Off => 0,
            StatDriftMode::Shadow => 1,
            StatDriftMode::On => 2,
        }
    }

    /// 出どころの添字。**網羅の `match`。**
    fn source_index(source: DriftModeSource) -> usize {
        match source {
            DriftModeSource::Character => 0,
            DriftModeSource::Default => 1,
        }
    }

    /// 添字を昇順に並べ直す。**期待値は `0..n` を直書きで突き合わせる**
    /// ——一覧から組み立てると、一覧が縮んだときに期待値も一緒に縮む。
    fn sorted_indices(indices: impl IntoIterator<Item = usize>) -> Vec<usize> {
        let mut seen: Vec<usize> = indices.into_iter().collect();
        seen.sort_unstable();
        seen
    }

    /// **この7つの型の集計キーを、持ち主の名前つきで全部並べる**（作業単位 B-40）。
    ///
    /// 名前を添えるのは、**どの型の綴りで落ちたのかが失敗の文面に出る**ようにするため。
    /// `D-6` の「写しから1列だけが抜ける」形は、**他の列が守られているので
    /// 対照を1つ選ぶと当たってしまう。**
    ///
    /// # ⚠ 7つ目（[`StatDriftMode`]）は B-40 の是正で足した
    ///
    /// **着手時の作業単位は「6つ」と書いていたが、そちらが数え違いだった**
    /// ——[`StatDriftMode::label`] は同じファイルにある7つ目の集計キーで、
    /// **[`every_aggregation_key_is_readable_back_as_a_log_field`] の doc の表の欄に載る。**
    /// **`D-6` の「写しから1列だけが抜ける」形そのもの。**
    ///
    /// **⚠ ここには「抜けていた1列は件数の押さえ（`keys.len()`）でしか捕まらない」と
    /// 書いてあった——B-40 の是正で誤りと分かった。**
    /// **件数は「列が丸ごと落ちる」しか見ない。**
    /// **長さが一致する型に差し替えると件数は動かない**
    /// （`DriftDelta` ↔ `DriftForm` が 2、`DriftNote` ↔ `StatDriftMode` が 3）。
    /// **実測: この列を `DriftDelta::ALL` から作る変異が 1940 全緑・exit 0 を通った。**
    /// **いまの押さえは
    /// [`every_aggregation_key_is_readable_back_as_a_log_field`] の
    /// 持ち主ごとの綴りの突き合わせ**で、件数はその上に乗る。
    ///
    /// [`every_aggregation_key_is_readable_back_as_a_log_field`]:
    ///     every_aggregation_key_is_readable_back_as_a_log_field
    fn every_aggregation_key() -> Vec<(&'static str, &'static str)> {
        let mut keys: Vec<(&'static str, &'static str)> = Vec::new();
        keys.extend(
            all_outcomes()
                .iter()
                .map(|outcome| ("DriftOutcome", outcome.label())),
        );
        keys.extend(
            all_scorings()
                .iter()
                .map(|scoring| ("DriftScoring", scoring.label())),
        );
        keys.extend(DriftDiscardReason::ALL.map(|reason| ("DriftDiscardReason", reason.label())));
        keys.extend(DriftDelta::ALL.map(|delta| ("DriftDelta", delta.label())));
        keys.extend(DriftForm::ALL.map(|form| ("DriftForm", form.label())));
        keys.extend(DriftNote::ALL.map(|note| ("DriftNote", note.label())));
        keys.extend(StatDriftMode::ALL.map(|mode| ("StatDriftMode", mode.label())));
        // **8つ目**（作業単位 A-8）。変容の記録の出どころの欄と、変容の応答に載る。
        keys.extend(DriftModeSource::ALL.map(|source| ("DriftModeSource", source.label())));
        keys
    }

    /// **7つの集計キーが、どれもログの欄として読み戻せる綴りである**（作業単位 B-40）。
    ///
    /// 7つとも**変容のログの欄の値として載る**のに、
    /// B-40 より前の検査は `!label.is_empty()` だけだった。
    /// **接頭辞つきの綴り（`drift_…`）をここに書かないこと**
    /// ——`server::drift` の `the_drift_fields_never_collide_with_another_module` が
    /// 走査で落とす。
    ///
    /// | 型 | 載る欄 | 出しているところ |
    /// |---|---|---|
    /// | [`DriftOutcome`] / [`DriftScoring`] / [`DriftDiscardReason`] | 結末の欄 | `server::drift` の `record_run` |
    /// | [`DriftDelta`] | 向きの欄 | 同上 |
    /// | [`StatDriftMode`] | **走らせ方の欄** ＋ 起動の行の `stat_drift` | 同上 ／ `server::drift` の `record_skip` ／ `server::App::bind` |
    /// | [`DriftForm`] | 起動の行の依頼の形の欄 | `server::App::bind` |
    /// | [`DriftNote`] | 起動の行の腕の欄 | `server::App::bind` |
    ///
    /// **⚠ 「6つ」だった時期がある。** B-40 の着手時の作業単位が6つと数え、
    /// **[`StatDriftMode`] だけが抜けていた**（`D-6` の形。是正で足した）。
    ///
    /// 判定は [`crate::aggkey::fault_in_aggregation_key`]
    /// ——**作業単位 B-39 が `parse` の私有として書き、
    /// 2つ目の持ち主（ここ）が現れたので B-40 で引き上げた。写しは作らない。**
    /// **判定そのものの権威はあちらの2本**で、
    /// **植えた綴りを同じ本体に通す対照**（`15b` の作法）も、
    /// **拒む綴りを `crate::testlog` の読み手に実際に通して壊れるところを見せる方**も、
    /// リポジトリ全体で1つずつしか無い。
    ///
    /// # ⚠ ここが受け持つのは「広げすぎていないこと」の側だけ
    ///
    /// **いまの値はすべて前提を満たしている**ので、判定を広げても
    /// この検査は一度もその枝に入らない（CLAUDE.md `13c-3a`
    /// 「フィクスチャがその分岐が効く状態に一度も入らない」）。
    /// **この検査の存在理由は「これから足される値」**で、そこへ辿り着く道は
    /// [`every_aggregation_key_list_covers_its_type_exactly_once`] の `match` の
    /// **コンパイル停止**である。**⚠ 「型で強制する」とは書かない**
    /// ——強制できるのは「止まること」までで、
    /// **一覧に並べさせるところまでは強制できない**（あちらの doc の表と実測）。
    ///
    /// # ⚠ 一覧へ正直に並べたら、残る赤はこの1本だけだった（B-40 / m11c ＝ 是正後は m11d）
    ///
    /// 7つ目の [`DriftDiscardReason`]（綴り `"drift planted"` ＝空白入り）を
    /// **`ALL` まで含めて全部の一覧へ並べ、綴りを写した見張り番も一緒に緩めた**状態で、
    /// **1939 passed / 1 failed** ——赤はここ1本で、文面は
    /// `集計キーに使える形にすること: DriftOutcome::label() -> "drift planted"`。
    /// **持ち主の名前を添えてあるのは、この文面のため**である。
    ///
    /// **⚠ 名乗るのは「その綴りを最初に外へ出した型」**である。
    /// 上の例で綴りを決めているのは [`DriftDiscardReason::label`] だが、
    /// [`DriftOutcome::label`] が破棄のときそれへ委ねるので、
    /// **一覧の先頭にある結末の側で先に落ちる**（この doc を書いたときに
    /// `DriftDiscardReason` と書いて、実測で違っていた）。
    ///
    /// # ⚠ 綴りそのものはここで固定しない
    ///
    /// それは [`all_lists_every_discard_reason_exactly_once`] などの仕事である。
    /// 写すと、片方を直したときにもう片方が黙って古びる。
    ///
    /// [`every_aggregation_key_list_covers_its_type_exactly_once`]:
    ///     every_aggregation_key_list_covers_its_type_exactly_once
    /// [`all_lists_every_discard_reason_exactly_once`]:
    ///     all_lists_every_discard_reason_exactly_once
    #[test]
    fn every_aggregation_key_is_readable_back_as_a_log_field() {
        let keys = every_aggregation_key();
        assert_eq!(
            keys.len(),
            34,
            "並べる集計キーの数が変わった（結末 3+6 / 採点表 1+6 / 理由 6 / 向き 2 / 形 2 / 注記 3 / 走らせ方 3 / 出どころ 2）: {keys:?}"
        );

        // **持ち主ごとに、その持ち主の型の値から作られていること**（B-40 の是正）。
        //
        // **⚠ 件数だけでは足りない。** 上の `keys.len()` は
        // **「列が丸ごと落ちる」しか見ない**ので、**長さが一致する型どうしの取り違え**を
        // 素通りする（`DriftDelta` ↔ `DriftForm` が 2、`DriftNote` ↔ `StatDriftMode` が 3）。
        // **是正前の実測: `DriftForm` の列を `DriftDelta::ALL` から作る変異が 1940 全緑。**
        // **綴りの列そのもので突き合わせる。**
        for (owner, expected) in [
            (
                "DriftOutcome",
                all_outcomes()
                    .iter()
                    .map(DriftOutcome::label)
                    .collect::<Vec<_>>(),
            ),
            (
                "DriftScoring",
                all_scorings()
                    .iter()
                    .map(DriftScoring::label)
                    .collect::<Vec<_>>(),
            ),
            (
                "DriftDiscardReason",
                DriftDiscardReason::ALL
                    .map(DriftDiscardReason::label)
                    .to_vec(),
            ),
            (
                "DriftDelta",
                DriftDelta::ALL.map(DriftDelta::label).to_vec(),
            ),
            ("DriftForm", DriftForm::ALL.map(DriftForm::label).to_vec()),
            ("DriftNote", DriftNote::ALL.map(DriftNote::label).to_vec()),
            (
                "StatDriftMode",
                StatDriftMode::ALL.map(StatDriftMode::label).to_vec(),
            ),
            (
                "DriftModeSource",
                DriftModeSource::ALL.map(DriftModeSource::label).to_vec(),
            ),
        ] {
            assert_eq!(
                keys.iter()
                    .filter(|(who, _)| *who == owner)
                    .map(|(_, label)| *label)
                    .collect::<Vec<_>>(),
                expected,
                "{owner} の列が、その型の値から作られていない（落ちているか、別の型に差し替わっている）"
            );
        }

        for (owner, label) in keys {
            assert_eq!(
                fault_in_aggregation_key(label),
                None,
                "集計キーに使える形にすること: {owner}::label() -> {label:?}"
            );
        }
    }

    /// **並べた一覧が、7つの型の値をちょうど1回ずつ覆っている**（作業単位 B-40）。
    ///
    /// # ⚠ 言えること・言えないこと（**B-40 で実測した。書いてから測って書き直した**）
    ///
    /// | 変更 | どうなるか |
    /// |---|---|
    /// | **一覧から値を落とす／同じ値を2回並べる** | **ここが赤**（添字の集合が `0..n` からずれる） |
    /// | **値を増やす** | **`match` が網羅なので `--lib` のコンパイルが止まる**（素の `cargo build` は通る。実測 / m11a） |
    /// | **値を増やし、腕だけ足して一覧へ並べない** | **緑を通る。** 実測: 7つ目の [`DriftDiscardReason`] を足し、`label()` と2つの `match` に腕を入れ、`ALL` へ並べない状態で **1940 全緑・exit 0**（m11b） |
    ///
    /// **⚠ 「腕を足しただけでは済まない」と書いてはならない。**
    /// **安定版の Rust は enum の値を列挙できない**ので、
    /// **型・値レベルのアサートだけでは「一覧に並んでいること」を強制できない。**
    /// **B-39 の `all_outcomes_lists_every_value_exactly_once` も同じ性質を持つ**
    /// ——この限界は B-40 が持ち込んだものではなく、この作法そのものの限界である。
    ///
    /// # ⚠ 「手段が無い」ではない（**B-40 の是正で訂正。着手時の診断が誤っていた**）
    ///
    /// ここには「**強制する手段が無い**」と書いてあった。
    /// **レビュアーが実際に書いて測り、否定した**——閉じる手段は少なくとも2つある。
    ///
    /// | 手段 | 実測・状況 |
    /// |---|---|
    /// | **ソース走査** | `include_str!` で自分のソースを読み、文字列・文字リテラル・コメントを潰してから `enum X { … }` の値を数えて `X::ALL.len()` と突き合わせる（`tests/calibration_target.rs` の `code_without_comments` と同じ作法 ／ 植えたソースの対照つき）。**7つ目を足して `ALL` へ並べない変異の下で `left: 7  right: 6` の 1 赤・exit 101** |
    /// | **derive マクロ** | `strum::EnumIter` 等。走査なしで閉じるが**依存が増える** |
    ///
    /// **どちらも採らない判断は司令塔が下した**——走査は
    /// **7型のうち5型しか閉じない**（[`DriftOutcome`] / [`DriftScoring`] に `ALL` が無く、
    /// 一覧は `#[cfg(test)]` の [`all_outcomes`] / [`all_scorings`]）うえ、
    /// `15b`（走査の対照の自己成就）・`B-21d`（日本語の doc で走査が狂う）・
    /// `E-3`（観測の道具にも観測される側と同じだけ変異を当てる）を抱え込む。
    ///
    /// **⚠ 「原理的に無理」と書かないこと**（`0012b` の逆向き）。
    /// **読んだ人が、探すのをやめる。**
    ///
    /// # それでも既存の `all_lists_every_*_exactly_once` の代わりにはならない（**止まる場所が違う**）
    ///
    /// あちらの `match` は**綴りを固定する節**にあり、腕を足す人は綴りの表へ案内される。
    /// **ここの `match` は B-40 の節にあり、[`every_aggregation_key`] の隣にある**
    /// ——`B-38`「止まる場所ではなく、名指しする直す先を比べること」。
    /// **一覧へ並べさえすれば、[`every_aggregation_key_is_readable_back_as_a_log_field`] が
    /// 新しい綴りを必ず1回通す**（m11c で実測。残った赤はそれ1本。
    /// **B-40 の是正で件数が 29 → 32 になったので、m11c は置換件数の門で止まり、
    /// 期待値を当て直した m11d で測り直した——結果は同じ 1939 / 1**）。
    ///
    /// [`every_aggregation_key_is_readable_back_as_a_log_field`]:
    ///     every_aggregation_key_is_readable_back_as_a_log_field
    #[test]
    fn every_aggregation_key_list_covers_its_type_exactly_once() {
        assert_eq!(
            sorted_indices(all_outcomes().iter().map(outcome_index)),
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            "DriftOutcome を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(all_scorings().iter().map(scoring_index)),
            vec![0, 1, 2, 3, 4, 5, 6],
            "DriftScoring を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(DriftDiscardReason::ALL.map(discard_reason_index)),
            vec![0, 1, 2, 3, 4, 5],
            "DriftDiscardReason を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(DriftDelta::ALL.map(delta_index)),
            vec![0, 1],
            "DriftDelta を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(DriftForm::ALL.map(form_index)),
            vec![0, 1],
            "DriftForm を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(DriftNote::ALL.map(note_index)),
            vec![0, 1, 2],
            "DriftNote を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(StatDriftMode::ALL.map(mode_index)),
            vec![0, 1, 2],
            "StatDriftMode を並べ忘れているか、同じ値を2回並べている"
        );
        assert_eq!(
            sorted_indices(DriftModeSource::ALL.map(source_index)),
            vec![0, 1],
            "DriftModeSource を並べ忘れているか、同じ値を2回並べている"
        );
    }
}
