//! **変容の集約規則**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md)
//! §決定1〜3・§決定5・§9・**訂正 2〜4** / 作業単位 16e-b・**16g**）。
//! **I/O を持たない純粋関数のみ。**
//!
//! # 何を決める層なのか
//!
//! `parse::drift` が読んだ**1ターンぶんの採点**（軸ごとの `+1` / `-1` / `0`）を、
//! **数値が動くかどうか**へ落とす。**モデルの判断と、値の適用を分けた層**である
//! （ADR-0027 §決定7「『なし』はモデルではなく閾値が出す」）。
//!
//! | 節 | 規則 | ここでの実装 |
//! |---|---|---|
//! | §決定1・**訂正 3** | 票の重みは [`DRIFT_VOTE_WEIGHT_COEFFICIENT`] `/ √(その軸の「0 でなかった」回数)` | [`DriftLedger::absorb`] |
//! | §決定2・§決定3・**訂正 4** | **`0` と採点された軸／選ばれなかった軸は薄れる** | 同上 |
//! | §決定5 | 下端は `0`（[`DRIFT_FLOOR`]） | [`DriftLedger::absorb`] の飽和 |
//! | §9・**訂正 2** | 1段の費用は**尺度で正規化**したうえで、**中点を基準に、上げは高い値ほど高く、下げは高い値ほど安い** | [`DriftTuning::step_cost_of`] |
//!
//! # **票の重みが `1/n` ではなく `0.1/√n` である理由**（ADR-0027 訂正 3）
//!
//! `1/n` の合計は**調和級数なので対数でしか伸びない。**
//! **貯まる速さが費用に届く速さを下回った時点で、その軸は恒久的に止まる**
//! ——16f / 腕H の `自信` は**上げ 53 票を集めながら 60 ターン動かなかった**
//! （累積 `0.911` に対し費用 `1.000`）。
//!
//! `1/√n` の合計は `2√n` で**発散する**ので、**係数を下げても凍らない。遅くなるだけ。**
//! **だから `K` が線形の速さのつまみになる。**
//!
//! # **薄れる量の重みは、依頼の形で違う**（違うのは**割る数だけ**）
//!
//! | 形 | 薄れる軸 | 薄れる重み |
//! |---|---|---|
//! | [`DriftForm::Single`] | 選ばれなかった **N−1 軸** | **その回に立った票の重みの合計 ÷ (N−1)** |
//! | [`DriftForm::All`] | **`0` と採点された軸**（0〜N 軸） | **その回に立った票の重みの合計 ÷ N** |
//!
//! **どちらも「その回に配られた票を、受け取らなかった軸が山分けする」という
//! 一つの規則である**（ADR-0027 訂正 4）。**割る数だけが形ごとに違う**
//! ——`Single` は薄れる軸が必ず `N−1` 本、`All` は 0〜N 本で可変なので `N` で割る。
//!
//! **`Single` は §決定3 の式そのもの**である——あの形では票が高々1つなので、
//! 「立った票の重みの合計」は「選ばれた軸の1票の重み `w`」に等しい。
//!
//! ## ⚠ **`All` は 16g まで定数 `1 ÷ N` だった**
//!
//! **票の重みが `n` とともに減るのに薄れが定数**だと、
//! **回数が増えるほど薄れが相対的に強くなる。**
//! 出荷済みの規則（`1/n` / λ=1 / `1/N`）でも **1008 ターンで `8 0 0 0 2`**
//! ——**5軸のうち4軸が 0 に落ちて戻らない**（ADR-0027 訂正 4 の実測）。
//! **16f が 72 ターンしか回さなかったので見えていなかった。**
//!
//! **票の重みに比例させると、この崩れが消える**（同じ 1008 ターンで `8 3 5 5 9` / 飽和 0）。
//! 見張り番は `the_thousand_turn_trajectory_matches_the_table_the_other_tool_produced` と
//! `the_all_form_fade_is_proportional_to_the_votes_cast_this_turn`。
//!
//! **帰結が1つある** ——**票が1つも立たなかった回は、どちらの形でも何も薄れない。**
//! 配られていない票は、失いようがない。
//!
//! # ⚠ **費用は絶対に `0` にならない**（傾きの上限が `2` 未満である理由）
//!
//! `費用(v, 上げ) = 基準 × (1 + 傾き × (v − 尺度/2) / 尺度)` に **`v = 0`** を入れると
//! **`基準 × (1 − 傾き/2)`** になる。**傾き `2` で厳密に `0`。**
//!
//! **費用が `0` だと、累積が `0` のままでも段が動く** ——
//! **値 0 の軸が、会話で何も言われなくても毎ターン 1 に上がる。**
//! 運用者が「語らない」と宣言した軸が、変容が走っている限り黙って現れることになる。
//!
//! > **`傾き < 2` は尺度に依らない普遍の条件である**（中点が常に `尺度/2` なので、
//! > `(0 − 尺度/2)/尺度` は尺度によらず `−1/2`）。
//! > だから [`MAX_DRIFT_SLOPE`] は **`2` 未満**でなければならない。
//!
//! **これは 16e-b の実装中に見つかった ADR 側の欠陥**で、
//! ADR-0027 の「訂正（2026-08-24）」節が既定を `2` → **`1.5`** に下げている。
//! **最終値は変わらない**（初期 5 も初期 0 も同じ配分に着く）。
//! **初期 0 の飽和が `0` → `1` になるのは正しい振る舞い**
//! ——値 0 の軸に `-1` が来て、下端で止まる。
//!
//! # ⚠ **入口は傾きだけではない**（レビュー指摘C / 16e-b）
//!
//! **`K` の側からも同じ状態に入れた。** `K = 5e-324`（最小の亜正規数）は
//! `is_finite()` も `> 0.0` も通るのに、**費用がアンダーフローして `0`** になる。
//! いまは [`DriftTuning::new`] が**「いちばん安い段が表現できること」**まで見る。
//!
//! **いちばん安い段は「いちばん高い尺度 × 上限の傾き」にある**
//! ——`K × (既定尺度/`[`StatScale::MAX`]`) × (1 − `[`MAX_DRIFT_SLOPE`]`/2)` = `K × 0.01`
//! （ADR-0027 訂正 2 で尺度の正規化が入ったぶん、16e-b の `K × 0.1` より 1 桁下がった）。
//!
//! 見張り番は `the_cost_never_reaches_zero_for_any_scale_or_slope` と
//! `a_step_cost_too_small_to_price_the_cheapest_step_is_refused`。
//!
use super::drift::{DRIFT_FLOOR, DriftChange, DriftDelta, DriftForm};
use super::stat::{StatAxis, StatScale, StatValue};

/// **薄れる強さ `λ`** の既定値（ADR-0027 §決定8 / **16j**）。
///
/// # 出どころは合成データではなく、実データ2本の当て直しである
///
/// **16h（359 ターン）と 16i（349 ターン）の実データの提案列に `λ × K` を 16 通り当て、
/// 5つの条件で採点して選んだ点**（ADR-0027「16j」節）——
/// **飽和 0 / 上げも下げも出る / 段数 4〜12 / 端に貼り付いた軸が無い / 分化している。**
///
/// **`λ = 1` / `K = 1` は、合成した提案列（腕C）で釣り合わせた出発点**で、
/// **実データでは 359 ターンで 3 段・下げ 0 だった**（16h）。
///
/// # ⚠ **`λ` は「薄れの強さ」ではなく「`0` と採点された回に効く強さ」である**
///
/// 薄れは `0` と採点された軸にしか効かない（§決定2）ので、
/// **`0` の率が違う軸の間では、同じ数が違う意味になる**
/// ——16j §3 の実測で **`自信` は 1%、`ユーモア` は 80%**（**80 倍**）。
/// **だから単一の既定値で全部の軸の組に合わせることはできない。**
/// **既定は「軸の組を選び直すまでの出発点」**であって、
/// 運用では 16j の手順（記録専用モードで 60〜100 ターン回して当て直す）で合わせる。
pub const DEFAULT_DRIFT_DECAY: f64 = 0.5;

/// **値付けの傾き**の既定値（ADR-0027「訂正（2026-08-24）」）。
///
/// # ⚠ **`2` ではない**
///
/// ADR は当初 `2` と書いていたが、**傾き `2` は `v = 0` の上げの費用を厳密に `0` にする**
/// （モジュール doc）。**下げたのは 16e-b。** 最終値は変わらない。
pub const DEFAULT_DRIFT_SLOPE: f64 = 1.5;

/// **1段の費用 `K`** の既定値（ADR-0027 §決定8 / **16j**）。
///
/// # 出どころは [`DEFAULT_DRIFT_DECAY`] と同じ当て直しである
///
/// **16i の軸の組で 5/5 になった点のうち、上げと下げが両方出て段数が中央のもの**
/// （ADR-0027「16j」節の表——`λ = 0.5` / `K = 0.3` で **着地点 6 6 2 3 3 / 上げ 3 / 下げ 8 / 飽和 0**）。
///
/// **`on` の腕でも確かめてある**（16l）——同じ設定で 366 ターン回し、
/// **予言を先に書いてから走らせて全軸 ±1。飽和は両方 0。**
/// **平坦な台本でも 5/5**（16n / 72 ターン / 上げ 2・下げ 2 で釣り合う）。
///
/// # ⚠ **`K` は「速さ」と「下がれるかどうか」を同時に決める**
///
/// **静かにするほど下向きが消える**（§決定7 の置き換え）——
/// 薄れはじわじわ効く滴なので、**`K` が大きいと溜まりきる前に次の票が来て流される。**
/// `1.0` → `0.3` は「速くした」だけではなく、**下げが出る側へ寄せた**変更でもある。
pub const DEFAULT_DRIFT_STEP_COST: f64 = 0.3;

/// **票の重みの係数**（ADR-0027「訂正 3」）。重みは `係数 / √(その軸の「0 でなかった」回数)`。
///
/// # ⚠ **つまみにしない**
///
/// [`DriftTuning`] の3つと違って、**これは設定で振る値ではない。**
/// **速さのつまみは `K`** である（訂正 3 の表——尺度は粒度、`K` と重みの形が速さ）。
/// 係数まで外へ出すと、**同じ「遅さ」を2つの値で作れてしまい、どちらで測ったのかが読めなくなる。**
///
/// # なぜ `0.1` なのか
///
/// **一晩（72ターン）で 2 段・200ターン超で 4 段・1000ターンで 9 段**という軌跡に着く値
/// （ADR-0027 訂正 3 / 訂正 4 の実測表）。
/// 「人間の本質はそう簡単には変わらない」という要件から逆算した数であって、
/// **理論から出た値ではない。**
pub const DRIFT_VOTE_WEIGHT_COEFFICIENT: f64 = 0.1;

/// **傾きの上限。`2` 未満でなければならない。**
///
/// # なぜ `2` そのものが上限になれないのか
///
/// `費用(0, 上げ) = K × (1 − 傾き/2)` なので、**傾き `2` で費用が厳密に `0`** になる。
/// 費用 `0` の段は「累積が `0` のままでも動く」——**言われていない軸が勝手に上がる**
/// （モジュール doc の ⚠）。**`傾き > 2` なら費用は負**になり、
/// 「累積がいくら負でも上がる」形になって**向きの意味そのものが壊れる。**
///
/// **この条件は尺度に依らない**（中点が常に `尺度/2` なので、`(0 − 尺度/2)/尺度` は
/// 尺度によらず `−1/2`）。見張り番の
/// `the_cost_never_reaches_zero_for_any_scale_or_slope` が尺度の全域で確かめる。
///
/// # なぜ `1.8` なのか
///
/// **「いちばん安い段でも `K` の1割は掛かる」**ところに置いた
/// ——`1 − 傾き/2 ≥ 0.1` を解くと `傾き ≤ 1.8`。
///
/// `2` に限りなく近い上限（`1.999…`）でも「費用は正」は成り立つが、
/// **1段が `K` の 1/2000 になる**ので、**端では実質いくらでも段が動く。**
/// 上限は「表現できる境界」ではなく「**測りたい範囲**」として置く
/// （`StatScale::MAX` が「推奨ではなく表現できる範囲」と書いているのとは逆の判断で、
/// **こちらは端の1段が壊れる向き**なので狭い方へ倒す）。
///
/// 既定（[`DEFAULT_DRIFT_SLOPE`] = 1.5、いちばん安い段が `K × 0.25`）**とは別の値**にしてある
/// ——同値にすると「上限を既定で代用している」実装が観測できない。
pub const MAX_DRIFT_SLOPE: f64 = 1.8;

/// **上限が `2` 未満であることを、コンパイル時に確かめる。**
///
/// `2` 以上にすると `費用(0, 上げ) = K × (1 − 傾き/2)` が `0` 以下になり、
/// **言われていない軸が動く**（[`MAX_DRIFT_SLOPE`] の doc）。
/// **テストではなくビルドで止める**——ここは尺度にも入力にも依らない普遍の条件で、
/// **満たさない値を書いた時点で間違い**だからである。
const _: () = assert!(MAX_DRIFT_SLOPE < 2.0);

/// **1ターン・1軸あたりに許す段数の上限。**
///
/// 飽和の段は値を動かさずに累積だけを消費する（ADR-0024 §5 の「上げようとし続けている」を
/// 累積の側にも残すため）ので、**費用が安いほど繰り返しが伸びる。**
/// 費用の下限は `K × 0.01`（[`MAX_DRIFT_SLOPE`] × [`StatScale::MAX`] / ADR-0027 訂正 2）なので
/// **累積 1 につき高々 100 段**だが、上限が無ければ「終わらない入力」を書ける。
/// `while` を素で回すと**終わらない入力を書ける**ので、必ず上限で切る
/// （CLAUDE.md「本番コードパスに panic を置かない」の裏返しで、**止まらない**のも同じくらい悪い）。
///
/// 実運用でここに当たることは無い——1ターンに増える累積は最大でも
/// **[`DRIFT_VOTE_WEIGHT_COEFFICIENT`]**（＝その軸の初回の票 / `0.1`）で、
/// **`K` が既定値（`0.3`）なら高々 1 段**
/// （既定尺度の `費用(0, 上げ) = 0.075` を払うと残りは `0.025`、
/// 次の `費用(1, 上げ) = 0.12` に届かない）。
/// **`1024` は「安い `K` を設定で振ったとき」の歯止め**であって、既定の余裕ではない。
const MAX_STEPS_PER_TURN: usize = 1024;

/// 集約のつまみが受け付けられない理由。`Display` は**定数文字列**（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DriftTuningError {
    /// 薄れる強さ `λ` が有限でない、または負。
    #[error("性格の変容の減衰の指定が不正です")]
    Decay,

    /// 値付けの傾きが有限でない、負、または [`MAX_DRIFT_SLOPE`] を超える。
    #[error("性格の変容の値付けの傾きの指定が不正です")]
    Slope,

    /// 1段の費用 `K` が有限でない、または `0` 以下。
    #[error("性格の変容の1段の費用の指定が不正です")]
    StepCost,
}

/// **集約規則のつまみ**（ADR-0027 §決定8 の ⚠「定数にして、設定で振れる形にすること」）。
///
/// # `f64` を3つ並べた構造体にしている理由
///
/// **3つとも「同じ単位の別の数」ではない**（`λ` は薄れる強さ、傾きは費用曲線、`K` は費用の基準）。
/// 引数として3つ並べて持ち回ると、**呼び出し側で順番を取り違えても型が通る。**
/// [`DriftTuning::new`] を通した値しか作れない形にして、
/// **範囲の検証を1箇所**に閉じてある。
///
/// `Eq` / `Hash` は持たない（`f64` なので）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftTuning {
    decay: f64,
    slope: f64,
    step_cost: f64,
}

impl Default for DriftTuning {
    /// ADR-0027 §決定8（**16j が置き換えた値**）。**`λ = 0.5` / 傾き `1.5` / `K = 0.3`。**
    fn default() -> Self {
        Self {
            decay: DEFAULT_DRIFT_DECAY,
            slope: DEFAULT_DRIFT_SLOPE,
            step_cost: DEFAULT_DRIFT_STEP_COST,
        }
    }
}

impl DriftTuning {
    /// 唯一のコンストラクタ。**範囲の外は拒否する**（黙って既定へ落とさない）。
    ///
    /// | つまみ | 受ける範囲 | 外したときに起きること |
    /// |---|---|---|
    /// | `decay`（`λ`） | 有限で `0` 以上 | 負にすると**言及されなかった軸が上がる** |
    /// | `slope`（傾き） | `0..=`[`MAX_DRIFT_SLOPE`]（**`2` 未満**） | `2` 以上にすると**費用が `0` 以下**になり、言われていない軸が動く |
    /// | `step_cost`（`K`） | 有限で、**いちばん安い段が `0` にならない大きさ** | `0` にすると**毎ターン全軸が端まで飛ぶ**。亜正規数だと**アンダーフローして同じことが起きる** |
    ///
    /// # ⚠ **上端には門が無い。意図的である**（作業単位 16g）
    ///
    /// **小さい側と大きい側は、危険の質が逆である。**
    ///
    /// * **費用 `0` は「止まる」ではなく「無償で動く」** ——
    ///   言われていない軸が勝手に上がる。**振る舞いが壊れる。**
    /// * **費用 `inf` は「止まる」だけ**で、**`K = 1e6` でも同じことが起きる。**
    ///   **連続的に遅くなっていく先の端なので、そこに引く線が無い。**
    ///
    /// **`K > f64::MAX / 10 ≒ 1.797e307` では費用が全域で `inf`** になる
    /// （[`DriftTuning::step_cost_of`] は `K × 既定尺度 / 尺度` と掛けてから割る）が、
    /// **段が一切動かないだけで `NaN` も飽和も出ない。**
    /// 見張り番は `a_step_cost_so_large_that_the_price_overflows_simply_never_moves`。
    pub fn new(decay: f64, slope: f64, step_cost: f64) -> Result<Self, DriftTuningError> {
        if !decay.is_finite() || decay < 0.0 {
            return Err(DriftTuningError::Decay);
        }
        if !slope.is_finite() || !(0.0..=MAX_DRIFT_SLOPE).contains(&slope) {
            return Err(DriftTuningError::Slope);
        }
        if !step_cost.is_finite() || step_cost <= 0.0 {
            return Err(DriftTuningError::StepCost);
        }
        // **いちばん安い段が表現できること**（レビュー指摘C / 16e-b）。
        //
        // 傾きの側はビルドで止めた（`MAX_DRIFT_SLOPE` の doc）が、
        // **`K` の側から同じ状態に入れた** ——`K = 5e-324`（最小の亜正規数）は
        // `is_finite()` も `> 0.0` も通るのに、
        // **`K × (1 − 傾き/2)` がアンダーフローして `0` になる。**
        // 費用 `0` の段は「累積が `0` のままでも動く」ので、
        // **言われていない軸が勝手に上がる**——潰したはずの「無償の1段」が復活する。
        //
        // **上限の傾き・いちばん高い尺度で見る。** そこがいちばん安いので、
        // ここを通れば**どの傾き・どの尺度でも費用は正**である
        // （`MAX_DRIFT_SLOPE` / `StatScale::MAX` を使うのは、傾きの側の門と対称の形にするため）。
        //
        // # ⚠ **尺度の正規化が入って、いちばん安い段はさらに安くなった**（ADR-0027 訂正 2）
        //
        // 基準が `K × (既定尺度/尺度)` になったので、
        // **いちばん安い段は `K × (10/100) × (1 − 1.8/2)` = `K × 0.01`**。
        // **`K × (1 − 傾き上限/2)` だけで見ると、`K × 0.1` は正でも `K × 0.01` が `0` になる
        // 領域**（尺度 100 のキャラクター）が素通りする。
        if step_cost
            * (f64::from(StatScale::DEFAULT.get()) / f64::from(StatScale::MAX))
            * (1.0 - MAX_DRIFT_SLOPE / 2.0)
            <= 0.0
        {
            return Err(DriftTuningError::StepCost);
        }
        Ok(Self {
            decay,
            slope,
            step_cost,
        })
    }

    /// 薄れる強さ `λ`（ADR-0027 §決定3）。
    #[must_use]
    pub const fn decay(self) -> f64 {
        self.decay
    }

    /// 値付けの傾き（ADR-0027 §9）。**`0` なら値付けは効かない**（費用は常に `K`）。
    #[must_use]
    pub const fn slope(self) -> f64 {
        self.slope
    }

    /// 1段の費用の基準 `K`（ADR-0027 §決定7）。
    #[must_use]
    pub const fn step_cost(self) -> f64 {
        self.step_cost
    }

    /// **1段動かす費用**（ADR-0027 §9 / **訂正 2** で尺度による正規化が入った）。
    ///
    /// ```text
    /// 基準       = K × (既定尺度 / 尺度)          既定尺度は StatScale::DEFAULT = 10
    /// 費用(v, 上げ) = 基準 × (1 + 傾き × (v − 尺度/2) / 尺度)
    /// 費用(v, 下げ) = 基準 × (1 − 傾き × (v − 尺度/2) / 尺度)
    /// ```
    ///
    /// # ⚠ **`K` は「1段の費用」ではない。「尺度を10段ぶん動かす費用」である**
    ///
    /// 薄れ（`λ × (v/尺度) × …`）も傾き（`(v − 尺度/2)/尺度`）も**比なので尺度に依らない**が、
    /// **1段の費用だけが尺度に依っていた**（ADR-0027 訂正 2）。
    /// **`0-100` では範囲の1割を動くのに 10 段＝10 倍の証拠が要る**ので、
    /// **同じ会話でキャラクターがほとんど動かない**（実測で全軸が 46〜53% に張り付き、分化が消える）。
    ///
    /// **尺度 10 の振る舞いは1バイトも変わらない**（`10/10 = 1`）。
    /// 見張り番は `a_step_costs_the_same_share_of_the_range_at_every_scale` と
    /// `the_same_conversation_moves_the_same_share_of_the_range_at_scale_one_hundred`。
    ///
    /// # ⚠ **中点を基準に引くこと**（ADR-0027 §9 の ⚠）
    ///
    /// `費用(下げ) = K × (1 + 傾き × (尺度 − v)/尺度)` と書くと
    /// **中点で上げも下げも `K × 1.5`** になり、**どちらも高くなるだけで非対称にならない。**
    /// ADR は「一度その形で書いて**下げが 0 件になった**」と実測を残している。
    ///
    /// 見張り番は `the_cost_is_symmetric_only_at_the_midpoint` と
    /// `a_symmetric_cost_would_lose_every_downward_step`。
    #[must_use]
    pub fn step_cost_of(self, value: u32, scale: u32, delta: DriftDelta) -> f64 {
        if scale == 0 {
            // **`StatScale::MIN` は 1 なので到達しない**が、割り算の前に必ず塞ぐ。
            return self.step_cost;
        }
        let scale = f64::from(scale);
        let midpoint = scale / 2.0;
        let tilt = self.slope * (f64::from(value) - midpoint) / scale;
        // **既定尺度で割り戻す**（ADR-0027 訂正 2）。`10/10 = 1` なので既定尺度は素通り。
        let base = self.step_cost * f64::from(StatScale::DEFAULT.get()) / scale;
        match delta {
            DriftDelta::Up => base * (1.0 + tilt),
            DriftDelta::Down => base * (1.0 - tilt),
        }
    }
}

/// 帳簿が組み立てられない理由。`Display` は**定数文字列**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DriftLedgerError {
    /// 採点の件数が軸の本数と違う。
    ///
    /// **黙って短い方に合わせない。** 合わせると
    /// **採点が1つずれた状態**（軸 A の採点が軸 B に入る）が静かに通る
    /// ——CLAUDE.md「抜けるのはいつも帰属」。
    #[error("性格の変容の採点の件数が軸の本数と合いません")]
    Mismatched,
}

/// **軸1本ぶんの帳簿。**
///
/// | 持つもの | どこから来るか |
/// |---|---|
/// | 軸名 | **保存済みの綴り**（`store::stat_allocs`） |
/// | 現在値 | 同上。**尺度を覚えている**（ADR-0015） |
/// | 累積 | 累積の表（`Store::drift_charges`。前のターンまでの持ち越し） |
/// | 「0 でなかった」回数 | 票の件数（`Store::recent_drift_votes`。**票は非ゼロの採点にだけ立つ**） |
///
/// # ⚠ **表の名前をここに綴らない**
///
/// 票の表への破壊的な文を探す走査（`store::drift` の
/// `no_statement_updates_or_deletes_a_vote`）は **`src/**` の生ソース**を読むので、
/// **綴った瞬間に許可されていない形として落ちる**
/// （`store::memory::table_word` と同じ制約）。**関数名で指すこと。**
///
/// # ⚠ **回数は「提案された回数」ではない**（ADR-0027 §12 の危険3）
///
/// 全軸採点では**毎ターン全軸が採点される**ので、
/// 「提案された回数」で数えると**全軸の回数が同じになり、重みが一律に戻る。**
/// 数えるのは **`0` でなかった回数**である。
#[derive(Debug, Clone, PartialEq)]
pub struct DriftAxisLedger {
    axis: StatAxis,
    value: StatValue,
    charge: f64,
    scored: u32,
}

impl DriftAxisLedger {
    /// 帳簿の1行を組む。
    ///
    /// **`charge` が有限でなければ `0` に倒す。** DB の `REAL` には
    /// `sqlite3` から直接 `9e999` を書けるので、**読み出し側だけを信じない**
    /// （`store` 側でも弾くが、写しではなく**二重の底**として置く）。
    #[must_use]
    pub fn new(axis: StatAxis, value: StatValue, charge: f64, scored: u32) -> Self {
        Self {
            axis,
            value,
            charge: if charge.is_finite() { charge } else { 0.0 },
            scored,
        }
    }

    /// 軸名。**保存済みの綴り。**
    #[must_use]
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 現在値。**尺度を覚えている。**
    #[must_use]
    pub const fn value(&self) -> StatValue {
        self.value
    }

    /// 累積（ADR-0026 §決定2 の「溜めて消費する」）。
    #[must_use]
    pub const fn charge(&self) -> f64 {
        self.charge
    }

    /// **`0` でなかった回数**（＝票の件数）。
    #[must_use]
    pub const fn scored(&self) -> u32 {
        self.scored
    }
}

/// **1ターンぶんの集約の結果。**
///
/// # 「動いた」と「端で動けなかった」を分けて持つ
///
/// ADR-0024 §5 の「クランプせずに飽和を数える」を、集約の側でも保つ。
/// 潰すと「毎ターン上げようとしているが上端で止まっている」と
/// 「そもそも累積が費用に届いていない」が**区別できなくなる。**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriftTurnEffect {
    changes: Vec<DriftChange>,
    saturations: Vec<(StatAxis, DriftDelta)>,
    ups: usize,
    downs: usize,
}

impl DriftTurnEffect {
    /// **実際に値が動いた「段」の数**（上げ）。
    ///
    /// # [`DriftTurnEffect::changes`] の件数とは別物である
    ///
    /// 費用が安い領域では**1軸が1ターンに2段以上動く**ので、
    /// 「動いた軸の数」では ADR-0027 の表の `上げ` / `下げ` を再現できない
    /// （あちらは**段**を数えている）。**飽和した段はここに入らない。**
    #[must_use]
    pub const fn ups(&self) -> usize {
        self.ups
    }

    /// **実際に値が動いた「段」の数**（下げ）。
    #[must_use]
    pub const fn downs(&self) -> usize {
        self.downs
    }

    /// **このターンに値が動いたか。** ADR-0027 §決定7 の「なし」の裏返し。
    ///
    /// **飽和は「動いた」に数えない**——値は変わっていない。
    #[must_use]
    pub const fn moved(&self) -> bool {
        self.ups > 0 || self.downs > 0
    }
    /// 実際に動いた軸。**1軸につき高々1件**で、`before` は**このターンの開始時の値**。
    ///
    /// # 段が2つ以上でも1件にまとめる
    ///
    /// 費用が `K` より安い領域（低い値の上げ）では**1ターンに2段以上動く。**
    /// 段ごとに `DriftChange` を出すと、`store` の
    /// `WHERE value = ?`（読んだ時点の値）が**2件目以降で必ず外れる。**
    #[must_use]
    pub fn changes(&self) -> &[DriftChange] {
        &self.changes
    }

    /// **端で動かせなかった段**（軸と向き）。**段ごとに1件。**
    #[must_use]
    pub fn saturations(&self) -> &[(StatAxis, DriftDelta)] {
        &self.saturations
    }
}

/// **キャラクター1体ぶんの帳簿**（ADR-0027 §決定1〜3・§9）。**純粋。**
///
/// # 依頼の形を持つ理由
///
/// **薄れる重みが形で違う**（モジュール doc の表）。
/// 引数で毎回渡す形にすると、**呼び出し側が形を取り違えても通る**
/// ——帳簿は1回の走らせ方につき1つなので、作るときに一度だけ決める。
#[derive(Debug, Clone, PartialEq)]
pub struct DriftLedger {
    form: DriftForm,
    tuning: DriftTuning,
    axes: Vec<DriftAxisLedger>,
}

impl DriftLedger {
    /// 帳簿を組む。**軸の並びは、依頼に載せた順と同じでなければならない**
    /// （[`DriftLedger::absorb`] は採点を**位置で**突き合わせる）。
    #[must_use]
    pub fn new(form: DriftForm, tuning: DriftTuning, axes: Vec<DriftAxisLedger>) -> Self {
        Self { form, tuning, axes }
    }

    /// 帳簿の中身（更新後の累積と値を読み戻すため）。
    #[must_use]
    pub fn axes(&self) -> &[DriftAxisLedger] {
        &self.axes
    }

    /// **1ターンぶんの採点を吸う。** 並びは [`DriftLedger::new`] に渡した軸と同じ。
    ///
    /// 1. **票を吸う** —— `0` でなかった軸は回数を1つ増やし、
    ///    重み [`DRIFT_VOTE_WEIGHT_COEFFICIENT`] `/ √回数`（**その票を含めて数える**）を
    ///    向きの符号つきで累積へ足す。
    /// 2. **薄れる** —— `0` と採点された軸（`single` では選ばれなかった軸）から
    ///    `λ × (現在値 / 尺度) × 薄れる重み` を引く。**現在値は動かす前の値。**
    /// 3. **消費する** —— 累積が費用に達している間、1段ずつ動かして費用ぶんを消す。
    ///    端では**値を動かさずに費用だけ消す**（飽和）。
    ///
    /// # 順番が結果を変える
    ///
    /// **票を全部吸ってから薄れる。** 薄れる重みは「その回に立った票の重みの合計」から
    /// 決まる（**両方の形で** / ADR-0027 訂正 4）ので、**票を吸う前には決まらない。**
    /// そして**段を動かすのは最後**——先に動かすと、
    /// 薄れる量が「動いた後の値」に比例してしまう。
    pub fn absorb(
        &mut self,
        scores: &[Option<DriftDelta>],
    ) -> Result<DriftTurnEffect, DriftLedgerError> {
        if scores.len() != self.axes.len() {
            return Err(DriftLedgerError::Mismatched);
        }

        // 1. 票を吸う。**重みはこの票を含めて数える**（ADR-0027 §決定1）。
        let mut vote_weight = 0.0_f64;
        for (score, axis) in scores.iter().zip(self.axes.iter_mut()) {
            let Some(delta) = score else { continue };
            axis.scored = axis.scored.saturating_add(1);
            let weight = DRIFT_VOTE_WEIGHT_COEFFICIENT / f64::from(axis.scored).sqrt();
            vote_weight += weight;
            axis.charge += match delta {
                DriftDelta::Up => weight,
                DriftDelta::Down => -weight,
            };
        }

        // 2. 薄れる（ADR-0027 §決定2 / §決定3）。**重みは形で違う**（モジュール doc）。
        let decay_weight = self.decay_weight(vote_weight);
        for (score, axis) in scores.iter().zip(self.axes.iter_mut()) {
            if score.is_some() {
                continue;
            }
            let scale = axis.value.scale().get();
            if scale == 0 {
                continue;
            }
            let ratio = f64::from(axis.value.get()) / f64::from(scale);
            axis.charge -= self.tuning.decay * ratio * decay_weight;
        }

        // 3. 消費する（ADR-0027 §決定7 / §9）。
        let mut effect = DriftTurnEffect::default();
        for axis in &mut self.axes {
            Self::spend(axis, self.tuning, &mut effect);
        }
        Ok(effect)
    }

    /// **薄れる重み。** 形ごとの根拠はモジュール doc の表。
    ///
    /// **どちらの形も「その回に配られた票を、受け取らなかった軸が山分けする」**
    /// （ADR-0027 訂正 4）。**違うのは割る数だけ。**
    /// だから**票が1つも立たない回は `0`** ——配られていない票は失いようがない。
    fn decay_weight(&self, vote_weight: f64) -> f64 {
        let count = self.axes.len();
        match self.form {
            // §決定3 の `w / (N−1)`。**票が高々1つ**の形なので、
            // 「立った票の重みの合計」は「選ばれた軸の1票の重み」に等しい。
            DriftForm::Single => {
                if count > 1 {
                    // **`u32` 経由で `f64` にする。** `as` を使うと、
                    // 桁が落ちたことが型からも読めなくなる。
                    let others = u32::try_from(count - 1).unwrap_or(u32::MAX);
                    vote_weight / f64::from(others)
                } else {
                    // 軸が1本しかなければ、薄れる相手が居ない。
                    0.0
                }
            }
            // **`Single` と同じ規則。割る数だけが違う**（ADR-0027 訂正 4）。
            // 薄れる軸は「`0` と採点された軸」で 0〜N 本と可変なので、割る数は `N`。
            DriftForm::All => {
                let all = u32::try_from(count).unwrap_or(u32::MAX);
                if all == 0 {
                    0.0
                } else {
                    vote_weight / f64::from(all)
                }
            }
        }
    }

    /// **累積を費用にぶつけて段を動かす。** 上げ側 → 下げ側の順に見る。
    ///
    /// # 上げと下げを `else if` にしない
    ///
    /// 片方の枝でしか見ないと、**上げ切って余った累積が負に振れた回**を
    /// 次のターンまで持ち越す形になる（`scratchpad/m16/all.awk` は2本の `while`）。
    /// **同じターンのうちに両方を見る。**
    fn spend(axis: &mut DriftAxisLedger, tuning: DriftTuning, effect: &mut DriftTurnEffect) {
        let scale = axis.value.scale();
        let before = axis.value;

        for _ in 0..MAX_STEPS_PER_TURN {
            let cost = tuning.step_cost_of(axis.value.get(), scale.get(), DriftDelta::Up);
            if axis.charge < cost {
                break;
            }
            axis.charge -= cost;
            if axis.value.get() >= scale.get() {
                // **クランプしない**（ADR-0024 §5）。費用だけ消えて値は動かない。
                effect.saturations.push((axis.axis.clone(), DriftDelta::Up));
                continue;
            }
            let Ok(next) = scale.value(axis.value.get().saturating_add(1)) else {
                break;
            };
            axis.value = next;
            effect.ups = effect.ups.saturating_add(1);
        }

        for _ in 0..MAX_STEPS_PER_TURN {
            let cost = tuning.step_cost_of(axis.value.get(), scale.get(), DriftDelta::Down);
            if axis.charge > -cost {
                break;
            }
            axis.charge += cost;
            // **`== DRIFT_FLOOR` に縮めない**（`domain::drift::apply_drift` と同じ理由）。
            #[expect(
                clippy::absurd_extreme_comparisons,
                reason = "下端が u32 の最小値でなくなった日に、この属性が警告になって気付ける"
            )]
            if axis.value.get() <= DRIFT_FLOOR {
                effect
                    .saturations
                    .push((axis.axis.clone(), DriftDelta::Down));
                continue;
            }
            let Ok(next) = scale.value(axis.value.get().saturating_sub(1)) else {
                break;
            };
            axis.value = next;
            effect.downs = effect.downs.saturating_add(1);
        }

        if axis.value.get() != before.get() {
            effect
                .changes
                .push(DriftChange::stepped(axis.axis.clone(), before, axis.value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::stat::{StatScale, fixture_scale};

    // =====================================================================
    // ADR-0027 の実測値を再現する（**この作業単位の唯一の外側の権威**）
    // =====================================================================

    /// **腕E の採点表**（`scratchpad/m16/scorecard.tsv` の 72 ターン × 5 軸）。
    ///
    /// 1文字が1軸の採点で、並びは [`ALL_AXES`] の順。
    /// `+` = `+1` / `-` = `-1` / `0` = **`0` と採点された**。
    ///
    /// # ⚠ **2ヶ所だけ、実機では捨てられた採点が `0` として入っている**
    ///
    /// turn 52（軸名の簡体字ゆらぎ）と turn 71（幅違反 `-2`）は、
    /// **本物の分解器なら採点表ごと捨てる回**である（ADR-0027 の 16e-c の節）。
    /// ADR の「追測」を出した道具（`scratchpad/m16/all.awk`）は
    /// **その2ヶ所を `0` として読んで**いるので、
    /// **同じ入力を与える**というこのテストの約束のとおり、ここでも `0` にしてある。
    ///
    /// **本番の配線では起きない形である**——`server::drift` は
    /// `parse_drift_all_report` が捨てた回に帳簿を回さない。
    const ALL_SCORECARD: &str = "\
        +0-++ +-00+ +-0++ +-0++ +-0++ +0-++ +-+++ +0-0+ +-+00 -0+0+ +-+0+ +-0++ \
        +-+0+ +0-0+ -++00 ++00+ -++00 ++-00 ++-0+ ++-++ +0+-+ -0+0+ +-+-+ +0+-+ \
        +-+0+ +-+-+ +-+0+ +-+++ +-+00 +-0-+ +-+0+ ++-00 +-+0+ +-0++ +-+0- +-+++ \
        ++-++ +--++ +0-++ +-00+ ++-0+ +--++ +-+-+ -++-+ 0++-+ -++-+ -++0- ++00+ \
        +0-0+ +-0++ +-0++ +--0+ +-0-+ +-0-+ -+-0+ +-+-0 +0-++ +-0-+ +-00+ ++0-+ \
        -++0+ +-+-+ +-+0+ +++-+ +-+0+ ++-0+ +-+0+ +-0-+ +-+++ +-0++ -0+0+ +-+0+";

    /// 腕E の軸（**採点表の列の並び**）。
    const ALL_AXES: [&str; 5] = ["自信", "好奇心", "慎重さ", "陽気さ", "共感"];

    /// **腕C の提案列**（`scratchpad/m14/c_no_empathy/votes.txt` の 72 ターン）。
    ///
    /// 1組が1ターンで、**数字が [`SINGLE_AXES`] への添字**、記号が向き。
    /// **1ターンに動くのは1軸だけ**（ADR-0024 §3）で、残りの3軸が「選ばれなかった」側になる。
    const SINGLE_VOTES: &str = "\
        3+ 3+ 3+ 3+ 3+ 0+ 0+ 0+ 0+ 0- 0+ 3+ 0+ 0- 0- 2+ 2+ 1+ \
        0+ 2+ 3+ 2+ 2+ 3+ 3+ 3+ 2+ 2+ 1+ 1+ 0+ 3- 1+ 3+ 1+ 3+ \
        3+ 3+ 3+ 0- 3- 3- 1+ 3- 1+ 2+ 2- 2- 1+ 3+ 1+ 0- 1+ 1+ \
        2+ 2+ 2+ 0+ 0+ 0+ 0+ 1+ 2+ 0+ 3+ 1+ 1+ 2+ 1+ 1+ 3+ 1+";

    /// 腕C の軸（**共感を外した4本**）。
    const SINGLE_AXES: [&str; 4] = ["自信", "好奇心", "慎重さ", "陽気さ"];

    /// 腕E・腕C の測定に使ったキャラクターの尺度（`scratchpad/m16/character.json`）。
    const REPLAY_SCALE: u32 = 10;

    /// 1回の当て直しの結果。**ADR の表の欄と1対1。**
    #[derive(Debug, PartialEq, Eq)]
    struct Replay {
        quiet_percent: u32,
        ups: usize,
        downs: usize,
        saturations: usize,
        total_before: u32,
        total_after: u32,
        values: Vec<u32>,
    }

    fn axis(raw: &str) -> StatAxis {
        StatAxis::parse(raw).expect("正当な軸名であること")
    }

    fn replay_scale() -> StatScale {
        StatScale::parse(REPLAY_SCALE).expect("正当な尺度であること")
    }

    fn ledger_for(form: DriftForm, names: &[&str], start: u32, tuning: DriftTuning) -> DriftLedger {
        let scale = replay_scale();
        let axes = names
            .iter()
            .map(|name| {
                DriftAxisLedger::new(
                    axis(name),
                    scale.value(start).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                )
            })
            .collect();
        DriftLedger::new(form, tuning, axes)
    }

    fn run_replay(
        mut ledger: DriftLedger,
        turns: &[Vec<Option<DriftDelta>>],
        start: u32,
    ) -> Replay {
        let axis_count = ledger.axes().len();
        let mut quiet = 0_usize;
        let mut ups = 0_usize;
        let mut downs = 0_usize;
        let mut saturations = 0_usize;
        for turn in turns {
            let effect = ledger.absorb(turn).expect("採点の件数が合っていること");
            ups += effect.ups();
            downs += effect.downs();
            saturations += effect.saturations().len();
            if !effect.moved() {
                quiet += 1;
            }
        }
        let values: Vec<u32> = ledger
            .axes()
            .iter()
            .map(|entry| entry.value().get())
            .collect();
        let total_after = values.iter().sum();
        let turns_count = u32::try_from(turns.len()).expect("ターン数が u32 に収まること");
        let quiet_u32 = u32::try_from(quiet).expect("なしの回数が u32 に収まること");
        let axis_count = u32::try_from(axis_count).expect("軸の本数が u32 に収まること");
        Replay {
            // ADR の表と同じ丸め方（四捨五入）。
            quiet_percent: (quiet_u32 * 100 + turns_count / 2) / turns_count,
            ups,
            downs,
            saturations,
            total_before: start * axis_count,
            total_after,
            values,
        }
    }

    /// 採点表の文字列を1ターンずつの採点に開く。
    fn all_turns() -> Vec<Vec<Option<DriftDelta>>> {
        let turns: Vec<Vec<Option<DriftDelta>>> = ALL_SCORECARD
            .split_whitespace()
            .map(|group| {
                assert_eq!(
                    group.chars().count(),
                    ALL_AXES.len(),
                    "1ターンは軸の本数ぶんの採点であること: {group}"
                );
                group
                    .chars()
                    .map(|mark| match mark {
                        '+' => Some(DriftDelta::Up),
                        '-' => Some(DriftDelta::Down),
                        '0' => None,
                        other => panic!("採点表に読めない印がある: {other}"),
                    })
                    .collect()
            })
            .collect();
        assert_eq!(turns.len(), 72, "腕E は 72 ターン");
        turns
    }

    /// 提案列の文字列を1ターンずつの採点に開く（**選ばれなかった軸は `None`**）。
    fn single_turns() -> Vec<Vec<Option<DriftDelta>>> {
        let turns: Vec<Vec<Option<DriftDelta>>> = SINGLE_VOTES
            .split_whitespace()
            .map(|entry| {
                let mut marks = entry.chars();
                let index = marks
                    .next()
                    .and_then(|digit| digit.to_digit(10))
                    .and_then(|digit| usize::try_from(digit).ok())
                    .expect("先頭が軸の添字であること");
                let delta = match marks.next() {
                    Some('+') => DriftDelta::Up,
                    Some('-') => DriftDelta::Down,
                    other => panic!("提案列に読めない向きがある: {other:?}"),
                };
                assert!(index < SINGLE_AXES.len(), "添字が軸の範囲内であること");
                (0..SINGLE_AXES.len())
                    .map(|slot| if slot == index { Some(delta) } else { None })
                    .collect()
            })
            .collect();
        assert_eq!(turns.len(), 72, "腕C は 72 ターン");
        for turn in &turns {
            assert_eq!(
                turn.iter().filter(|score| score.is_some()).count(),
                1,
                "1ターン1軸であること（残りが「選ばれなかった軸」）"
            );
        }
        turns
    }

    // =====================================================================
    // **長い軌跡を、別の道具が出した値と突き合わせる**（ADR-0027 訂正 2〜4）
    // =====================================================================

    /// 採点表を何周させるか。**72 × 14 = 1008 ターン。**
    const LONG_RUN_PASSES: usize = 14;

    /// 軌跡を読む時点（ADR-0027 訂正 3・訂正 4 の表の欄）。
    const LONG_RUN_CHECKPOINTS: [usize; 4] = [72, 216, 504, 1008];

    /// **ある時点の帳簿の姿。** ADR / awk の表の1行と1対1。
    ///
    /// # 「なし」と総和も持つ（作業単位 16g で畳み込んだ）
    ///
    /// 値・上げ・下げ・飽和だけでは、**「1ターンで2段」と「2ターンで1段ずつ」が区別できない。**
    /// **`quiet`（値が1バイトも動かなかったターンの数）がそこを見る。**
    #[derive(Debug, PartialEq, Eq)]
    struct Snapshot {
        turns: usize,
        values: Vec<u32>,
        total: u32,
        ups: usize,
        downs: usize,
        saturations: usize,
        quiet: usize,
    }

    /// [`ALL_SCORECARD`] を [`LONG_RUN_PASSES`] 周させ、各時点の姿を返す。
    ///
    /// **つまみは訂正 4 の表を出したときの組を直書き**（λ=1 / 傾き 1.5 / K=1）。
    ///
    /// # ⚠ **これはもう既定ではない**（作業単位 19）
    ///
    /// **既定は 16j が λ=`0.5` / K=`0.3` へ移した**（ADR-0027 §決定8）。
    /// ここが直書きしているのは**表が出たときの設定**であって、既定ではない
    /// ——CLAUDE.md「表は、どの設定で出したかを書かないと写しですらない」。
    /// **[`DriftTuning::default`] に置き換えないこと**——ここは**算術の照合**であって
    /// 既定の主張ではない（呼び出し側のテストの doc）。置き換えると表と合わなくなる。
    fn long_run(scale_raw: u32, start: u32) -> Vec<Snapshot> {
        long_run_tuned(
            scale_raw,
            start,
            DriftTuning::new(1.0, 1.5, 1.0).expect("つまみが範囲内であること"),
        )
    }

    /// つまみを指定して同じ軌跡を回す。
    ///
    /// **つまみを振って比べるために要る**
    /// ——[`the_default_tuning_moves_the_ledger_further_than_the_one_it_replaced`] が
    /// **既定と「既定の1つだけ 16j 以前へ戻した組」**を同じ入力に通す。
    /// **ここに [`DriftTuning::default`] を既定引数として埋め込まないこと**
    /// （呼ぶ側がどの組で回したかが読めなくなる）。
    fn long_run_tuned(scale_raw: u32, start: u32, tuning: DriftTuning) -> Vec<Snapshot> {
        let turns = all_turns();
        let scale = StatScale::parse(scale_raw).expect("正当な尺度であること");
        let axes = ALL_AXES
            .iter()
            .map(|name| {
                DriftAxisLedger::new(
                    axis(name),
                    scale.value(start).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                )
            })
            .collect();
        let mut ledger = DriftLedger::new(DriftForm::All, tuning, axes);

        let mut ups = 0_usize;
        let mut downs = 0_usize;
        let mut saturations = 0_usize;
        let mut quiet = 0_usize;
        let mut seen = 0_usize;
        let mut shots = Vec::new();
        for _ in 0..LONG_RUN_PASSES {
            for turn in &turns {
                let effect = ledger.absorb(turn).expect("採点の件数が合っていること");
                ups += effect.ups();
                downs += effect.downs();
                saturations += effect.saturations().len();
                if !effect.moved() {
                    quiet += 1;
                }
                seen += 1;
                if LONG_RUN_CHECKPOINTS.contains(&seen) {
                    let values: Vec<u32> =
                        ledger.axes().iter().map(|row| row.value().get()).collect();
                    shots.push(Snapshot {
                        turns: seen,
                        total: values.iter().sum(),
                        values,
                        ups,
                        downs,
                        saturations,
                        quiet,
                    });
                }
            }
        }
        assert_eq!(seen, 1008, "72 × 14 = 1008 ターン回っていない");
        assert_eq!(
            shots.len(),
            LONG_RUN_CHECKPOINTS.len(),
            "読む時点が足りない"
        );
        shots
    }
    /// 値を**尺度に対する割合**（%）へ直す。**尺度をまたいで比べるための共通の単位。**
    fn percents(values: &[u32], scale: u32) -> Vec<u32> {
        values.iter().map(|value| value * 100 / scale).collect()
    }

    /// **1008 ターンの軌跡が、別の道具が出した表と一致する**
    /// （ADR-0027 訂正 2 / 訂正 3 / 訂正 4 / §決定8）。
    ///
    /// # なぜこれが要るのか
    ///
    /// 3つの規則——**1段の費用の尺度による正規化**（訂正 2）・
    /// **票の重み `0.1/√n`**（訂正 3）・**`all` の薄れを票の重みに比例させる**（訂正 4）
    /// ——は**互いに打ち消し合う。** どれか1つを元に戻しても、
    /// 72 ターンの短い当て直しでは「少し数が違う」以上のことが言えない。
    ///
    /// **長い軌跡だけが、3つの釣り合いを見る。**
    /// 訂正 4 が見つけた崩れ（**5軸のうち4軸が 0 に落ちて戻らない**）は
    /// **1008 ターン回さないと現れなかった**——16f が 72 ターンしか回さなかったので
    /// 見えていなかった。
    ///
    /// # ⚠ **期待値は実装の外から来ている**
    ///
    /// `scratchpad/m18/shape2.awk`（司令塔）と `scratchpad/m19/nr.awk`（**独立の実装**）が
    /// 同じ入力に対して出した値を**直書き**してある。
    /// **実装から作った値ではないし、定数から組み立ててもいない。**
    ///
    /// 入力は [`ALL_SCORECARD`]（腕E の 72 ターン）を **14 周**したもの
    /// ——`scratchpad/m16/e.tsv` と1行ずつ同じである。
    ///
    /// # ⚠ **`the_all_axis_scorecard_reproduces_the_numbers_in_the_adr` を畳み込んである**
    ///
    /// **16g で消した。** あちらの2組は**ここの `turns: 72` の2件と1フィールドも違わず**、
    /// そのうえ**下げの枝にも飽和の枝にも一度も入らなくなっていた**
    /// （16g 以前は 下げ7 / 飽和1 だった）。
    /// **あちらだけが持っていたもの**——「なし」の回数・総和・
    /// **[`DriftTuning::default`] との結び直し**——を**ここへ移した。**
    ///
    /// # ⚠ **このテストは「既定の主張」ではない。算術の照合である**（作業単位 19）
    ///
    /// **λ と `K` は入力にすぎない。** ここが持っている性質は
    /// **「台帳の算術が、別実装（awk）と1フィールドも違わず一致する」**ことであって、
    /// **「既定がこの値だ」ではない。**
    /// **だから λ=`1` / 傾き `1.5` / `K=1` を [`long_run`] に直書きで固定してある**
    /// ——**表が出たときの設定**を書き写しているのであって、既定を写しているのではない
    /// （CLAUDE.md「表は、どの設定で出したかを書かないと写しですらない」）。
    ///
    /// **[`DriftTuning::default`] へ結び直さないこと。**
    /// 結ぶと**既定が動くたびにこのテストが壊れる。**
    /// **2026-08-25 に実際に壊れた**——16j が既定を λ=`0.5` / `K=0.3` へ移し、
    /// ここに置いてあった結び直しが一致しなくなった（作業単位 19 で外した）。
    ///
    /// > **⚠ 新既定の表は出せる。それでも埋めない。**
    /// > 司令塔が `scratchpad/m18/shape2.awk` を新既定で回すと
    /// > **初期5 は 1008 ターンで `10 0 8 5 10`、飽和 10 件**になる。
    /// > **これは欠陥ではなく 16j の結論そのもの**——[`ALL_SCORECARD`] は
    /// > **16h の軸の組**（`共感` を含む）で、**16j は「この組はどの λ・`K` でも合格しない」**
    /// > と結論している（`共感` はどのつまみでも上端に着く）。
    /// > **その表を期待値に埋めると、「なぜ既定で飽和するのか」という問いが
    /// > 永久にこのファイルに残る。** 算術の照合に、軸の組の欠陥を持ち込まない。
    ///
    /// # 既定を通る観測は別に1本ある
    ///
    /// [`the_default_tuning_moves_the_ledger_further_than_the_one_it_replaced`] が
    /// **数の一致ではなく向きの対比**（安い側は動く / 高い側は動かない）で置いてある。
    /// **既定を 16j 以前の値へ戻す変異は、あちらで赤になる。**
    #[test]
    fn the_thousand_turn_trajectory_matches_the_table_the_other_tool_produced() {
        // ---- 尺度 10 / 初期値 5（ADR-0027 訂正 4「票の重み/N（採用）」の行）----
        let from_five = vec![
            Snapshot {
                turns: 72,
                values: vec![6, 5, 5, 5, 6],
                total: 27,
                ups: 2,
                downs: 0,
                saturations: 0,
                quiet: 70,
            },
            Snapshot {
                turns: 216,
                values: vec![6, 4, 5, 5, 7],
                total: 27,
                ups: 3,
                downs: 1,
                saturations: 0,
                quiet: 212,
            },
            Snapshot {
                turns: 504,
                values: vec![7, 4, 5, 5, 8],
                total: 29,
                ups: 5,
                downs: 1,
                saturations: 0,
                quiet: 498,
            },
            Snapshot {
                turns: 1008,
                values: vec![8, 3, 5, 5, 9],
                total: 30,
                ups: 7,
                downs: 2,
                saturations: 0,
                quiet: 999,
            },
        ];
        assert_eq!(
            long_run(10, 5),
            from_five,
            "尺度10・初期5 の軌跡が ADR-0027 訂正 4 の表と食い違っている"
        );

        // ---- 尺度 10 / 初期値 0（§決定6「ゼロから育てる」）----
        //
        // **凍らないこと**が訂正 4 の要件そのもの
        // （薄れが定数 `1/N` のままだと 504 ターンで `4 0 1 0 3` に止まる）。
        let from_zero = vec![
            Snapshot {
                turns: 72,
                values: vec![2, 0, 1, 1, 3],
                total: 7,
                ups: 7,
                downs: 0,
                saturations: 0,
                quiet: 66,
            },
            Snapshot {
                turns: 216,
                values: vec![4, 0, 1, 1, 4],
                total: 10,
                ups: 10,
                downs: 0,
                saturations: 0,
                quiet: 207,
            },
            Snapshot {
                turns: 504,
                values: vec![5, 0, 2, 1, 6],
                total: 14,
                ups: 14,
                downs: 0,
                saturations: 0,
                quiet: 491,
            },
            Snapshot {
                turns: 1008,
                values: vec![6, 0, 3, 1, 7],
                total: 17,
                ups: 17,
                downs: 0,
                saturations: 1,
                quiet: 992,
            },
        ];
        assert_eq!(
            long_run(10, 0),
            from_zero,
            "尺度10・初期0 の軌跡が ADR-0027 訂正 4 の表と食い違っている"
        );

        // **軸の本数**——`all` の薄れは `票の重みの合計 ÷ N` なので、
        // **N が変わると上の表がまるごと動く。** フィクスチャの前提として名指しする。
        assert_eq!(ALL_AXES.len(), 5, "腕E の採点表は5軸");
    }

    /// **既定のつまみが帳簿へ届いていて、16j 以前の組より深く動かす**
    /// （ADR-0027 §決定8 / 16j / 作業単位 19）。
    ///
    /// # ここが「既定のつまみを通る」側の結び直しである
    ///
    /// [`the_thousand_turn_trajectory_matches_the_table_the_other_tool_produced`] は
    /// **つまみを明示して**回っているので、**既定を変える変異では赤にならない。**
    /// ADR-0027 §決定8 が公表しているのは「この値で釣り合わせた」という**組そのもの**なので、
    /// **既定を通る観測が別に1本要る**（CLAUDE.md「テスト専用の入口を作ると…」）。
    ///
    /// # なぜ数の一致ではなく向きの対比なのか
    ///
    /// **新しい組（λ=0.5 / K=0.3）で [`ALL_SCORECARD`] を 1008 ターン回した表は、
    /// どの道具も出していない。** 実装の出力を写すと、
    /// **既定を変える変異が期待値を道連れにして緑を通る**（CLAUDE.md）。
    /// **比べる相手を「既定の1つだけ 16j 以前の値へ戻した組」にすると、
    /// 決め打ちの変異は両辺を同値にするので、厳密な不等号が割れる。**
    ///
    /// # ⚠ **つまみごとに別の対比を置く**
    ///
    /// | つまみ | 比べる相手 | 期待する向き |
    /// |---|---|---|
    /// | `K`（`0.3` ← `1.0`） | **λ は既定のまま、`K` だけ `1.0`** | **1段が安いので段数が増える** |
    /// | `λ`（`0.5` ← `1.0`） | **`K` は既定のまま、λ だけ `1.0`** | **薄れが半分なので上げが増える** |
    ///
    /// **1つにまとめない**——まとめると、片方だけを戻す変異が
    /// もう片方の差でごまかされて緑を通る。
    #[test]
    fn the_default_tuning_moves_the_ledger_further_than_the_one_it_replaced() {
        /// 16j より前の既定（ADR-0027 §決定8 の取り消し線の側）。
        const RETIRED: f64 = 1.0;

        let default = DriftTuning::default();
        let last = |tuning: DriftTuning| {
            long_run_tuned(10, 5, tuning)
                .pop()
                .expect("1008 ターンの姿があること")
        };

        let mine = last(default);

        // ---- `K` が届いているか（安い側は動く / 高い側は動かない）----
        let dearer = DriftTuning::new(default.decay(), default.slope(), RETIRED)
            .expect("つまみが範囲内であること");
        let dear = last(dearer);
        let steps = |shot: &Snapshot| shot.ups + shot.downs + shot.saturations;
        assert!(
            steps(&mine) > steps(&dear),
            "既定の `K` が高い側と同じ段数しか動かない（K が届いていない）: \
             既定 {mine:?} / K={RETIRED} {dear:?}"
        );

        // ---- λ が届いているか（薄れが半分なので上げが増える）----
        let fadier = DriftTuning::new(RETIRED, default.slope(), default.step_cost())
            .expect("つまみが範囲内であること");
        let faded = last(fadier);
        assert!(
            mine.ups > faded.ups,
            "既定の λ が薄れの強い側と同じだけしか上げない（λ が届いていない）: \
             既定 {mine:?} / λ={RETIRED} {faded:?}"
        );

        // **前提: 3つとも 16j 以前の値ではないこと**（対比の相手が既定と同値だと何も言えない）。
        assert_ne!(default.step_cost(), RETIRED, "K が 16j 以前の値のまま");
        assert_ne!(default.decay(), RETIRED, "λ が 16j 以前の値のまま");
    }

    /// **尺度 100 が尺度 10 と同じ割合まで動く**（ADR-0027 訂正 2）。
    ///
    /// # 正規化が入っていないと、ここで止まる
    ///
    /// 正規化前は、尺度 100 の同じ会話で**全軸が 46〜53% に張り付き、分化が消える**
    /// （ADR-0027 訂正 2 の実測表）。**尺度は粒度のつまみであって、速さのつまみではない。**
    ///
    /// # ⚠ **完全一致はしない。値が整数なので丸めが残る**
    ///
    /// ADR-0027 訂正 2 が予告したとおり**数ポイントずれる**ので、
    /// **割合が近いこと**で置く。**絶対値の側は別の道具が出した表を直書き**してある
    /// （こちらは丸めも含めて決定的）。
    #[test]
    fn the_same_conversation_moves_the_same_share_of_the_range_at_scale_one_hundred() {
        let coarse = long_run(10, 5);
        let fine = long_run(100, 50);

        // **別の道具が出した表**（`scratchpad/m18/shape2.awk` の `-v HI=100 -v START=50`）。
        assert_eq!(
            fine.first().expect("72 ターンの姿があること").values,
            vec![60, 43, 50, 50, 62],
            "尺度100・72ターンの値が別の道具の表と食い違っている"
        );
        assert_eq!(
            fine.last().expect("1008 ターンの姿があること").values,
            vec![85, 28, 57, 46, 90],
            "尺度100・1008ターンの値が別の道具の表と食い違っている"
        );

        // **割合が近いこと。** 丸めぶんの許容は 8 ポイント。
        for (rough, exact) in coarse.iter().zip(fine.iter()) {
            assert_eq!(rough.turns, exact.turns, "同じ時点どうしを比べていない");
            let want = percents(&rough.values, 10);
            let got = percents(&exact.values, 100);
            for (index, (left, right)) in want.iter().zip(got.iter()).enumerate() {
                let gap = left.abs_diff(*right);
                assert!(
                    gap <= 8,
                    "{}ターン目・{}軸目で尺度10と尺度100が {gap} ポイント離れた: {want:?} vs {got:?}",
                    exact.turns,
                    index + 1
                );
            }
        }

        // **ずれが丸めの範囲に収まっているだけでなく、分化そのものが残っていること。**
        // 正規化を外すと尺度 100 は全軸が中間へ張り付く（ADR-0027 訂正 2）。
        let spread = percents(&fine.last().expect("1008 ターンの姿").values, 100);
        let high = spread.iter().max().copied().unwrap_or_default();
        let low = spread.iter().min().copied().unwrap_or_default();
        assert!(
            high - low >= 50,
            "尺度100で分化が消えている（正規化が効いていない）: {spread:?}"
        );
    }

    /// **1段の費用は尺度で正規化されている**（ADR-0027 訂正 2）。
    ///
    /// 軌跡は「噛み合った結果」しか見ないので、**式そのものを1本で押さえる。**
    /// `K` の意味は「1段の費用」から**「尺度を10段ぶん動かす費用」**へ変わった。
    #[test]
    fn a_step_costs_the_same_share_of_the_range_at_every_scale() {
        let tuning = DriftTuning::new(1.0, 1.5, 1.0).expect("つまみが範囲内であること");

        // **既定尺度（10）では1バイトも変わらない**——訂正 2 の約束。
        assert!((tuning.step_cost_of(5, 10, DriftDelta::Up) - 1.0).abs() < 1e-12);
        assert!((tuning.step_cost_of(8, 10, DriftDelta::Up) - 1.45).abs() < 1e-12);
        assert!((tuning.step_cost_of(2, 10, DriftDelta::Up) - 0.55).abs() < 1e-12);

        // **他の尺度では `K × (既定尺度 / 尺度)` が基準になる。**
        // 尺度は **10 の倍数だけ**にしてある——中点（`尺度/2`）と 80% 地点が
        // **整数で表せる尺度でないと、丸めが混ざって「比例しているか」が読めない。**
        for (scale, base) in [(10_u32, 1.0_f64), (20, 0.5), (50, 0.2), (100, 0.1)] {
            let midpoint = scale / 2;
            for delta in DriftDelta::ALL {
                let at_mid = tuning.step_cost_of(midpoint, scale, delta);
                assert!(
                    (at_mid - base).abs() < 1e-12,
                    "尺度{scale} の中点の費用が K×(10/尺度) でない: {at_mid} vs {base}"
                );
            }
            // **同じ「範囲の何割」を動くのに、同じだけ掛かる。**
            // 尺度10 の 8/10 は尺度100 の 80/100 と同じ位置。
            let eighty = scale * 4 / 5;
            let scaled = tuning.step_cost_of(eighty, scale, DriftDelta::Up);
            assert!(
                (scaled - base * 1.45).abs() < 1e-9,
                "尺度{scale} の 80% 地点の費用が尺度10 と比例していない: {scaled}"
            );
        }

        // **中点が整数にならない尺度でも、基準は `K × (10/尺度)` である。**
        // 上げと下げの平均は傾きの項が打ち消えるので、**どの値からでも基準を取り出せる。**
        for scale in [1_u32, 3, 7, 33, 99] {
            let from_zero = (tuning.step_cost_of(0, scale, DriftDelta::Up)
                + tuning.step_cost_of(0, scale, DriftDelta::Down))
                / 2.0;
            assert!(
                (from_zero * f64::from(scale) - 10.0).abs() < 1e-9,
                "尺度{scale} の基準が K×(10/尺度) でない: {from_zero}"
            );
        }

        // **正規化を落とすと、尺度 100 の1段が尺度 10 と同じ `K` になる。**
        // その状態と区別が付くことを、名指しで置く。
        assert!(
            tuning.step_cost_of(50, 100, DriftDelta::Up)
                < tuning.step_cost_of(5, 10, DriftDelta::Up),
            "尺度100 の1段が尺度10 と同じ値のまま（正規化が入っていない）"
        );
    }

    /// **票の重みは `係数 / √(0 でなかった回数)`**（ADR-0027 訂正 3）。
    ///
    /// # `1/n` との違いは「凍るかどうか」
    ///
    /// `1/n` の合計は調和級数なので**対数でしか伸びず、序盤で決着して以後凍る**
    /// （16f / 腕H で `自信` が上げ 53 票を集めながら累積 `0.911` / 費用 `1.000` で止まった）。
    /// `1/√n` の合計は `2√n` で**発散する**ので、**係数を下げても凍らない。遅くなるだけ。**
    ///
    /// **係数は速さのつまみではない**（速さのつまみは `K`）ので、
    /// [`DRIFT_VOTE_WEIGHT_COEFFICIENT`] という名前の定数に据えてある。
    #[test]
    fn the_vote_weight_falls_as_the_inverse_square_root_of_the_count() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // **段を動かさない費用**にして、累積だけを見る。薄れる側も止める。
            DriftTuning::new(0.0, 0.0, 1_000_000.0).expect("つまみが範囲内であること"),
            vec![DriftAxisLedger::new(
                axis("ひとすじ"),
                scale.value(5).expect("尺度に収まる値であること"),
                0.0,
                0,
            )],
        );

        let mut want = 0.0_f64;
        for count in 1..=9_u32 {
            ledger
                .absorb(&[Some(DriftDelta::Up)])
                .expect("件数が合うこと");
            want += 0.1 / f64::from(count).sqrt();
            let charge = ledger.axes().first().expect("行があること").charge();
            assert!(
                (charge - want).abs() < 1e-12,
                "{count} 票目までの累積が 0.1/√n の合計になっていない: {charge} vs {want}"
            );
        }

        // **`1/n` との差**——同じ 9 票でも `1/√n` の方が 1.5 倍以上貯まる。
        // 調和級数は**対数でしか伸びない**ので、票が増えるほど差が開く。
        let charge = ledger.axes().first().expect("行があること").charge();
        let harmonic: f64 = (1..=9_u32).map(|n| 0.1 / f64::from(n)).sum();
        assert!(
            charge > harmonic * 1.5,
            "9 票の累積が `1/n` と見分けが付かない: {charge} vs {harmonic}"
        );

        // **公開している係数と同じ値であること。**
        assert!(
            (DRIFT_VOTE_WEIGHT_COEFFICIENT - 0.1).abs() < 1e-12,
            "票の重みの係数が 0.1 でない"
        );
    }

    /// **`all` の薄れは、その回に立った票の重みに比例する**（ADR-0027 訂正 4）。
    ///
    /// # 定数 `1/N` との違いは「長期で崩れるかどうか」
    ///
    /// 票の重みが `n` とともに減るのに薄れが定数だと、
    /// **回数が増えるほど薄れが相対的に強くなり、5軸のうち4軸が 0 に落ちて戻らない**
    /// （訂正 4 の実測。**出荷済みの規則でも 1008 ターンで `8 0 0 0 2`**）。
    ///
    /// # 1ターンで直接見る
    ///
    /// [`the_thousand_turn_trajectory_matches_the_table_the_other_tool_produced`] は
    /// 1008 ターンの合算なので、**何が壊れたのかが読めない。**
    /// ここは**同じ軸・同じ値に、票の数だけが違う1ターン**を当てる
    /// ——定数なら2回とも同じだけ薄れる。
    #[test]
    fn the_all_form_fade_is_proportional_to_the_votes_cast_this_turn() {
        let scale = replay_scale();
        let faded = |scores: &[Option<DriftDelta>]| {
            let mut ledger = DriftLedger::new(
                DriftForm::All,
                DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
                (0..4)
                    .map(|index| {
                        DriftAxisLedger::new(
                            axis(&format!("軸{index}")),
                            scale.value(5).expect("尺度に収まる値であること"),
                            0.0,
                            0,
                        )
                    })
                    .collect(),
            );
            ledger.absorb(scores).expect("件数が合うこと");
            ledger.axes().get(1).expect("2本目があること").charge()
        };

        // 1票（重み 0.1）。薄れる重み = 0.1/4。値 5/10 なので −0.0125。
        let one_vote = faded(&[Some(DriftDelta::Up), None, None, None]);
        assert!(
            (one_vote - (-0.0125)).abs() < 1e-12,
            "1票の回の薄れが「票の重みの合計 ÷ N」になっていない: {one_vote}"
        );

        // 3票（重み 0.1 が3つ）。薄れる重み = 0.3/4。**ちょうど3倍。**
        let three_votes = faded(&[
            Some(DriftDelta::Up),
            None,
            Some(DriftDelta::Down),
            Some(DriftDelta::Up),
        ]);
        assert!(
            (three_votes - (-0.0375)).abs() < 1e-12,
            "3票の回の薄れが票の重みの合計に比例していない: {three_votes}"
        );
        assert!(
            (three_votes / one_vote - 3.0).abs() < 1e-9,
            "票が3倍でも薄れが3倍になっていない（定数のまま）: {three_votes} / {one_vote}"
        );

        // **票が1つも立たなかった回は、何も薄れない**——`single` と同じ規則である
        // （見張り番は `a_turn_with_no_vote_fades_nothing_in_the_single_form`）。
        let no_vote = faded(&[None, None, None, None]);
        assert!(
            no_vote.abs() < 1e-12,
            "票が1つも無いのに薄れた（配られていない票を失っている）: {no_vote}"
        );
    }

    /// **ADR-0027 §9 の表の「形」を再現する**（腕C の提案列 × `single`）。
    ///
    /// # `all` の再現だけでは足りない
    ///
    /// **薄れる重みが形で違う**（モジュール doc の表）。`all` の側だけを見張ると、
    /// **`single` の薄れる重みを「割る数 `N`」に揃える変異**が緑を通り、
    /// 「選ばれなかった N−1 軸が薄れる」という §決定2・§決定3 の式が
    /// **誰にも守られなくなる。**
    ///
    /// # ⚠ **`K` は `1` ではなく `0.2`**（作業単位 16g）
    ///
    /// **票の重みが `1/n` から `0.1/√n` になった**（ADR-0027 訂正 3）ので、
    /// **72 ターンで貯まる累積が約 1/5 になる。**
    /// `K = 1` のままだと**この提案列では 72 ターンで1段も動かず**、
    /// 「上端寄りでは下げが優勢」も「ゼロ始まりが伸びる」も**観測できなくなる。**
    /// **`K` を 1/5 にして、同じ景色を同じ長さで見る。**
    ///
    /// **`K` は速さのつまみである**（訂正 3 の表）。ここで振っているのは速さだけで、
    /// **見張っている性質は §9 のまま**である。
    ///
    /// # ⚠ **数の出どころは ADR-0027「訂正 5」の表**（作業単位 16g）
    ///
    /// **§9 の表（初期 8 → 上げ4 下げ9 / 32→27 など）は票の重み `1/n` で出ていた**ので、
    /// 訂正 2〜4 のあとは再現されない。**§9 の決定は生きていて、数だけが訂正 5 へ移った。**
    /// §9 には ⚠ と誘導が入っている。
    ///
    /// 訂正 5 の数は**この見張り番と同じ道具**（`scratchpad/m19/nr.awk`。
    /// 旧規則を入れると §9 の公表値を1桁まで再現する）が出したもので、
    /// **実装から作った値ではない。**
    ///
    /// **§9 の「形」——上端寄りは下げが優勢・中間は上げが優勢・ゼロ始まりは伸びる——は
    /// そのまま残っている。** それがこの見張り番の中身である。
    #[test]
    fn the_single_axis_proposals_reproduce_the_numbers_in_the_adr() {
        let turns = single_turns();
        // **`K` は `0.2`**（doc の ⚠）。票の重みが 1/5 になったぶんを戻す。
        let tuned = || DriftTuning::new(1.0, 1.5, 0.2).expect("つまみが範囲内であること");

        // §9「レベル依存＋非対称」の初期 8（上端寄り）。**下げが優勢になる行。**
        let from_eight = run_replay(
            ledger_for(DriftForm::Single, &SINGLE_AXES, 8, tuned()),
            &turns,
            8,
        );
        assert_eq!(
            (from_eight.ups, from_eight.downs, from_eight.total_after),
            (2, 5, 29),
            "ADR-0027 訂正 5 §9 の初期 8（上げ2 下げ5 / 32→29）と食い違っている: {from_eight:?}"
        );
        assert_eq!(from_eight.total_before, 32);
        assert_eq!(from_eight.saturations, 0, "上端寄りでも飽和しないこと");
        assert_eq!(from_eight.values, vec![7, 8, 7, 7], "軸ごとの着地点");
        assert!(
            from_eight.downs > from_eight.ups,
            "§9 の「上端寄りでは下げが優勢」が崩れている: {from_eight:?}"
        );

        // 初期 5（中間）。**中点の付近は値付けの影響がほとんど無い**（§9 の主張）。
        let from_five = run_replay(
            ledger_for(DriftForm::Single, &SINGLE_AXES, 5, tuned()),
            &turns,
            5,
        );
        assert_eq!(
            (from_five.ups, from_five.downs, from_five.total_after),
            (4, 1, 23),
            "ADR-0027 訂正 5 §9 の初期 5（上げ4 下げ1 / 20→23）と食い違っている: {from_five:?}"
        );
        assert_eq!(from_five.total_before, 20);
        assert_eq!(from_five.saturations, 0);
        assert_eq!(from_five.values, vec![5, 6, 6, 6], "軸ごとの着地点");
        assert!(
            from_five.ups > from_five.downs,
            "§9 の「中間では上げが優勢」が崩れている: {from_five:?}"
        );

        // 初期 0（育てる）。**ADR の数は傾き 2 のもの**なので、性質の側で押さえる。
        let from_zero = run_replay(
            ledger_for(DriftForm::Single, &SINGLE_AXES, 0, tuned()),
            &turns,
            0,
        );
        let flat_from_zero = run_replay(
            ledger_for(
                DriftForm::Single,
                &SINGLE_AXES,
                0,
                DriftTuning::new(1.0, 0.0, 0.2).expect("つまみが範囲内であること"),
            ),
            &turns,
            0,
        );
        // **一律の列**（上げ8 / 0→8）。
        assert_eq!(
            (flat_from_zero.ups, flat_from_zero.total_after),
            (8, 8),
            "ADR-0027 訂正 5「値付けを切った列」の初期 0（上げ8 / 0→8）と食い違っている: {flat_from_zero:?}"
        );
        assert!(
            from_zero.ups > flat_from_zero.ups
                && from_zero.total_after > flat_from_zero.total_after,
            "値付けを入れてもゼロ始まりの伸びが速くならない: {from_zero:?} vs {flat_from_zero:?}"
        );
        assert_eq!(
            from_zero.saturations, 0,
            "値付けがあれば、ゼロ始まりでも下端に貼り付かないこと"
        );
        // **無償の1段が復活すると、この数を追い越す**（傾き 2 では費用 0 の段が作れる）。
        assert_eq!(
            (from_zero.ups, from_zero.total_after),
            (16, 16),
            "ADR-0027 訂正 5 の値付けあり・初期 0（上げ16 / 0→16）から動いた（無償の1段が戻っていないか）: {from_zero:?}"
        );
    }

    /// **値付けを外すと（傾き 0）、ADR §9 の「一律」の列の形になる。**
    ///
    /// # なぜ「外した側」も見張るのか
    ///
    /// ADR §10 が「**減衰と値付けは別物。両方要る**」と書いている。
    /// 値付けだけを見張ると、**傾きを読み捨てて常に既定で回す実装**が緑を通る
    /// ——つまみが効いていないことは、既定値で測っている限り観測できない。
    ///
    /// # ⚠ **`K` は `1` ではなく `0.2`**（作業単位 16g）
    ///
    /// 理由は [`the_single_axis_proposals_reproduce_the_numbers_in_the_adr`] と同じ
    /// ——**票の重みが `0.1/√n` になり、72 ターンで貯まる累積が約 1/5 になった。**
    /// **数の出どころは ADR-0027「訂正 5」の「値付けを切った列」**
    /// （同じ道具 `scratchpad/m19/nr.awk` が新しい規則で出したもの）で、
    /// **実装から作った値ではない。**
    ///
    /// > **⚠ 訂正 5 の表と、初期 8・初期 5 の「上げ」の欄が食い違っている。**
    /// > **ADR 側の転記の誤り**——16g の作業者が測った実測は
    /// > 初期 8 が **上げ4 下げ5 / 32→31**（ADR は 上げ3 / 32→**35**。
    /// > **ADR 自身が併記している着地点 `7 8 8 8` は 31 なので、35 は内部でも矛盾している**）、
    /// > 初期 5 が **上げ4 下げ1**（ADR は 上げ3。総和 23 と着地点 `5 6 6 6` は一致）。
    /// > **初期 0 の行は完全に一致する。**
    /// > **ここは実測の側を持つ。** 司令塔へ報告済み（16g）。
    ///
    /// # 値付けありの側と比べる
    ///
    /// **一律の列だけを直書きすると、傾きを読み捨てる実装が「たまたま同じ数」で
    /// 通り抜けうる**（中点の付近は値付けの影響がほとんど無いので、初期 5 では実際に同値）。
    /// **初期 8 と初期 0 は値付けの有無で着地点が違う**ので、そこを名指しで比べる。
    #[test]
    fn turning_off_the_pricing_gives_the_flat_column_of_the_adr() {
        let turns = single_turns();
        let flat = || DriftTuning::new(1.0, 0.0, 0.2).expect("つまみが範囲内であること");
        let priced = || DriftTuning::new(1.0, 1.5, 0.2).expect("つまみが範囲内であること");

        let flat_from_eight = run_replay(
            ledger_for(DriftForm::Single, &SINGLE_AXES, 8, flat()),
            &turns,
            8,
        );
        assert_eq!(
            flat_from_eight,
            Replay {
                quiet_percent: 92,
                ups: 4,
                downs: 5,
                saturations: 0,
                total_before: 32,
                total_after: 31,
                values: vec![7, 8, 8, 8],
            },
            "ADR-0027 訂正 5「値付けを切った列」の初期 8 の行と食い違っている"
        );
        assert_eq!(
            run_replay(
                ledger_for(DriftForm::Single, &SINGLE_AXES, 5, flat()),
                &turns,
                5
            ),
            Replay {
                quiet_percent: 93,
                ups: 4,
                downs: 1,
                saturations: 0,
                total_before: 20,
                total_after: 23,
                values: vec![5, 6, 6, 6],
            },
            "ADR-0027 訂正 5「値付けを切った列」の初期 5 の行と食い違っている"
        );
        let flat_from_zero = run_replay(
            ledger_for(DriftForm::Single, &SINGLE_AXES, 0, flat()),
            &turns,
            0,
        );
        assert_eq!(
            flat_from_zero,
            Replay {
                quiet_percent: 89,
                ups: 8,
                downs: 0,
                saturations: 0,
                total_before: 0,
                total_after: 8,
                values: vec![1, 3, 2, 2],
            },
            "ADR-0027 訂正 5「値付けを切った列」の初期 0 の行と食い違っている"
        );

        // **傾きを読み捨てる実装は、ここで赤くなる。**
        // 端の2つは、値付けの有無で着地点そのものが違う。
        for (start, without) in [(8_u32, &flat_from_eight), (0, &flat_from_zero)] {
            let with = run_replay(
                ledger_for(DriftForm::Single, &SINGLE_AXES, start, priced()),
                &turns,
                start,
            );
            assert_ne!(
                &with, without,
                "初期 {start} で値付けの有無が同じ着地点になっている（傾きが読まれていない）"
            );
        }
    }

    /// **`K` を上げると静かになり、下向きが消える**（ADR-0027 §7 の表の向き）。
    ///
    /// **`K` を読み捨てる実装**はここで赤くなる（上の再現は `K = 0.2` でしか回っていない）。
    ///
    /// # ⚠ **`K` の目盛りが 1/5 になった**（作業単位 16g）
    ///
    /// §7 の表は `K` を `0.75 / 1 / 1.5 / 2` で振っていたが、
    /// **票の重みが `1/n` から `0.1/√n` になった**（ADR-0027 訂正 3）ので、
    /// **その範囲では 72 ターンで1段も動かない。**
    /// **同じ比のまま 1/5 にした値**（`0.15 / 0.2 / 0.3 / 0.4`）で振る。
    ///
    /// **`K` は速さのつまみである**（訂正 3 の表）ので、目盛りが変わっても
    /// **見張っている向き——`K` を上げれば静かになり、下げれば下向きが出る——は同じ。**
    ///
    /// **数の出どころは ADR-0027「訂正 5」の §7 の置き換えの表**
    /// （同じ道具 `scratchpad/m19/nr.awk` が新しい規則で出したもの）である。
    /// **§7 の決定は生きていて、数だけが訂正 5 へ移った。** §7 には ⚠ と誘導が入っている。
    ///
    /// 訂正 5 の ⚠ が「**単調性だけでは足りない。全部 0 でも成り立つ**」と書いているので、
    /// **いちばん安い `K` で段が動いていること**も一緒に押さえる。
    #[test]
    fn a_larger_step_cost_makes_more_turns_quiet_and_removes_the_downward_steps() {
        let turns = single_turns();
        let replay_with = |cost: f64| {
            run_replay(
                ledger_for(
                    DriftForm::Single,
                    &SINGLE_AXES,
                    5,
                    DriftTuning::new(1.0, 0.0, cost).expect("つまみが範囲内であること"),
                ),
                &turns,
                5,
            )
        };

        let mut previous_quiet = 0;
        let mut previous_downs = usize::MAX;
        for cost in [0.15_f64, 0.2, 0.3, 0.4] {
            let replay = replay_with(cost);
            assert!(
                replay.quiet_percent >= previous_quiet,
                "K を上げたのに「なし」が増えていない: K={cost} {replay:?}"
            );
            assert!(
                replay.downs <= previous_downs,
                "K を上げたのに下げが増えた: K={cost} {replay:?}"
            );
            previous_quiet = replay.quiet_percent;
            previous_downs = replay.downs;
        }

        // §7 の表の両端に当たる2点（**別の道具が出した数を直書き**）。
        assert_eq!(
            replay_with(0.15).downs,
            3,
            "ADR-0027 訂正 5 §7 の K=0.15 の「下げ3」と食い違っている"
        );
        assert_eq!(
            replay_with(0.4).downs,
            0,
            "ADR-0027 訂正 5 §7 の K=0.4 の「下げ0」と食い違っている"
        );
        // **静かな端まで届いていること**——`K` を上げても何も起きないなら、
        // 上の単調性は「全部 0」でも成り立ってしまう。
        assert_eq!(
            replay_with(0.4).ups,
            0,
            "いちばん高い K でまだ段が動いている（目盛りが合っていない）"
        );
        assert!(
            replay_with(0.15).ups > 0,
            "いちばん安い K でも段が動かない（この提案列では何も観測できない）"
        );
    }

    // =====================================================================
    // 規則そのもの
    // =====================================================================

    /// **票の重みは「`0` でなかった回数」で決まる**（ADR-0027 §決定1 / §12 の危険3）。
    ///
    /// # `0` を数に入れる変異が、ここで赤くなる
    ///
    /// 全軸採点では**毎ターン全軸が採点される**ので、
    /// 「採点された回数」で割ると**全軸の回数が同じになり、重みが一律に戻る。**
    /// ADR が名指しで「数えるのは `0` でなかった回数」と書いた当のものである。
    ///
    /// 2本の軸に**同じ回数の採点**を与え、**非ゼロの回数だけを変える。**
    /// 数え方が「採点された回数」なら2本の累積は同じになり、
    /// 「`0` でなかった回数」なら**珍しい方の1票が重く残る。**
    #[test]
    fn the_weight_counts_only_the_turns_that_were_not_scored_zero() {
        let scale = replay_scale();
        let value = scale.value(5).expect("尺度に収まる値であること");
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // **段を動かさない費用**にして、累積だけを見る。薄れる側も止める。
            DriftTuning::new(0.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(axis("常連"), value, 0.0, 0),
                DriftAxisLedger::new(axis("たまに"), value, 0.0, 0),
            ],
        );

        // 3ターン: 常連は毎回 +1、たまには 0・0・+1。**採点された回数は同じ 3。**
        for scores in [
            [Some(DriftDelta::Up), None],
            [Some(DriftDelta::Up), None],
            [Some(DriftDelta::Up), Some(DriftDelta::Up)],
        ] {
            ledger.absorb(&scores).expect("件数が合っていること");
        }

        let entries = ledger.axes();
        let frequent = entries.first().expect("常連の行があること");
        let rare = entries.get(1).expect("たまにの行があること");

        assert_eq!(frequent.scored(), 3, "常連は3回とも 0 でなかった");
        assert_eq!(rare.scored(), 1, "たまには1回だけ 0 でなかった");

        // 常連: 0.1 × (1 + 1/√2 + 1/√3)。たまに: 0.1 × 1。
        let frequent_want = 0.1 * (1.0 + 1.0 / 2.0_f64.sqrt() + 1.0 / 3.0_f64.sqrt());
        assert!(
            (frequent.charge() - frequent_want).abs() < 1e-9,
            "常連の累積が希少度の重みになっていない: {} vs {frequent_want}",
            frequent.charge()
        );
        assert!(
            (rare.charge() - 0.1).abs() < 1e-9,
            "たまにの累積が1票ぶんの重みになっていない: {}",
            rare.charge()
        );
        // **3票の軸の1票あたりが、1票の軸より軽い。**
        assert!(
            frequent.charge() / 3.0 < rare.charge(),
            "よく出る軸の1票が軽くなっていない"
        );
    }

    /// **`0` と採点された軸は、現在値に比例して薄れる**（ADR-0027 §決定2 / §決定3）。
    ///
    /// # 値が違う3本を同じターンに置く
    ///
    /// 比例していなければ**2本が同じだけ減る。**
    /// 片方だけを測ると「比例しない実装」と区別が付かない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    ///
    /// **値 0 の軸は一切薄れない**——ADR §決定3 が「ゼロから育つのを邪魔しない」と
    /// 書いている当のものなので、同じ一巡で確かめる。
    ///
    /// # ⚠ **票を立てる軸を1本混ぜてある**（作業単位 16g）
    ///
    /// `all` の薄れる重みが **「その回に立った票の重みの合計 ÷ N」**になった
    /// （ADR-0027 訂正 4）ので、**全軸が `0` の回は何も薄れない。**
    /// 薄れそのものを観測するには、**同じターンに票が1つ以上立っている**必要がある。
    /// 見張り番の中身（**現在値に比例する** / **値 0 は薄れない**）は変えていない。
    #[test]
    fn a_zero_score_fades_the_axis_in_proportion_to_its_current_value() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // **段を動かさない費用**にして、累積だけを見る。
            DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(
                    axis("高い"),
                    scale.value(8).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("低い"),
                    scale.value(2).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("ゼロ"),
                    scale.value(0).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                // **票を配る軸**（doc の ⚠）。値は他の3本と別にしてある。
                DriftAxisLedger::new(
                    axis("語られた"),
                    scale.value(4).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
            ],
        );

        ledger
            .absorb(&[None, None, None, Some(DriftDelta::Up)])
            .expect("件数が合うこと");
        let entries = ledger.axes();
        let high = entries.first().expect("高い軸").charge();
        let low = entries.get(1).expect("低い軸").charge();
        let zero = entries.get(2).expect("ゼロの軸").charge();

        // λ=1 / 尺度10 / 薄れる重み = 票の重み 0.1 ÷ N(=4)。
        let fade_weight = 0.1 / 4.0;
        assert!(
            (high - (-8.0 / 10.0 * fade_weight)).abs() < 1e-9,
            "高い軸の薄れる量が現在値に比例していない: {high}"
        );
        assert!(
            (low - (-2.0 / 10.0 * fade_weight)).abs() < 1e-9,
            "低い軸の薄れる量が現在値に比例していない: {low}"
        );
        assert!(
            (high / low - 4.0).abs() < 1e-9,
            "8 と 2 の薄れる量の比が 4 になっていない: {high} / {low}"
        );
        assert!(
            zero.abs() < 1e-12,
            "値 0 の軸が薄れている（ゼロから育つ道を塞いでいる）: {zero}"
        );
    }

    /// **薄れる強さ `λ` が効いている。**
    ///
    /// `λ` を読み捨てて `1` で回す実装は、既定値でしか測っていない限り観測できない。
    ///
    /// # ⚠ **票を立てる軸を1本混ぜてある**（作業単位 16g）
    ///
    /// `all` の薄れる重みが票の重みに比例するようになった（ADR-0027 訂正 4）ので、
    /// **票が1つも立たない回は `λ` をいくつにしても `0`** である。
    /// 見張り番の中身（**`λ` に比例する**）は変えていない。
    #[test]
    fn the_decay_strength_scales_the_fade() {
        let scale = replay_scale();
        let fade = |decay: f64| {
            let mut ledger = DriftLedger::new(
                DriftForm::All,
                DriftTuning::new(decay, 0.0, 1_000.0).expect("つまみが範囲内であること"),
                vec![
                    DriftAxisLedger::new(
                        axis("さざなみ"),
                        scale.value(6).expect("尺度に収まる値であること"),
                        0.0,
                        0,
                    ),
                    // **票を配る軸**（doc の ⚠）。
                    DriftAxisLedger::new(
                        axis("語られた"),
                        scale.value(3).expect("尺度に収まる値であること"),
                        0.0,
                        0,
                    ),
                ],
            );
            ledger
                .absorb(&[None, Some(DriftDelta::Up)])
                .expect("件数が合うこと");
            ledger.axes().first().expect("行があること").charge()
        };

        assert!(fade(0.0).abs() < 1e-12, "λ=0 で薄れてはいけない");
        let one = fade(1.0);
        let half = fade(0.5);
        assert!(one < 0.0, "λ=1 で薄れていない: {one}");
        assert!(
            (one * 0.5 - half).abs() < 1e-9,
            "λ が薄れる量に比例していない: {one} / {half}"
        );
        // λ=1 / 値 6/10 / 薄れる重み = 票の重み 0.1 ÷ N(=2)。
        assert!(
            (one - (-0.6 * 0.1 / 2.0)).abs() < 1e-12,
            "λ=1 の薄れる量が規則どおりでない: {one}"
        );
    }

    /// **費用は中点で対称、端で非対称**（ADR-0027 §9）。
    ///
    /// 中点で両向きとも `K` になっていることが「中間の振る舞いを一切乱さない」の中身で、
    /// 端で上下が入れ替わることが「上げ偏りへの経済的な対抗策」の中身である。
    #[test]
    fn the_cost_is_symmetric_only_at_the_midpoint() {
        let tuning = DriftTuning::new(1.0, 1.5, 1.0).expect("つまみが範囲内であること");
        let scale = 10;

        // 中点（5/10）。
        assert!((tuning.step_cost_of(5, scale, DriftDelta::Up) - 1.0).abs() < 1e-9);
        assert!((tuning.step_cost_of(5, scale, DriftDelta::Down) - 1.0).abs() < 1e-9);

        // 上端寄り（8/10）。**上げが高く、下げが安い。**
        let up_high = tuning.step_cost_of(8, scale, DriftDelta::Up);
        let down_high = tuning.step_cost_of(8, scale, DriftDelta::Down);
        assert!((up_high - 1.45).abs() < 1e-9, "上げの費用: {up_high}");
        assert!((down_high - 0.55).abs() < 1e-9, "下げの費用: {down_high}");
        assert!(down_high < up_high, "上端寄りで下げが安くなっていない");

        // 下端寄り（2/10）。**向きが入れ替わる。**
        let up_low = tuning.step_cost_of(2, scale, DriftDelta::Up);
        let down_low = tuning.step_cost_of(2, scale, DriftDelta::Down);
        assert!((up_low - 0.55).abs() < 1e-9, "上げの費用: {up_low}");
        assert!((down_low - 1.45).abs() < 1e-9, "下げの費用: {down_low}");
        assert!(up_low < down_low, "下端寄りで上げが安くなっていない");

        // **傾き 0 なら値付けは効かない**（費用は常に K）。
        let flat = DriftTuning::new(1.0, 0.0, 3.0).expect("つまみが範囲内であること");
        for value in 0..=scale {
            for delta in DriftDelta::ALL {
                assert!(
                    (flat.step_cost_of(value, scale, delta) - 3.0).abs() < 1e-9,
                    "傾き 0 で費用が動いた: {value} {delta:?}"
                );
            }
        }
    }

    /// **中点を基準に引かない式（両端から測る式）では、下げが消える**
    /// （ADR-0027 §9 の ⚠ が実測で残している失敗）。
    ///
    /// # なぜ「間違った式」を書いて比べるのか
    ///
    /// ADR は「一度対称に書いて**下げが 0 件になった**」と記録している。
    /// **その失敗を再現できる形で置いておかないと、
    /// 同じ書き換えが「なんとなく通る」変異になる。**
    ///
    /// # 見分けは「`K` より安くなる場所があるか」
    ///
    /// ADR が挙げた式は
    ///
    /// ```text
    /// 費用(上げ) = K × (1 + 傾き × v / 尺度)
    /// 費用(下げ) = K × (1 + 傾き × (尺度 − v) / 尺度)
    /// ```
    ///
    /// で、**どちらも `K` を下回らない**（中点では両方 `K × (1 + 傾き/2)`）。
    /// つまり**全域で `K` を上げたのと同じ**になり、
    /// §7 の表の「`K` を上げると下げが 0 件になる」に落ちる
    /// （見張り番は `a_larger_step_cost_makes_more_turns_quiet_and_removes_the_downward_steps`）。
    ///
    /// **中点を基準に引く正しい式は、片側が必ず `K` より安くなる。**
    /// そこが「下向きが出る」の供給源である。
    #[test]
    fn a_symmetric_cost_would_lose_every_downward_step() {
        let base = 1.0_f64;
        let slope = 1.5_f64;
        let tuning = DriftTuning::new(1.0, slope, base).expect("つまみが範囲内であること");
        let scale = 10_u32;

        // ADR が書いている「効かなかった式」。**両端から測っている。**
        let both_ends = |value: u32, delta: DriftDelta| {
            let ratio = match delta {
                DriftDelta::Up => f64::from(value) / f64::from(scale),
                DriftDelta::Down => (f64::from(scale) - f64::from(value)) / f64::from(scale),
            };
            base * (1.0 + slope * ratio)
        };

        // **中点で上げも下げも同じ値になり、しかも `K` より高い。**
        let mid_up = both_ends(5, DriftDelta::Up);
        let mid_down = both_ends(5, DriftDelta::Down);
        assert!((mid_up - mid_down).abs() < 1e-9, "両端から測ると中点で同値");
        assert!(mid_up > base, "しかも K より高い: {mid_up}");
        assert!(
            (tuning.step_cost_of(5, scale, DriftDelta::Up) - base).abs() < 1e-9,
            "正しい式は中点でちょうど K"
        );
        assert!(
            (tuning.step_cost_of(5, scale, DriftDelta::Down) - base).abs() < 1e-9,
            "正しい式は中点でちょうど K"
        );

        // **間違った式は、どこでも `K` より安くならない。**
        for value in 0..=scale {
            for delta in DriftDelta::ALL {
                assert!(
                    both_ends(value, delta) >= base - 1e-12,
                    "前提: 両端から測る式は K を下回らないこと: {value} {delta:?}"
                );
            }
        }

        // **正しい式は、上端寄りの下げと下端寄りの上げが `K` より安い。**
        assert!(
            tuning.step_cost_of(8, scale, DriftDelta::Down) < base,
            "上端寄りの下げが K より安くない（下向きの供給源が消える）"
        );
        assert!(
            tuning.step_cost_of(2, scale, DriftDelta::Up) < base,
            "下端寄りの上げが K より安くない（ゼロから育つのが遅くなる）"
        );
    }

    /// **累積が費用に達したら1段動き、費用ぶんだけ消える**（ADR-0027 §決定7）。
    ///
    /// **端数は残る。** 消したまま次のターンへ持ち越さない実装
    /// （累積を `0` に落とす変異）は、ここで赤くなる。
    #[test]
    fn a_step_consumes_exactly_its_cost_and_leaves_the_remainder() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // 費用は常に 0.05。0.01 + 1票（重み 0.1）で 2 段動いて、端数 0.01 が残る。
            DriftTuning::new(0.0, 0.0, 0.05).expect("つまみが範囲内であること"),
            vec![DriftAxisLedger::new(
                axis("階段"),
                scale.value(4).expect("尺度に収まる値であること"),
                0.01,
                0,
            )],
        );

        let effect = ledger
            .absorb(&[Some(DriftDelta::Up)])
            .expect("件数が合うこと");
        assert_eq!(effect.ups(), 2, "0.11 の累積で 0.05 の段が2つ動くこと");
        assert_eq!(effect.downs(), 0);
        assert!(effect.moved(), "動いたのに「なし」に数えられている");
        assert_eq!(effect.changes().len(), 1, "1軸ぶんは1件にまとめること");
        let change = effect.changes().first().expect("動いた軸があること");
        assert_eq!(change.before().get(), 4, "before はターン開始時の値");
        assert_eq!(change.after().get(), 6, "2段ぶん動くこと");
        assert_eq!(change.axis(), &axis("階段"), "動いた軸の帰属");

        let left = ledger.axes().first().expect("行があること").charge();
        assert!(
            (left - 0.01).abs() < 1e-9,
            "端数が残っていない（費用ぶんだけ消すこと）: {left}"
        );
    }

    /// **端では飽和する。値はクランプしない**（ADR-0024 §5 / ADR-0027 §決定5）。
    ///
    /// 上端の `+1` と**下端（`0`）の `-1`** の両方を、同じ一巡で見る。
    #[test]
    fn the_two_ends_saturate_instead_of_clamping() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            DriftTuning::new(0.0, 0.0, 0.05).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(
                    axis("上端"),
                    scale.value(REPLAY_SCALE).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("下端"),
                    scale.value(DRIFT_FLOOR).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
            ],
        );

        let effect = ledger
            .absorb(&[Some(DriftDelta::Up), Some(DriftDelta::Down)])
            .expect("件数が合うこと");

        assert_eq!(effect.ups(), 0, "上端で値が動いてはいけない");
        assert_eq!(effect.downs(), 0, "下端で値が動いてはいけない");
        assert!(effect.changes().is_empty(), "動いていないのに変更が出た");
        assert!(!effect.moved(), "飽和は「動いた」に数えない");
        assert!(
            effect
                .saturations()
                .contains(&(axis("上端"), DriftDelta::Up)),
            "上端の飽和が数えられていない: {:?}",
            effect.saturations()
        );
        assert!(
            effect
                .saturations()
                .contains(&(axis("下端"), DriftDelta::Down)),
            "下端の飽和が数えられていない: {:?}",
            effect.saturations()
        );

        let values: Vec<u32> = ledger
            .axes()
            .iter()
            .map(|entry| entry.value().get())
            .collect();
        assert_eq!(
            values,
            vec![REPLAY_SCALE, DRIFT_FLOOR],
            "値が動いてしまった"
        );
    }

    /// **下端は `0`。`0` へ落ちられる**（ADR-0027 §決定5）。
    ///
    /// # 「`0` から戻れること」とは**別の見張り番**にしてある
    ///
    /// 戻れる側は `domain::drift` の
    /// `a_zero_valued_axis_can_now_be_raised_from_the_floor_up` と、
    /// このモジュールの [`a_zero_valued_axis_can_climb_back_out`]。
    /// **1本にまとめると、`0` が片道になっている状態が緑を通る。**
    #[test]
    fn drift_can_now_reach_zero_at_the_bottom() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // **費用は 0.1。1票（重み 0.1）でちょうど1段**——2段目まで届かせると、
            // 下端での飽和が混ざって「落ちられたのか」が読めなくなる。
            DriftTuning::new(0.0, 0.0, 0.1).expect("つまみが範囲内であること"),
            vec![DriftAxisLedger::new(
                axis("最後の一歩"),
                scale.value(1).expect("尺度に収まる値であること"),
                0.0,
                0,
            )],
        );

        let effect = ledger
            .absorb(&[Some(DriftDelta::Down)])
            .expect("件数が合うこと");
        assert_eq!(effect.downs(), 1, "1 から 0 へ落ちられること");
        assert!(
            effect.saturations().is_empty(),
            "1 からの -1 が飽和になっている（下端が 1 のまま）"
        );
        let change = effect.changes().first().expect("動いた軸があること");
        assert_eq!(change.before().get(), 1);
        assert_eq!(change.after().get(), DRIFT_FLOOR);
        assert_eq!(
            change.after().get(),
            0,
            "下端の値そのものを直書きで押さえる"
        );
    }

    /// **`0` から戻れる**（ADR-0027 §決定5 の前提。落ちられることとは別の見張り番）。
    ///
    /// 上げの提案が来れば `0` の軸は上がる。**片道になっていないこと**が
    /// 下端を `0` へ開けた根拠そのものである。
    #[test]
    fn a_zero_valued_axis_can_climb_back_out() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            // **値付けを外す**（傾き 0）。傾き 2 だと `v=0` の上げが費用 0 になり、
            // 「提案が効いたのか、費用が 0 だったのか」が切り分けられない。
            DriftTuning::new(0.0, 0.0, 0.05).expect("つまみが範囲内であること"),
            vec![DriftAxisLedger::new(
                axis("ゼロから"),
                scale.value(DRIFT_FLOOR).expect("尺度に収まる値であること"),
                0.0,
                0,
            )],
        );

        let effect = ledger
            .absorb(&[Some(DriftDelta::Up)])
            .expect("件数が合うこと");
        assert_eq!(
            effect.ups(),
            2,
            "1票（重み 0.1）で 0.05 の段が2つ上がること"
        );
        let change = effect.changes().first().expect("動いた軸があること");
        assert_eq!(change.before().get(), DRIFT_FLOOR);
        assert_eq!(change.after().get(), 2);
        assert!(
            effect.saturations().is_empty(),
            "0 からの +1 が飽和になっている（0 が片道になっている）"
        );
    }

    /// **採点の件数が軸の本数と違えば、何も起きずに理由が返る。**
    ///
    /// 短い方に合わせる実装は、**採点が1つずれた状態**（軸 A の採点が軸 B に入る）を
    /// 静かに通す（CLAUDE.md「抜けるのはいつも帰属」）。
    #[test]
    fn a_scorecard_of_the_wrong_length_changes_nothing() {
        let scale = replay_scale();
        let axes = vec![
            DriftAxisLedger::new(
                axis("一本目"),
                scale.value(5).expect("尺度に収まる値であること"),
                0.0,
                0,
            ),
            DriftAxisLedger::new(
                axis("二本目"),
                scale.value(3).expect("尺度に収まる値であること"),
                0.0,
                0,
            ),
        ];
        let mut ledger = DriftLedger::new(DriftForm::All, DriftTuning::default(), axes.clone());

        for scores in [vec![], vec![Some(DriftDelta::Up)], vec![None, None, None]] {
            assert_eq!(
                ledger.absorb(&scores),
                Err(DriftLedgerError::Mismatched),
                "件数が {} の採点が通った",
                scores.len()
            );
        }
        assert_eq!(ledger.axes(), axes.as_slice(), "帳簿が動いてしまった");
    }

    /// **軸の並びと採点の並びが対応している**（CLAUDE.md「抜けるのはいつも帰属」）。
    ///
    /// **軸を取り違える実装**（採点を逆順に当てる／先頭に決め打つ）はここで赤くなる。
    /// 値も向きも本数も全部違う組にしてある。
    #[test]
    fn each_score_lands_on_the_axis_that_sits_at_the_same_position() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            DriftTuning::new(0.0, 0.0, 0.1).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(
                    axis("先頭"),
                    scale.value(4).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("真ん中"),
                    scale.value(6).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("末尾"),
                    scale.value(2).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
            ],
        );

        let effect = ledger
            .absorb(&[None, Some(DriftDelta::Down), None])
            .expect("件数が合うこと");
        assert_eq!(effect.changes().len(), 1, "動くのは1軸だけのはず");
        let change = effect.changes().first().expect("動いた軸があること");
        assert_eq!(change.axis(), &axis("真ん中"), "採点が別の軸に当たった");
        assert_eq!(change.before().get(), 6);
        assert_eq!(change.after().get(), 5);
        assert_eq!(change.delta(), DriftDelta::Down, "向きが逆になった");

        // 残り2本は値が動いていない。
        let values: Vec<u32> = ledger
            .axes()
            .iter()
            .map(|entry| entry.value().get())
            .collect();
        assert_eq!(values, vec![4, 5, 2]);
    }

    /// **薄れる重みは依頼の形で違う**（モジュール doc の表）。
    ///
    /// # 形を読み捨てる実装が、ここで赤くなる
    ///
    /// ADR の再現テストは**それぞれ片方の形しか通らない**ので、
    /// 「形によらず `w/N`」も「形によらず `w/(N−1)`」も**片方の再現だけは緑になる。**
    /// ここは**同じ採点を両方の形に通して、違う結果になること**を見る。
    ///
    /// **2つの違いは割る数だけ**（ADR-0027 訂正 4）なので、**比まで見る。**
    #[test]
    fn the_fade_weight_differs_between_the_two_request_forms() {
        let scale = replay_scale();
        let faded = |form: DriftForm| {
            let mut ledger = DriftLedger::new(
                form,
                DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
                (0..4)
                    .map(|index| {
                        DriftAxisLedger::new(
                            axis(&format!("軸{index}")),
                            scale.value(6).expect("尺度に収まる値であること"),
                            0.0,
                            0,
                        )
                    })
                    .collect(),
            );
            ledger
                .absorb(&[Some(DriftDelta::Up), None, None, None])
                .expect("件数が合うこと");
            ledger.axes().get(1).expect("2本目があること").charge()
        };

        // `single`: 立った票の重み（0.1）÷ (N−1) = 0.1/3。値 6/10 なので −0.02。
        let single = faded(DriftForm::Single);
        assert!(
            (single - (-0.6 * 0.1 / 3.0)).abs() < 1e-12,
            "single の薄れる重みが 票の重み/(N−1) になっていない: {single}"
        );
        // `all`: 立った票の重み（0.1）÷ N = 0.1/4。値 6/10 なので −0.015。
        let all = faded(DriftForm::All);
        assert!(
            (all - (-0.6 * 0.1 / 4.0)).abs() < 1e-12,
            "all の薄れる重みが 票の重み/N になっていない: {all}"
        );
        assert!(single < all, "2つの形で薄れる量が同じになっている");
        // **違いは割る数だけ**（ADR-0027 訂正 4 が2つを一つの規則に揃えた）。
        assert!(
            (single / all - 4.0 / 3.0).abs() < 1e-9,
            "2つの形の違いが「割る数」だけになっていない: {single} / {all}"
        );
    }

    /// **`single` で薄れる量は、その回に立った票の重みに連動する**（§決定3 の `w`）。
    ///
    /// **`w` を無視して `1/(N−1)` を固定で引く変異**は、
    /// ADR §9 の再現でも赤くなるが、**そこは 72 ターンの合算**なので
    /// 何が壊れたのかが読めない。ここは1ターンで直接見る。
    #[test]
    fn the_single_form_fade_follows_the_weight_of_the_vote_that_was_cast() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(
                    axis("選ばれる"),
                    scale.value(5).expect("尺度に収まる値であること"),
                    0.0,
                    // **すでに3回選ばれている**ので、次の票の重みは 0.1/√4 = 0.05。
                    3,
                ),
                DriftAxisLedger::new(
                    axis("選ばれない"),
                    scale.value(5).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
            ],
        );

        ledger
            .absorb(&[Some(DriftDelta::Up), None])
            .expect("件数が合うこと");
        let quiet = ledger.axes().get(1).expect("2本目があること").charge();
        // w = 0.1/√4 = 0.05、N−1 = 1、値 5/10 → −0.025。
        assert!(
            (quiet - (-0.025)).abs() < 1e-12,
            "薄れる量が票の重みに連動していない: {quiet}"
        );
        // **初回の票（重み 0.1）と取り違えると −0.05 になる。**
        assert!(
            (quiet - (-0.05)).abs() > 1e-6,
            "回数を数えずに初回の重みで薄れている: {quiet}"
        );
    }

    /// **票が1つも立たなかった回は、`single` では何も薄れない。**
    ///
    /// 「なし」と読める回に**全軸が薄れる**実装にすると、
    /// **モデルが黙った回が下向きの供給源になる**——ADR-0027 §決定2 は
    /// 「**選ばれなかった**軸が薄れる」であって「誰も選ばれなかったら全部薄れる」ではない。
    #[test]
    fn a_turn_with_no_vote_fades_nothing_in_the_single_form() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
            vec![
                DriftAxisLedger::new(
                    axis("片方"),
                    scale.value(9).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
                DriftAxisLedger::new(
                    axis("もう片方"),
                    scale.value(3).expect("尺度に収まる値であること"),
                    0.0,
                    0,
                ),
            ],
        );

        ledger.absorb(&[None, None]).expect("件数が合うこと");
        for entry in ledger.axes() {
            assert!(
                entry.charge().abs() < 1e-12,
                "票が無いのに薄れた: {} {}",
                entry.axis(),
                entry.charge()
            );
        }
    }

    /// **軸が1本しかなければ、薄れる相手が居ない**（`N−1 = 0` で割らない）。
    #[test]
    fn a_single_axis_character_never_divides_by_zero() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::Single,
            DriftTuning::default(),
            vec![DriftAxisLedger::new(
                axis("ひとつだけ"),
                scale.value(5).expect("尺度に収まる値であること"),
                0.0,
                0,
            )],
        );

        ledger.absorb(&[None]).expect("件数が合うこと");
        let charge = ledger.axes().first().expect("行があること").charge();
        assert!(charge.is_finite(), "累積が有限でなくなった: {charge}");
        assert!(
            charge.abs() < 1e-12,
            "薄れる相手が居ないのに減った: {charge}"
        );
    }

    // =====================================================================
    // つまみの検証
    // =====================================================================

    /// **既定値は ADR-0027 §決定8（16j が置き換えた値）。**
    ///
    /// **数を直書き**する——既定を変える変異は、
    /// ADR の再現テスト（つまみを明示して回している）では赤くならない。
    ///
    /// # ⚠ **3つの数は互いに違う値でなければならない**
    ///
    /// 同値にすると、**つまみを取り違えて代入する実装**（λ の欄に `K` を入れる等）が
    /// 観測できなくなる（CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
    /// `0.5` / `1.5` / `0.3` はその条件を満たしている。
    #[test]
    fn the_default_tuning_is_the_one_the_adr_settled_on() {
        let tuning = DriftTuning::default();
        assert!((tuning.decay() - 0.5).abs() < 1e-12, "λ の既定は 0.5");
        assert!((tuning.slope() - 1.5).abs() < 1e-12, "傾きの既定は 1.5");
        assert!((tuning.step_cost() - 0.3).abs() < 1e-12, "K の既定は 0.3");
        assert_eq!(
            tuning,
            DriftTuning::new(0.5, 1.5, 0.3).expect("つまみが範囲内であること"),
            "既定値が new を通した値と食い違っている"
        );
        // **公開している定数も同じ値を指していること**（設定側が使う）。
        assert!((DEFAULT_DRIFT_DECAY - 0.5).abs() < 1e-12);
        assert!((DEFAULT_DRIFT_SLOPE - 1.5).abs() < 1e-12);
        assert!((DEFAULT_DRIFT_STEP_COST - 0.3).abs() < 1e-12);
        // **3つが互いに別の値であること**（取り違えの検出が成り立つ前提）。
        assert_ne!(DEFAULT_DRIFT_DECAY, DEFAULT_DRIFT_SLOPE);
        assert_ne!(DEFAULT_DRIFT_DECAY, DEFAULT_DRIFT_STEP_COST);
        assert_ne!(DEFAULT_DRIFT_SLOPE, DEFAULT_DRIFT_STEP_COST);
        assert!(
            (MAX_DRIFT_SLOPE - 1.8).abs() < 1e-12,
            "傾きの上限は 1.8（**2 未満**でなければ費用が 0 になる）"
        );
        // **「2 未満」はビルドで止めてある**（`const _: () = assert!(...)`）。
        // ここに写すと、ビルド側を消しても緑のままになる。
        assert_ne!(
            MAX_DRIFT_SLOPE, DEFAULT_DRIFT_SLOPE,
            "上限と既定が同値だと、上限を既定で代用している実装が観測できない"
        );
    }

    /// **範囲の外は理由つきで拒否する。黙って既定へ落とさない。**
    ///
    /// 3つのつまみが**それぞれ別の理由**を返すこと（取り違えの検出）も見る。
    #[test]
    fn every_tuning_knob_rejects_its_own_kind_of_bad_value() {
        for bad in [-0.1_f64, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                DriftTuning::new(bad, 1.5, 1.0),
                Err(DriftTuningError::Decay),
                "λ = {bad} が通った"
            );
        }
        // **`2` は費用を `0` にする**（モジュール doc の ⚠）ので、上限は `2` 未満。
        for bad in [
            -0.1_f64,
            MAX_DRIFT_SLOPE + 0.01,
            1.9,
            2.0,
            2.000_001,
            f64::NAN,
            f64::INFINITY,
        ] {
            assert_eq!(
                DriftTuning::new(1.0, bad, 1.0),
                Err(DriftTuningError::Slope),
                "傾き = {bad} が通った"
            );
        }
        for bad in [0.0_f64, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                DriftTuning::new(1.0, 1.5, bad),
                Err(DriftTuningError::StepCost),
                "K = {bad} が通った"
            );
        }

        // 端はちょうど受ける。
        assert!(DriftTuning::new(0.0, 0.0, f64::MIN_POSITIVE).is_ok());
        assert!(DriftTuning::new(0.0, MAX_DRIFT_SLOPE, 1.0).is_ok());
        assert!(
            DriftTuning::new(1.0, 2.0, 1.0).is_err(),
            "傾き 2 が通った（費用 0 の段が作れる）"
        );

        // 3つの理由の綴りが重ならない（ログと応答で区別できること）。
        let mut messages = [
            DriftTuningError::Decay.to_string(),
            DriftTuningError::Slope.to_string(),
            DriftTuningError::StepCost.to_string(),
        ];
        let total = messages.len();
        messages.sort_unstable();
        let mut unique = messages.to_vec();
        unique.dedup();
        assert_eq!(unique.len(), total, "理由の綴りが重なっている");
    }

    /// **いちばん安い段が表現できない `K` は受け付けない**（レビュー指摘C / 16e-b）。
    ///
    /// # 傾きを塞いだのに、`K` の側から同じ状態に入れた
    ///
    /// `K = 5e-324`（最小の亜正規数）は `is_finite()` も `> 0.0` も通る。
    /// **しかし `K × (1 − 傾き/2)` はアンダーフローして `0`** になり、
    /// **費用 `0` の段**——潰したはずの「無償の1段」がそのまま復活する。
    /// **実害の形も同じ**（言われていない軸が勝手に上がる）。
    ///
    /// 実測（レビュアー / 16e-b）: `K = 5e-324` を通すと、
    /// **採点が全軸 `0` の1ターンで 値 0 → 3 / ups=3 / 飽和0** になった。
    ///
    /// # 受ける側も測る
    ///
    /// 拒否だけを見ると、**何でも拒否する実装**が緑を通る。
    /// **`f64::MIN_POSITIVE`（最小の正規化数）は受ける**ことと、
    /// **そのときの端の費用が本当に正である**ことを一緒に見る。
    ///
    /// # ⚠ **いちばん安い段は、いちばん高い尺度にある**（ADR-0027 訂正 2 / 作業単位 16g）
    ///
    /// 1段の費用が `K × (既定尺度/尺度)` で正規化されたので、
    /// **いちばん安い段は `K × (10/100) × (1 − 1.8/2)` = `K × 0.01`**。
    /// **`K × (1 − 傾き上限/2)` = `K × 0.1` だけで見ると、
    /// 「尺度 10 では正だが尺度 100 では `0`」という `K` が素通りする。**
    /// 傾きの側を [`MAX_DRIFT_SLOPE`] で見るのと同じ形で、**尺度は [`StatScale::MAX`] で見る。**
    #[test]
    fn a_step_cost_too_small_to_price_the_cheapest_step_is_refused() {
        // **アンダーフローする領域**（レビュアーが実際に通した値を含む）。
        //
        // **`1e-322` は、尺度の正規化が入る前は受けていた値**である
        // ——`1e-322 × 0.1` は亜正規数として表現できるが、`1e-322 × 0.01` は `0` に落ちる。
        for bad in [5e-324_f64, 1e-323, 2e-323, f64::from_bits(1), 1e-322] {
            assert_eq!(
                DriftTuning::new(1.0, 0.0, bad),
                Err(DriftTuningError::StepCost),
                "いちばん安い段が 0 になる K が通った: {bad:e}"
            );
        }

        // **受ける側**——受けたなら、端の費用は本当に正でなければならない。
        //
        // **上端は `1e300` で止める**（`f64::MAX / 4.0` ではない）。
        // [`DriftTuning::step_cost_of`] は `K × 既定尺度 / 尺度` と**掛けてから割る**ので、
        // **`K > f64::MAX / 10` では尺度に依らず費用が `inf` へ溢れる。**
        // 溢れる向きは「段が一切動かない」＝**安全な側**なので門は設けていない
        // （[`DriftTuning::new`] の doc）。**この見張り番は「費用が正で有限」を言う**ので、
        // 上端の領域は `a_step_cost_so_large_that_the_price_overflows_simply_never_moves` が持つ。
        for good in [f64::MIN_POSITIVE, 1e-300_f64, 1.0, 1e300] {
            let tuning = DriftTuning::new(1.0, MAX_DRIFT_SLOPE, good)
                .unwrap_or_else(|err| panic!("K = {good:e} を受けること: {err}"));
            for scale in [StatScale::MIN, 7, StatScale::MAX] {
                let cheapest = tuning.step_cost_of(0, scale, DriftDelta::Up);
                assert!(
                    cheapest > 0.0 && cheapest.is_finite(),
                    "受けた K で費用が 0 になった: K={good:e} 尺度{scale} -> {cheapest}"
                );
            }
        }

        // **受理と「費用が正」が一致する。**
        //
        // 境界の `K` を目分量で書かずに、**亜正規数の下端から桁を上げながら**
        // 「受けたなら正／拒んだなら 0」を両方向で確かめる
        // ——**どちらに倒れても矛盾しない**ことだけを言うので、
        // 境界そのものが動いても壊れない。
        let mut checked_both = (false, false);
        for shift in 0..64 {
            let candidate = f64::from_bits(1_u64 << shift);
            if !candidate.is_finite() || candidate <= 0.0 {
                continue;
            }
            // **いちばん安い段は「いちばん高い尺度 × 上限の傾き」にある**（doc の ⚠）。
            let cheapest = candidate
                * (f64::from(StatScale::DEFAULT.get()) / f64::from(StatScale::MAX))
                * (1.0 - MAX_DRIFT_SLOPE / 2.0);
            match DriftTuning::new(1.0, MAX_DRIFT_SLOPE, candidate) {
                Ok(tuning) => {
                    checked_both.0 = true;
                    // **尺度 10 だけでなく、いちばん高い尺度でも正であること。**
                    for scale in [StatScale::MIN, 10, StatScale::MAX] {
                        assert!(
                            tuning.step_cost_of(0, scale, DriftDelta::Up) > 0.0,
                            "受けた K の端の費用が 0 だった: K={candidate:e} 尺度{scale}"
                        );
                    }
                }
                Err(reason) => {
                    checked_both.1 = true;
                    assert_eq!(reason, DriftTuningError::StepCost);
                    assert!(
                        cheapest <= 0.0,
                        "費用が正なのに拒んだ: K={candidate:e} -> {cheapest:e}"
                    );
                }
            }
        }
        assert_eq!(
            checked_both,
            (true, true),
            "受ける側と拒む側の両方を通っていない（境界を跨いでいない）"
        );

        // **門は `K` の側にしかない**——傾きが `0` でも同じ `K` を拒む
        // （傾きの実値ではなく [`MAX_DRIFT_SLOPE`] で見ているため）。
        assert_eq!(
            DriftTuning::new(1.0, 0.0, 5e-324),
            DriftTuning::new(1.0, MAX_DRIFT_SLOPE, 5e-324),
            "傾きによって K の受理が変わっている（門が上限で見ていない）"
        );
    }

    /// **`K` が大きすぎて費用が `inf` になる領域では、段が一切動かない**
    /// ——**`NaN` も飽和も出ない**（作業単位 16g / レビュー指摘4）。
    ///
    /// # なぜ上端には門を作らないのか
    ///
    /// **小さい側と大きい側は、危険の質が逆である。**
    ///
    /// * **費用 `0` は「止まる」ではなく「無償で動く」** ——
    ///   言われていない軸が勝手に上がる。**振る舞いが壊れる。**
    /// * **費用 `inf` は「止まる」だけ**で、**`K = 1e6` でも同じことが起きる。**
    ///   **連続的に遅くなっていく先の端なので、そこに引く線が無い。**
    ///
    /// **だから門ではなく、振る舞いを言う見張り番を置く。**
    ///
    /// # ⚠ **`inf` になるのは尺度 1 だけではない**
    ///
    /// [`DriftTuning::step_cost_of`] は `K × 既定尺度 / 尺度` と**掛けてから割る**ので、
    /// **`K > f64::MAX / 10 ≒ 1.797e307` では尺度に依らず全域で `inf`** になる。
    ///
    /// # `NaN` の入口が塞がっていること
    ///
    /// `inf × 0` は `NaN` だが、**`(1 ± 傾き)` は傾きの上限 1.8 のもとで最小 `0.1`**
    /// （[`the_cheapest_step_still_costs_a_tenth_of_the_base`]）なので、
    /// **`0` を掛ける経路が存在しない。**
    /// `NaN` になると `累積 >= 費用` も `累積 <= -費用` も偽になって静かに止まるので、
    /// **「動かない」だけを見ていると `NaN` と区別が付かない。** 費用そのものも見る。
    #[test]
    fn a_step_cost_so_large_that_the_price_overflows_simply_never_moves() {
        // **上端の領域**（`f64::MAX / 10` を越える側）。`f64::MAX` そのものを含む。
        for huge in [f64::MAX / 8.0, f64::MAX / 2.0, f64::MAX] {
            let tuning = DriftTuning::new(1.0, DEFAULT_DRIFT_SLOPE, huge)
                .unwrap_or_else(|err| panic!("K = {huge:e} を受けること: {err}"));

            // **費用は `inf`。ただし `NaN` ではない。**
            for scale in [StatScale::MIN, 10, StatScale::MAX] {
                for value in [0, scale / 2, scale] {
                    for delta in DriftDelta::ALL {
                        let cost = tuning.step_cost_of(value, scale, delta);
                        assert!(
                            !cost.is_nan(),
                            "費用が NaN になった: K={huge:e} 尺度{scale} 値{value} {delta:?}"
                        );
                        assert!(
                            cost > 0.0,
                            "費用が 0 以下になった: K={huge:e} 尺度{scale} 値{value} {delta:?} -> {cost}"
                        );
                        assert!(
                            cost.is_infinite(),
                            "前提: この領域では費用が溢れること: K={huge:e} 尺度{scale} -> {cost}"
                        );
                    }
                }
            }

            // **帳簿を回しても、段は一切動かない。** 飽和もしない（費用を消費できない）。
            let scale = StatScale::parse(StatScale::MAX).expect("正当な尺度であること");
            let mut ledger = DriftLedger::new(
                DriftForm::All,
                tuning,
                // **端と中間**を混ぜる。上端・下端は飽和の枝に入りうる位置である。
                [0_u32, 50, StatScale::MAX]
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        DriftAxisLedger::new(
                            axis(&format!("溢れ{index}")),
                            scale.value(value).expect("尺度に収まる値であること"),
                            0.0,
                            0,
                        )
                    })
                    .collect(),
            );

            for scores in [
                [Some(DriftDelta::Up), Some(DriftDelta::Up), None],
                [None, Some(DriftDelta::Down), Some(DriftDelta::Down)],
                [None, None, None],
            ] {
                let effect = ledger.absorb(&scores).expect("件数が合うこと");
                assert_eq!(effect.ups(), 0, "K={huge:e} で段が上がった");
                assert_eq!(effect.downs(), 0, "K={huge:e} で段が下がった");
                assert!(
                    effect.saturations().is_empty(),
                    "K={huge:e} で飽和した（費用を消費できないはず）: {:?}",
                    effect.saturations()
                );
                assert!(!effect.moved(), "K={huge:e} で動いたことになっている");
            }

            // **累積は有限のまま残る**（票と薄れは `K` に依らない）。
            for entry in ledger.axes() {
                assert!(
                    entry.charge().is_finite(),
                    "累積が有限でなくなった: K={huge:e} {} -> {}",
                    entry.axis(),
                    entry.charge()
                );
            }
            let values: Vec<u32> = ledger.axes().iter().map(|row| row.value().get()).collect();
            assert_eq!(values, vec![0, 50, StatScale::MAX], "値が動いてしまった");
        }

        // **境目の手前は溢れない**（「何を渡しても `inf`」という実装と区別する）。
        let modest = DriftTuning::new(1.0, DEFAULT_DRIFT_SLOPE, 1e300).expect("受けること");
        assert!(
            modest
                .step_cost_of(0, StatScale::MAX, DriftDelta::Up)
                .is_finite(),
            "溢れない K まで `inf` にしている"
        );
    }

    /// **費用は、どの尺度・どの傾き・どの値でも `0` より大きい**
    /// （ADR-0027「訂正（2026-08-24）」/ 訂正 2 / 作業単位 16e-b・16g）。
    ///
    /// # 費用 `0` は「言われていない軸が動く」ことである
    ///
    /// 段は `累積 >= 費用` で動く。**費用が `0` なら、累積が `0` のままでも動く。**
    /// 薄れる量は `λ × (現在値/尺度) × 重み` なので、**値 0 の軸は一切薄れない**
    /// ——累積は `0` に留まり、**毎ターン 1 段上がり続ける。**
    /// 運用者が「語らない」と宣言した軸が、変容が走っている限り黙って現れる。
    ///
    /// # 危ないのは端の2点だけだが、全域を走る
    ///
    /// `費用(0, 上げ)` と `費用(尺度, 下げ)` がその尺度の最小で、どちらも
    /// **`基準 × (1 − 傾き/2)`**（基準 = `K × (既定尺度/尺度)`）。
    ///
    /// # ⚠ **最小費用は尺度に依らない、ではない**（ADR-0027 訂正 2 / 16g）
    ///
    /// `(1 − 傾き/2)` の**係数**は尺度に依らない（中点が常に `尺度/2` なので
    /// `(0 − 尺度/2)/尺度` は尺度によらず `−1/2`）が、
    /// **基準そのものが尺度に反比例する**ので、**最小費用は尺度で変わる。**
    /// 尺度 100 では `K × 0.01` まで下がる ——**そこが `0` に落ちるかどうかを
    /// 見張っているのが [`DriftTuning::new`] の門**（`a_step_cost_too_small_to_price_the_cheapest_step_is_refused`）。
    ///
    /// **ここは「尺度で割るのを忘れる変異」を捕まえない。**
    /// 割り忘れても費用は正のままなので、この網羅は緑を通る。
    /// **その変異の権威は `a_step_costs_the_same_share_of_the_range_at_every_scale` と
    /// `the_same_conversation_moves_the_same_share_of_the_range_at_scale_one_hundred`。**
    /// ここが見ているのは**符号だけ**である——だから尺度の全域を走る必要がある
    /// （「中点を定数にする」変異は特定の尺度でしか符号を割らない）。
    ///
    /// # 上限そのものを緩める変異が、ここで赤くなる
    ///
    /// [`MAX_DRIFT_SLOPE`] を `2.0` に戻すと `費用(0, 上げ) = 0` が**作れてしまう**
    /// ので、この網羅が赤になる。**綴りは
    /// `the_default_tuning_is_the_one_the_adr_settled_on` が直書きで押さえ、
    /// なぜその値なのかはここが押さえる。**
    #[test]
    fn the_cost_never_reaches_zero_for_any_scale_or_slope() {
        // **許される傾きの端**と、その手前・既定・値付けなし。
        let slopes = [0.0_f64, 0.5, DEFAULT_DRIFT_SLOPE, MAX_DRIFT_SLOPE];
        // **`K` も振る**（費用は `K` に比例するので、`K` が小さいほど 0 に近づく）。
        let costs = [f64::MIN_POSITIVE, 0.01_f64, 1.0, 100.0];

        for scale in StatScale::MIN..=StatScale::MAX {
            for slope in slopes {
                for step_cost in costs {
                    let tuning = DriftTuning::new(1.0, slope, step_cost)
                        .expect("端まで含めて受けられること");
                    for value in 0..=scale {
                        for delta in DriftDelta::ALL {
                            let cost = tuning.step_cost_of(value, scale, delta);
                            assert!(
                                cost > 0.0,
                                "費用が 0 以下になった: 尺度{scale} 値{value} 傾き{slope} \
                                 K={step_cost} {delta:?} -> {cost}"
                            );
                            assert!(cost.is_finite(), "費用が有限でない: {cost}");
                        }
                    }
                }
            }
        }
    }

    /// **端の2点の費用が、`K` の1割を下回らない**（[`MAX_DRIFT_SLOPE`] の置き場の根拠）。
    ///
    /// # 「正であること」だけでは足りない
    ///
    /// 上限を `1.999…` にしても費用は正のままだが、
    /// **いちばん安い段が `K` の 1/2000 になる**ので、端では実質いくらでも段が動く
    /// ——「無償の1段」が「ほぼ無償の 2000 段」に化けるだけである。
    ///
    /// **上限は「表現できる境界」ではなく「測りたい範囲」として置いた**
    /// （[`MAX_DRIFT_SLOPE`] の doc）。ここはその置き場そのものを言う。
    ///
    /// # ⚠ **「基準」は `K` ではなく `K × (既定尺度/尺度)`**（ADR-0027 訂正 2 / 16g）
    ///
    /// 正規化が入る前は「いちばん安い段は**どの尺度でも** `K × 0.1`」だった。
    /// **いまは尺度に反比例する**（尺度 100 なら `K × 0.01`）。
    /// **1割という比そのものは尺度に依らない**ので、
    /// **その尺度の中点の費用（＝基準）と比べる形**に置き直してある。
    ///
    /// **「尺度に依らず 0.1」を戻さないこと。** 戻すと訂正 2 が撤回される
    /// ——尺度 100 のキャラクターが同じ会話でほとんど動かない状態に帰る。
    #[test]
    fn the_cheapest_step_still_costs_a_tenth_of_the_base() {
        let edge = DriftTuning::new(1.0, MAX_DRIFT_SLOPE, 1.0).expect("上限は受けること");
        for scale in StatScale::MIN..=StatScale::MAX {
            let cheapest_up = edge.step_cost_of(0, scale, DriftDelta::Up);
            let cheapest_down = edge.step_cost_of(scale, scale, DriftDelta::Down);
            // **その尺度の基準**＝上げと下げの平均（傾きの項が打ち消える）。
            // **中点の費用で取り出さない**——尺度が奇数だと `尺度/2` が整数にならず、
            // **「中点」を整数で名指しできない。**
            let base = (edge.step_cost_of(0, scale, DriftDelta::Up)
                + edge.step_cost_of(0, scale, DriftDelta::Down))
                / 2.0;
            for (label, cost) in [("上げ", cheapest_up), ("下げ", cheapest_down)] {
                assert!(
                    cost >= base * 0.1 - 1e-12,
                    "尺度{scale} の端の{label}が基準の1割を下回った: {cost} / 基準 {base}"
                );
            }
            // **上端と下端で同じ**（中点が常に `尺度/2` である帰結。ここは尺度に依らない）。
            assert!(
                (cheapest_up - cheapest_down).abs() < 1e-12,
                "上端と下端で最小費用が違う: 尺度{scale} {cheapest_up} vs {cheapest_down}"
            );
            // **比はちょうど 1割**——上限を緩めるとここが割れる。
            assert!(
                (cheapest_up / base - 0.1).abs() < 1e-9,
                "尺度{scale} で「基準の1割」から外れた: {cheapest_up} / {base}"
            );
            // **基準そのものが尺度に反比例している**（訂正 2）。
            assert!(
                (base * f64::from(scale) - 10.0).abs() < 1e-9,
                "尺度{scale} の基準が K×(10/尺度) でない: {base}"
            );
        }

        // **既定はもっと高い**（既定尺度で `K × 0.25` ＝ `0.3 × 0.25` ＝ `0.075`）。
        // **上限で組むと `K × 0.1` ＝ `0.03`** なので、
        // **上限と既定を取り違える実装はここで落ちる。**
        //
        // **比ではなく数を直書きする**——`K` の既定から作ると、
        // `K` を変える変異が期待値を道連れにする（CLAUDE.md「関数の外にある値で押さえる」）。
        let default = DriftTuning::default();
        assert!(
            (default.step_cost_of(0, 10, DriftDelta::Up) - 0.075).abs() < 1e-12,
            "既定のいちばん安い段が K × 0.25（＝ 0.075）でない: {}",
            default.step_cost_of(0, 10, DriftDelta::Up)
        );
    }

    /// **DB から有限でない累積を読んでも、帳簿は壊れない。**
    #[test]
    fn a_charge_that_is_not_finite_is_folded_to_zero() {
        let scale = fixture_scale();
        let value = scale.value(1).expect("尺度に収まる値であること");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let entry = DriftAxisLedger::new(axis("壊れた行"), value, bad, 0);
            assert!(
                entry.charge().abs() < 1e-12,
                "有限でない累積がそのまま入った: {bad}"
            );
        }
        // 有限ならそのまま持つ（**何でも 0 にする実装**と区別する）。
        let kept = DriftAxisLedger::new(axis("ふつうの行"), value, -0.25, 7);
        assert!((kept.charge() - (-0.25)).abs() < 1e-12);
        assert_eq!(kept.scored(), 7);
        assert_eq!(kept.value().get(), 1);
        assert_eq!(kept.axis(), &axis("ふつうの行"));
    }

    /// **値付けを外すと、ゼロ始まりが下端に貼り付く**
    /// （ADR-0027「ゼロから育てるには、値付け（§9）が必須だった」）。
    ///
    /// # ここが §10「減衰と値付けは別物。両方要る」の片側
    ///
    /// 減衰（§決定2）は**下向きの唯一の供給源**だが、
    /// **値 0 の軸に `-1` が来ると下端で飽和する。**
    /// 値付けは「低い値を下げるのは高くつく」を作るので、その飽和が消える。
    ///
    /// # ⚠ **1008 ターン回す**（作業単位 16g）
    ///
    /// **票の重みが `0.1/√n` になった**（ADR-0027 訂正 3）ので、
    /// **72 ターンでは値付けの有無どちらも飽和が `0`** ——
    /// **この見張り番が見ようとしている状態に、一度も入らなくなる。**
    /// 「下端に貼り付く」は**貼り付くまで回して初めて観測できる。**
    ///
    /// **数の出どころは ADR-0027「訂正 5」の「ゼロから育てる形」の表**
    /// （同じ道具 `scratchpad/m19/nr.awk` が出したもの）で、
    /// **実装から作った値ではない。**
    /// 訂正 5 の ⚠ にも「**72 ターンでは飽和 0 になり、この見張り番が見るべき状態に
    /// 一度も入らない**」と書いてある。
    /// 値付けありの側は
    /// [`the_thousand_turn_trajectory_matches_the_table_the_other_tool_produced`] も持つ。
    #[test]
    fn without_the_pricing_a_zero_start_saturates_at_the_floor() {
        // **72 ターンでは足りない**（doc の ⚠）。同じ採点表を 14 周させる。
        let turns: Vec<Vec<Option<DriftDelta>>> = std::iter::repeat_n(all_turns(), LONG_RUN_PASSES)
            .flatten()
            .collect();
        assert_eq!(turns.len(), 1008, "72 × 14 = 1008 ターンであること");

        let flat = run_replay(
            ledger_for(
                DriftForm::All,
                &ALL_AXES,
                0,
                DriftTuning::new(1.0, 0.0, 1.0).expect("つまみが範囲内であること"),
            ),
            &turns,
            0,
        );
        assert_eq!(
            flat.saturations, 2,
            "ADR-0027 訂正 5「ゼロから育てる形」の値付けなし（飽和 2）と食い違っている: {flat:?}"
        );
        assert_eq!(
            flat.total_after, 10,
            "ADR-0027 訂正 5「ゼロから育てる形」の値付けなし（0→10）と食い違っている: {flat:?}"
        );
        assert_eq!(flat.values, vec![4, 0, 1, 0, 5], "値付けなしの着地点");

        // **値付けを入れると、下端の飽和が 2 から 1 へ落ちる。**
        //
        // 値付けは「低い値を下げるのは高くつく」を作るので、
        // **値 0 の軸に `-1` が来ても下端に留まりにくい。**
        let priced = run_replay(
            ledger_for(
                DriftForm::All,
                &ALL_AXES,
                0,
                DriftTuning::new(1.0, 1.5, 1.0).expect("つまみが範囲内であること"),
            ),
            &turns,
            0,
        );
        assert_eq!(
            priced.saturations, 1,
            "ADR-0027 訂正 5「ゼロから育てる形」の値付けあり（飽和 1）と食い違っている: {priced:?}"
        );
        assert_eq!(
            priced.total_after, 17,
            "ADR-0027 訂正 5「ゼロから育てる形」の値付けあり（上げ17 / 0→17）と食い違っている: {priced:?}"
        );
        assert_eq!(priced.values, vec![6, 0, 3, 1, 7], "値付けありの着地点");
        assert!(
            priced.saturations < flat.saturations,
            "値付けを入れても下端の貼り付きが減らない: {} vs {}",
            priced.saturations,
            flat.saturations
        );
        assert!(
            priced.total_after > flat.total_after,
            "値付けを入れても育ちが速くならない: {} vs {}",
            priced.total_after,
            flat.total_after
        );
    }

    /// **ゼロから育つ。** 総和が増え、飽和が出ない（ADR-0027 §11）。
    ///
    /// # 上の再現テストと重ならないのか
    ///
    /// あちらは**表を丸ごと突き合わせる**ので、どれか1つが動けば赤くなる。
    /// ここは**「ゼロ始まりが動かない床にならない」という主張そのもの**を、
    /// 短い入力で名指しする——§決定3 が総和保存を却下した理由である
    /// （総和保存では**実測で飽和 4**、ゼロ始まりは育つ前に止まった）。
    #[test]
    fn a_character_that_starts_at_zero_grows_without_saturating() {
        let scale = replay_scale();
        let mut ledger = DriftLedger::new(
            DriftForm::All,
            DriftTuning::default(),
            (0..3)
                .map(|index| {
                    DriftAxisLedger::new(
                        axis(&format!("ゼロ{index}")),
                        scale.value(0).expect("尺度に収まる値であること"),
                        0.0,
                        0,
                    )
                })
                .collect(),
        );

        // **1本だけが上げの提案を受け、残り2本は「選ばれなかった」側。**
        let mut saturations = 0;
        for _ in 0..10 {
            let effect = ledger
                .absorb(&[Some(DriftDelta::Up), None, None])
                .expect("件数が合うこと");
            saturations += effect.saturations().len();
        }

        let values: Vec<u32> = ledger
            .axes()
            .iter()
            .map(|entry| entry.value().get())
            .collect();
        let total: u32 = values.iter().sum();
        assert!(total > 0, "ゼロから育っていない: {values:?}");
        assert_eq!(saturations, 0, "ゼロ始まりで飽和した: {values:?}");
        assert!(
            values.iter().all(|value| *value <= scale.get()),
            "尺度の外へ出た: {values:?}"
        );
        // **票が集まった軸の方が高くなる**（希少度の重みがあっても、
        // 10票の軸と0票の軸は開く / ADR-0027 §11「ゼロから育てると分化する」）。
        let voted = values.first().copied().unwrap_or_default();
        let quiet = values.get(1).copied().unwrap_or_default();
        assert!(
            voted > quiet,
            "票が入った軸と入らなかった軸が分化していない: {values:?}"
        );
    }
}
