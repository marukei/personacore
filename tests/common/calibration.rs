//! **校正の器具の「走らせなくても確かめられる部分」**（作業単位 E-3）。
//!
//! # なぜ切り出したのか
//!
//! 器具の本体（`tests/prompt_estimate_calibration.rs`）は**4本とも `#[ignore]`** で、
//! **LM Studio の起動が要る。** 上流に繋がる部分は素の `cargo test` では動かせないので、
//! **上流を要らない部分だけをここへ集め、`tests/calibration_target.rs` が見張る。**
//!
//! ここに置いてよいのは**純粋関数だけ**（I/O も時刻も乱数も見ない）。
//!
//! # 何を決めているか
//!
//! | もの | 決めること |
//! |---|---|
//! | [`calibration_target`] | **どのモデルを測るか** |
//! | [`calibration_body`] | 上流へ送る依頼（`model` は対象から来る） |
//! | [`measured_on_note`] | 表を測ったモデルと、いま測る相手がずれていたら言う |
//! | [`golden_block`] / [`overhead_block`] | **そのまま貼れる形**の出力 |
//! | [`assembled_shape`] | 組み上がった行から**防御なしの行**を作る（上流を呼ばない / B-25） |
//! | [`probe_shape`] | **測った値をどの欄へ入れるか**（配線 / B-25） |
//! | [`ShapeProbe`] | **崩した形の2点を、判定してよいかどうかごと**決める（B-25） |
//! | [`shape_block`] | 実測2行と判定1行の塊（**報告を落とす変異をここで止める** / B-25） |
//! | [`lookalike_side`] | 対照が**規則の内側か外側か**（無害化そのものに訊く / B-27） |
//! | [`LookalikeProbe`] | 対照が特殊になったときの**直す先を、側ごとに**決める（B-27） |
//!
//! # ⚠ ここが緑でも「別のモデルを測れた」とは言えない
//!
//! **言えるのは「対象を指せる」まで。** 指した綴りで上流が本当に答えるか
//! （そのモデルが LM Studio に入っているか、トークナイザが別系列かどうか）は、
//! **実機でしか分からない。** 2026-08-28 時点で、実機で確かめた者はまだ居ない。

use personacore::llm::{DEFAULT_MAIN_MODEL, ModelId, ModelIdError};
use personacore::prompt::{
    MEASURED_CONTROL_TOKEN_SPELLINGS, MEASURED_MODEL_DEPENDENT_SPELLINGS, SafeText,
};
use serde_json::{Value, json};

/// **校正の対象モデル**を指す環境変数名。
///
/// # なぜ本番の振り分けと別の綴りなのか
///
/// `PERSONACORE_MAIN_MODEL` / `PERSONACORE_SUB_MODEL` は
/// **本番でどのモデルに何をさせるか**を決めるつまみで、
/// **「いま何を測るか」ではない。**
///
/// あちらを読むと、**サブモデルを差し替えただけで校正の対象が黙って動く。**
/// それは E-3 が直した欠陥（**校正が常にメインを測る**）と
/// **向きが逆なだけの同じ事故**である——どちらも
/// 「走らせた」と「確かめた」がずれる（ADR-0033 §決定4 の追記）。
///
/// **だから校正の対象は、校正のための変数ひとつだけが決める。**
/// 見張り番は `tests/calibration_target.rs` の
/// `the_production_model_variables_do_not_move_the_calibration_target` と
/// `only_the_calibration_variable_is_ever_read`。
pub const ENV_CALIBRATION_MODEL: &str = "PERSONACORE_CALIBRATION_MODEL";

/// **どのモデルを測るか**を決める。**純粋関数**（環境そのものは呼び出し側が渡す）。
///
/// * 未設定なら [`DEFAULT_MAIN_MODEL`]。**既定は E-3 の前と同じ。**
/// * 設定されていれば、その綴りを [`ModelId::parse`] に通す。
/// * **読めない綴りは `Err`。黙って既定へ落とさない**——落とすと
///   「別のモデルを指したつもりで既定を測り続ける」が、綴り間違いのたびに再現する。
///   空白だけ・空文字も `Err`（`PERSONACORE_SUB_MODEL` と同じ作法）。
///
/// # ⚠ 既定側の帰属は、誰も押さえていない
///
/// **`DEFAULT_MAIN_MODEL` と `DEFAULT_SUB_MODEL` は今日は同じ綴り**
/// （`ministral-3-14b-instruct-2512`）。したがって
/// **「未設定のときサブの既定を返す」変異は、原理的に検出できない**——
/// **値が同一なので、どの見張り番にも掛からない**（E-3 で実測: この行を
/// メインの既定へ決め打つ変異と、サブの既定へ決め打つ変異は、**同じ4本を赤にした**）。
///
/// **押さえられているのは「未設定なら既定へ落ちる」ことまで。**
/// **「その既定がメインの側である」ことは、2つの定数の綴りが分かれるまで誰も見ていない。**
/// CLAUDE.md の最頻の穴（**フィクスチャが2つの値を同値にすると、その取り違えは検出できない**）が、
/// **定数の層で出た形**である。**E-3 は定数を触っていない**（値を変えることは別の話）。
pub fn calibration_target<F>(lookup: F) -> Result<ModelId, ModelIdError>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = lookup(ENV_CALIBRATION_MODEL).unwrap_or_else(|| DEFAULT_MAIN_MODEL.to_owned());
    ModelId::parse(&raw)
}

/// プロセスの環境変数から対象を決める。**環境に触るのはこの1行だけ。**
///
/// # ⚠ この1行だけは、環境を書き換える腕で見張っている
///
/// 見張り番は **`tests/calibration_env_wiring.rs`（テスト1本だけの専用バイナリ）**。
///
/// **⚠ なぜ普通のテストに混ぜないのか。** [`calibration_target`] を見る 11 本は
/// **参照関数を渡す形**で、**プロセスの環境に一切触らない**
/// （`src/server/config.rs` の doc:「`std::env::set_var` は並行するテストを壊す」）。
/// **その代わり、11 本はどれもこの1行を通らない。**
///
/// **⚠ なぜ専用バイナリなのか、実測で何が緑を通ったかは、あちらの doc に書いてある**
/// （**写しを作らない** / CLAUDE.md「同じ発見を2箇所に書かない。片方を直しても、
/// もう片方が黙って古びる」）。
pub fn calibration_target_from_env() -> Result<ModelId, ModelIdError> {
    calibration_target(|key| std::env::var(key).ok())
}

/// 上流へ送る依頼。**`model` は必ず対象から来る**（決め打ちを作らない）。
///
/// `max_tokens = 1` で投げて `usage.prompt_tokens` だけを読む
/// （測り方は `tests/prompt_estimate_calibration.rs` の doc）。
pub fn calibration_body(target: &ModelId, messages: Value) -> Value {
    json!({
        "model": target.as_str(),
        "messages": messages,
        "stream": false,
        "max_tokens": 1,
    })
}

/// **表を測ったモデルと、いま測る相手が違うなら注意書きを返す**（ADR-0033 §決定1）。
///
/// 同じなら `None`——**毎回鳴る注意書きは読まれなくなる。**
///
/// # なぜ要るのか
///
/// 対象を指せるようになった瞬間、**「この数はどのモデルで出たのか」が
/// 出力を見ただけでは分からなくなる。** 別のモデルで出した値を
/// `ministral-3-14b-instruct-2512` と書いてある表へ貼れば、
/// **表は静かに嘘になる。**
pub fn measured_on_note(measured_on: &str, target: &ModelId) -> Option<String> {
    if measured_on == target.as_str() {
        return None;
    }

    Some(
        [
            format!("⚠ 表の値は {measured_on} で測ったものです。いま測っているのは {target} です。"),
            "⚠ ずれても「上流が変わった」とは限りません。貼り替えるなら、貼り先の doc の実測モデルも直すこと。"
                .to_owned(),
        ]
        .join("\n"),
    )
}

/// **`GOLDEN` / `GOLDEN_BASELINE` を、そのまま貼れる形**に組み立てる。
///
/// # なぜ文字列を返すのか（`println!` を並べないのか）
///
/// **cargo は同じバイナリのテストを並行に走らせる。** `#[ignore]` の4本を
/// `--ignored --nocapture` で回すと**出力が混ざる**ので、行ごとに書くと
/// 「そのまま貼れる」が崩れる。**1本の文字列にして `println!` 1回で出せば、
/// 標準出力の錠が1回で済み、塊が割れない。**
///
/// **注意書きは貼る範囲の外（目印より前）に置く。** 中に入れると貼った先が壊れる。
pub fn golden_block(
    target: &ModelId,
    measured_on: &str,
    baseline: u32,
    rows: &[(&str, u32)],
) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(note) = measured_on_note(measured_on, target) {
        lines.push(note);
    }

    lines.push(format!(
        "--- ここから下を GOLDEN に貼る（実測: {target}） ---"
    ));
    lines.push(format!(
        "const GOLDEN_BASELINE: Option<u32> = Some({baseline});"
    ));
    for (text, measured) in rows {
        lines.push(format!("    ({text:?}, Some({measured})),"));
    }
    lines.push("--- ここまで ---".to_owned());

    lines.join("\n")
}

/// **`MESSAGE_OVERHEAD_TOKENS` を、そのまま貼れる形**に組み立てる。
///
/// # ⚠ 下げる向きには貼らせない
///
/// この定数は**メッセージ1件あたりの固定分の上界**であって、実測そのものではない。
/// 実測が定数を下回っているときに実測値を貼ると**余裕が消える**ので、
/// 貼る値は `max(実測, いまの定数)`。**実測そのものは別の行に出す**ので、
/// 判断の材料は失われない。
///
/// 引数の `one` / `three` は、件数だけが違う2点の実測（測り方は器具の doc）。
pub fn overhead_block(
    target: &ModelId,
    measured_on: &str,
    one: u32,
    three: u32,
    per_message: u32,
    constant: u32,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    if let Some(note) = measured_on_note(measured_on, target) {
        lines.push(note);
    }

    lines.push(format!(
        "メッセージ1件 = {one} トークン / 3件 = {three} トークン（実測: {target}）"
    ));
    lines.push(format!(
        "メッセージ1件あたりの増分 = {per_message} トークン / いまの MESSAGE_OVERHEAD_TOKENS = {constant}"
    ));
    if per_message <= constant {
        lines.push("いまの定数で足りている（引き下げないこと。上界の余裕が消える）".to_owned());
    }
    lines.push("--- ここから下を src/prompt/estimate.rs に貼る ---".to_owned());
    lines.push(format!(
        "pub const MESSAGE_OVERHEAD_TOKENS: u32 = {};",
        per_message.max(constant)
    ));
    lines.push("--- ここまで ---".to_owned());

    lines.join("\n")
}

/// **特殊トークンであるときのトークン数。**
///
/// 1 は測った値ではなく**定義**（特殊トークンは1つのトークンに落ちる）。
/// **どのモデルで測ったかを名乗らないのは、名乗るものが無いから**
/// ——実測由来の定数と混ぜないこと（ADR-0033）。
pub const SPECIAL_TOKEN_COUNT: u32 = 1;

/// **器具が「組み上がった行」の比較に使う分類**（括弧を含まない）。
///
/// 括弧は `Seg::Lit` 側から来るので、綴りは `[` ＋ これ ＋ `]` で組み上がる
/// （[`compared_shape_spelling`]）。
///
/// # ⚠ ここが「一覧に載っている綴り」であることに寄りかかっている
///
/// [`ShapeProbe`] は**素の綴りが特殊でない上流では何も判定しない**ので、
/// **「崩した綴りが 1 トークンでないこと」を当てる口がここには無い。**
/// **それは兄弟のループ**（`measured_control_tokens_are_still_special_after_neutralizing`）
/// **が `MEASURED_CONTROL_TOKEN_SPELLINGS` の全件で見ている**——
/// **この綴りがその一覧に載っている限り、同じ綴りが同じ上流で当たっている。**
///
/// **その前提そのものは `tests/calibration_target.rs` の
/// `the_compared_spelling_is_also_measured_by_the_sibling_loop` が
/// `#[ignore]` 無しで見張る**（器具の中に書くと、器具ごと走らない）。
pub const COMPARED_SHAPE_CATEGORY: &str = "INST";

/// [`COMPARED_SHAPE_CATEGORY`] から組み上がる綴り。
pub fn compared_shape_spelling() -> String {
    format!("[{COMPARED_SHAPE_CATEGORY}]")
}

/// **組み上がった行と、そこから作る「防御なし」の行**（作業単位 B-25）。
///
/// # なぜ器具の外にあるのか
///
/// ここに要る判断は**すべて `build_prompt` と `SafeText::sanitize` だけで決まり、
/// LM Studio を1回も呼ばない。** 器具の本体は `#[ignore]` で
/// **素の `cargo test` では1行も走らない**ので、そこに置くと**誰も見ていない主張**になる
/// （ロードマップの記録: この器具は `0004b` の日から数日、測定に到達すらしていなかった）。
#[derive(Debug)]
pub struct AssembledShape {
    spelling: String,
    neutralized: String,
    guarded_line: String,
    unguarded_line: String,
}

impl AssembledShape {
    /// 崩す前の綴り。
    pub fn spelling(&self) -> &str {
        &self.spelling
    }

    /// 崩した形（`SafeText::sanitize` が返したもの。**表に転記しない**）。
    pub fn neutralized(&self) -> &str {
        &self.neutralized
    }

    /// 組み上がった行（崩した形を含む）。
    pub fn guarded_line(&self) -> &str {
        &self.guarded_line
    }

    /// 空白を抜いて綴りへ戻した行（＝防御が無かった場合の形）。
    pub fn unguarded_line(&self) -> &str {
        &self.unguarded_line
    }

    /// **測る対象。** 器具はこれを回すだけで、
    /// **どれをどの欄へ入れるかは [`probe_shape`] が決める**（配線を器具に持たせない）。
    pub fn texts(&self) -> [&str; 3] {
        [self.spelling(), self.guarded_line(), self.unguarded_line()]
    }
}

/// 組み上がった行から、比較に要る文字列を組む。**純粋関数**（上流を呼ばない）。
///
/// # 3つの `Err` は、それぞれ別の壊れ方
///
/// | `Err` | 何が起きている |
/// |---|---|
/// | 規則が崩していない | その綴りは `control_token_shape_len` の外（比較の前提が無い） |
/// | 行に原形が載った | **無害化が素通りした**（この器具の存在理由そのもの） |
/// | 行に崩した形が無い | 無害化の形が変わった（`[INST ]` 以外になった） |
///
/// **「防御なしの形が作れている」を別の検査にしていないのは、到達できないから**
/// ——上の3つを抜けた時点で崩した形は綴りと必ず違い、置換は必ず行を変える。
/// **到達しない検査は、どんなテストでも赤にできない**（CLAUDE.md「死んだ検査」）。
pub fn assembled_shape(line: &str, spelling: &str) -> Result<AssembledShape, String> {
    let neutralized = SafeText::sanitize(spelling).to_string();

    if neutralized == spelling {
        return Err(format!(
            "規則がこの綴りを崩していない（比較の前提が無い）: {spelling:?}"
        ));
    }
    if line.contains(spelling) {
        return Err(format!("組み上がった行に綴りが原形で載った: {line:?}"));
    }
    if !line.contains(&neutralized) {
        return Err(format!(
            "組み上がった行に崩した形（{neutralized:?}）が無い（無害化の形が変わった可能性）: {line:?}"
        ));
    }

    let unguarded_line = line.replace(&neutralized, spelling);

    Ok(AssembledShape {
        spelling: spelling.to_owned(),
        neutralized,
        guarded_line: line.to_owned(),
        unguarded_line,
    })
}

/// **崩した形を「防御あり／防御なし」で読むときの材料**（作業単位 B-25）。
///
/// 器具は、制御トークンの綴りが**プロンプトに組み上がった行**で崩れていることを、
/// 2点の実測（崩した行と、空白を抜いて綴りへ戻した行）の差で確かめる。
///
/// **`raw` が要るのは、その差が読めるかどうかがモデルで変わるから**（[`ShapeProbe::verdict`]）。
///
/// **⚠ 自分で組み立てないこと。** [`probe_shape`] を通すこと——
/// **どの実測がどの欄へ入るかの配線は、器具に持たせると誰も見ていない**
/// （実測: 欄を2つ入れ替える変異が 1749 全緑を通り、**B-25 が消した偽警報がそのまま復活した**）。
pub struct ShapeProbe<'a> {
    /// 崩す前の綴り。
    pub spelling: &'a str,
    /// **素の綴りだけ**を測った値（ベースラインを引いた後）。
    pub raw: u32,
    /// **崩した形**を含む行の値。
    pub guarded: u32,
    /// **空白を抜いて綴りへ戻した行**の値。
    pub unguarded: u32,
}

/// **測った値を欄へ入れる配線。純粋関数**（測る手段は引数で受け取る）。
///
/// 器具は「[`AssembledShape::texts`] を回して測る」だけになり、
/// **どれがどの欄かを一度も書かない。**
/// 見張り番は `tests/calibration_target.rs` の
/// `the_wiring_cannot_swap_the_two_sides`——**入力ごとに違う値を返す偽の `measure`** を渡す。
///
/// **⚠ `15b` の「走査本体を引数でソースを受け取る形にする」と同じ形。**
pub fn probe_shape<'a>(shape: &'a AssembledShape, measure: impl Fn(&str) -> u32) -> ShapeProbe<'a> {
    ShapeProbe {
        spelling: shape.spelling(),
        raw: measure(shape.spelling()),
        guarded: measure(shape.guarded_line()),
        unguarded: measure(shape.unguarded_line()),
    }
}

/// [`ShapeProbe`] から言えること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeVerdict {
    /// 素の綴りがこの上流で特殊トークンで、**崩した側の方がトークン数が多い**
    /// ——特殊トークンが普通の文字列に戻っている。
    Broken,
    /// 素の綴りが特殊トークンなのに、**崩しても数が増えていない。**
    /// **本物の失敗**（無害化が空振りしている）。
    StillSpecial,
    /// **素の綴りが、この上流では特殊トークンではない。**
    /// **2点の差は原理的に何も言わない**ので判定しない（下の doc）。
    NotObservable,
}

impl ShapeProbe<'_> {
    /// **判定。純粋関数。**
    ///
    /// # なぜ「特殊でないなら判定しない」なのか（作業単位 B-25）
    ///
    /// 差が読めるのは、**素の綴りが特殊トークン1つに落ちる上流だけ。**
    /// そうでない上流では、崩す前も後もただの文字列なので**数はほとんど動かない。**
    /// **`qwen/qwen3.5-9b` で `[INST]` を測ると `8` 対 `8`**（C-14 / 2026-08-29）。
    ///
    /// **ここを「差が無ければ失敗」にすると、偽の警報になる** ——
    /// 規則（`control_token_shape_len`）は正しく崩し続けているので、
    /// **落ちた人は直す先の無い場所を探すことになる。**
    ///
    /// # ⚠ B-22 の形の「片方」である（**同じではない**）
    ///
    /// `MEASURED_MODEL_DEPENDENT_SPELLINGS` の器具は2つ当てている——
    /// **素の値は判定しない**（こちらが採った側）と、
    /// **無害化後が 1 トークンでない**（`broken < 2`。**こちらは持っていない**）。
    /// **後者をここに写さないのは、同じ綴り・同じ上流で兄弟のループが測っているから**
    /// （[`COMPARED_SHAPE_CATEGORY`] の doc に前提と見張り番がある）。
    pub fn verdict(&self) -> ShapeVerdict {
        if self.raw != SPECIAL_TOKEN_COUNT {
            return ShapeVerdict::NotObservable;
        }
        if self.guarded > self.unguarded {
            ShapeVerdict::Broken
        } else {
            ShapeVerdict::StillSpecial
        }
    }

    /// **人が読む1行。** 3つの判定それぞれで文面が違う
    /// （見張り番: `each_verdict_reads_differently`）。
    ///
    /// **相手を名乗る**（ADR-0033 §決定1）。
    /// **観測できないときは、直す先の無い指示を出さない**——それが B-25 の本題。
    pub fn report(&self, target: &ModelId) -> String {
        let spelling = self.spelling;
        let (raw, guarded, unguarded) = (self.raw, self.guarded, self.unguarded);

        match self.verdict() {
            ShapeVerdict::Broken => format!(
                "崩せている: 防御あり {guarded} > 防御なし {unguarded} トークン\
                 （素の {spelling:?} = {raw} トークン＝この上流では特殊 / 実測: {target}）"
            ),
            ShapeVerdict::StillSpecial => format!(
                "⚠ 崩せていない: 防御あり {guarded} / 防御なし {unguarded} トークン\
                 （素の {spelling:?} = {raw} トークン＝この上流では特殊 / 実測: {target}）"
            ),
            ShapeVerdict::NotObservable => format!(
                "⚠ 判定しない: この上流では素の {spelling:?} = {raw} トークンで、特殊トークンではない。\
                 防御あり {guarded} / 防御なし {unguarded} の差は、この上流では何も言わない\
                 （モデル差であって無害化の失敗ではない）。\
                 綴りの帰属は docs/04-environment.md の手順2 / 実測: {target}"
            ),
        }
    }

    /// **失敗の文面**（[`ShapeVerdict::StillSpecial`] のときだけ）。
    ///
    /// 直す先を名乗る。**観測できないときは `None`**——鳴らす相手が居ない。
    pub fn failure(&self) -> Option<String> {
        if self.verdict() != ShapeVerdict::StillSpecial {
            return None;
        }

        let spelling = self.spelling;
        let (guarded, unguarded) = (self.guarded, self.unguarded);
        Some(format!(
            "無害化しても特殊トークンのままに見える: {spelling:?} \
             防御あり {guarded} / 防御なし {unguarded} トークン。\
             src/prompt/safe.rs の規則（control_token_shape_len）を見直すこと"
        ))
    }
}

/// **器具が標準出力へ出す塊。** 実測2行と判定1行を**1回の `println!` で**出す
/// （`golden_block` と同じ理由——4本は並行に走るので、行ごとに書くと出力が混ざる）。
///
/// # ⚠ 判定を出すこと自体が、この関数の主張である
///
/// **「観測できないときは判定しない」を選べる根拠は「報告はする」の一点**
/// （それが無いと、Qwen で回した人は**なぜ何も起きないのかを知る手段を失う**）。
/// **報告を落とす変異は、器具の中に置くと誰も見ていない**（実測: 1749 全緑）ので、
/// **塊の組み立てをここへ出して `the_block_shows_both_lines_and_the_verdict` が見る。**
pub fn shape_block(target: &ModelId, shape: &AssembledShape, probe: &ShapeProbe<'_>) -> String {
    [
        format!(
            "防御あり {:?} = {} トークン（実測: {target}）",
            shape.guarded_line(),
            probe.guarded
        ),
        format!(
            "防御なし {:?} = {} トークン",
            shape.unguarded_line(),
            probe.unguarded
        ),
        probe.report(target),
    ]
    .join("\n")
}

/// **対照の綴りが、規則の内側か外側か**（作業単位 B-27）。
///
/// `MEASURED_ORDINARY_LOOKALIKES` は**2種類の探り針**でできている
/// （`src/prompt/safe.rs` のその定数の doc。**ここに写しを作らない**）。
/// **対照が特殊トークンになったとき、直す先は種類ごとに違う。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookalikeSide {
    /// **規則の内側**（`[control_8]` 型）。規則はこの綴りを**すでに崩している。**
    Inside,
    /// **規則の外側**（`>>ABSTRACT<<` 型）。規則はこの綴りを**触らない。**
    Outside,
}

/// 綴りがどちら側かを、**無害化そのものに訊く**（作業単位 B-27）。
///
/// **公開 API だけで決まる** ——`SafeText::sanitize` が値を変えたなら内側。
/// **判定を写さない**ので、**規則を広げて針が内側へ落ちた日には、ここも一緒に動く。**
///
/// # ⚠ 前提: 綴りの前後に空白が無いこと
///
/// `SafeText::sanitize` は行の前後の空白も落とすので、
/// **前後に空白のある綴りを渡すと、崩されていなくても内側に見える。**
/// 対照の一覧がその前提を満たすことは、`src/prompt/safe.rs` の
/// `every_lookalike_is_blank_free` が見ている。
pub fn lookalike_side(spelling: &str) -> LookalikeSide {
    if SafeText::sanitize(spelling).as_str() == spelling {
        LookalikeSide::Outside
    } else {
        LookalikeSide::Inside
    }
}

/// **対照を1件測った結果**（作業単位 B-27）。
///
/// # なぜ器具の外にあるのか
///
/// 器具の本体（`measured_control_tokens_are_still_special_after_neutralizing`）は
/// **`#[ignore]`** で、**素の `cargo test` では1行も走らない。**
/// **文面の中身は、ここに置いて `tests/calibration_target.rs` が見張る**
/// （[`ShapeProbe`] と同じ理由）。
pub struct LookalikeProbe<'a> {
    /// 対照の綴り。
    pub spelling: &'a str,
    /// **素の綴りだけ**を測った値（ベースラインを引いた後）。
    pub raw: u32,
}

impl LookalikeProbe<'_> {
    /// 規則の内側か外側か。
    pub fn side(&self) -> LookalikeSide {
        lookalike_side(self.spelling)
    }

    /// **この上流で特殊トークンになったか**（＝対照でなくなったか）。
    pub fn became_special(&self) -> bool {
        self.raw == SPECIAL_TOKEN_COUNT
    }

    /// 側を名乗る短い語。
    fn side_note(&self) -> &'static str {
        match self.side() {
            LookalikeSide::Inside => "規則の内側の針",
            LookalikeSide::Outside => "規則の外側の針",
        }
    }

    /// **直す先。側で分かれる**（作業単位 B-27）。
    ///
    /// # なぜ1つの文面ではいけないのか
    ///
    /// 以前は側に関わらず「**src/prompt/safe.rs の規則と一覧を見直すこと**」だった。
    /// **半分は直す先を持たない。**
    ///
    /// | 側 | 何が起きているか | 直す先 |
    /// |---|---|---|
    /// | [`LookalikeSide::Outside`] | **規則がこの形を触っていない**（`>>ABSTRACT<<` 型） | **規則**（B-22 の前例） |
    /// | [`LookalikeSide::Inside`] | **規則はすでに崩している**（`[control_8]` 型） | **一覧の帰属**（規則には無い） |
    ///
    /// **⚠ 判定は止めない。** 対照が特殊になったのは**どちらの側でも本当の事実**で、
    /// **その一覧の帰属が崩れた**ことを意味する。
    /// **B-25（比較が原理的に無意味になる ＝ 判定を止める）とは形が違う。**
    fn where_to_fix(&self) -> &'static str {
        match self.side() {
            LookalikeSide::Outside => {
                "規則がこの形を触っていないので、防御は素通りする。\
                 src/prompt/safe.rs の is_shape_body / control_token_shape_len を広げること\
                 （B-22 の前例。一覧を増やすのではなく規則を広げる）"
            }
            LookalikeSide::Inside => {
                "規則はこの綴りをすでに崩しているので、規則の側に直す先は無い。\
                 崩れたのは一覧の帰属のほうで、直すのは src/prompt/safe.rs の\
                 MEASURED_ORDINARY_LOOKALIKES から外し、\
                 この上流でだけ特殊なら MEASURED_MODEL_DEPENDENT_SPELLINGS へ、\
                 実測の基準にする上流でも特殊なら MEASURED_CONTROL_TOKEN_SPELLINGS へ\
                 移すこと（B-22 が `<|im_start|>` でやった形）。\
                 内側の針が0本にならないよう、代わりの針を1本足すこと\
                 （the_lookalikes_probe_both_sides_of_the_rule）"
            }
        }
    }

    /// **人が読む1行。** 相手を名乗る（ADR-0033 §決定1）。
    pub fn report(&self, target: &ModelId) -> String {
        let spelling = self.spelling;
        let raw = self.raw;
        let side = self.side_note();

        if self.became_special() {
            format!(
                "⚠ 対照が特殊トークンになった: {spelling:?} = {raw} トークン\
                 （{side} / 実測: {target}）。{}",
                self.where_to_fix()
            )
        } else {
            format!(
                "対照 {spelling:?} = {raw} トークン\
                 （{side} / この上流では特殊ではない / 実測: {target}）"
            )
        }
    }

    /// **失敗の文面**（特殊トークンになったときだけ）。
    ///
    /// **判定は止めない。** 対照が特殊になったのは**どちらの側でも本当の事実**で、
    /// その一覧の帰属が崩れたことを意味する（B-25 の「観測できない」とは形が違う）。
    pub fn failure(&self) -> Option<String> {
        if !self.became_special() {
            return None;
        }

        let spelling = self.spelling;
        let raw = self.raw;
        let side = self.side_note();
        Some(format!(
            "対照が特殊トークンになった: {spelling:?} が {raw} トークン（{side}）。{}",
            self.where_to_fix()
        ))
    }
}

/// **制御トークンの一覧の種類**（作業単位 B-35）。
///
/// **当てるものが一覧ごとに違う。** 器具の中に `if` を書くと、
/// **どちらの一覧にどちらの判定が付くかを誰も見ていない**（`#[ignore]` なので走らない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlListKind {
    /// `MEASURED_CONTROL_TOKEN_SPELLINGS` ——
    /// **素の綴りが「いまも特殊」であることまで当てる。**
    /// 外れたらモデル側が変わった合図で、**直す先はその一覧**（測り直す）。
    Measured,
    /// `MEASURED_MODEL_DEPENDENT_SPELLINGS` ——
    /// **素の値は報告するだけで判定しない**（`ministral` で 6・`qwen` で 1。どちらも正しい）。
    /// 理由は `src/prompt/safe.rs` のその定数の doc（**ここに写しを作らない**）。
    ModelDependent,
}

/// **どの一覧を、どちらの判定で回すか**（作業単位 B-35 のレビューが空いていると指摘した場所）。
///
/// # なぜ表が器具の外にあるのか
///
/// B-35 で**判定**は器具の外へ出たが、**「どちらの `kind` を渡すか」という選択は器具の中に残った。**
/// **`ControlListKind::Measured` と `::ModelDependent` の2行を入れ替えても 1867 全緑**だった
/// （レビュアーが実測 / 2026-09-02）——
/// **「配線だけの場所」は、層に関係なく観測対象から落ちる**（CLAUDE.md）。
///
/// # ⚠ 入れ替わったときの実害は、片側だけ重い
///
/// **`Measured` が `ModelDependent` へ化けると、`MEASURED_CONTROL_TOKEN_SPELLINGS` の全件について
/// 「素の綴りがまだ 1 トークンか」を一切見なくなる。**
/// それは docs/04-environment.md が「この器具がやること」として名指ししている検査で、
/// **B-22 が Qwen で「一覧は丸ごと測り直しになる」と気づいた唯一の経路**である。
/// **器具は緑のまま、一覧だけが黙って検証されなくなる。**
///
/// **逆向き**（`ModelDependent` が `Measured` へ化ける）は `ministral` で必ず落ちるが、
/// **直す先が無い**（B-25 の「直す先の無い警報」）。
///
/// 見張り番は `tests/calibration_target.rs` の
/// `each_control_list_is_paired_with_its_own_judgement` と
/// `the_instrument_reads_the_pairing_from_the_shared_table`。
pub const CONTROL_LISTS: [(ControlListKind, &[&str]); 2] = [
    (ControlListKind::Measured, &MEASURED_CONTROL_TOKEN_SPELLINGS),
    (
        ControlListKind::ModelDependent,
        &MEASURED_MODEL_DEPENDENT_SPELLINGS,
    ),
];

/// **制御トークンの綴りを1件測った結果**（作業単位 B-35）。
///
/// # なぜ器具の外にあるのか
///
/// 器具の本体（`measured_control_tokens_are_still_special_after_neutralizing`）は
/// **`#[ignore]`** で、**素の `cargo test` では1行も走らない。**
/// **判定も文面もここに置いて `tests/calibration_target.rs` が見張る**
/// （[`ShapeProbe`] / [`LookalikeProbe`] と同じ作法）。
///
/// **⚠ 自分で組み立てないこと。** [`probe_control`] を通すこと——
/// **どの実測がどの欄へ入るかの配線を器具に持たせると、欄を入れ替える変異が緑を通る**
/// （B-25 のレビューが `ShapeProbe` で実測: 1749 全緑）。
pub struct ControlProbe<'a> {
    /// どちらの一覧の綴りか。
    pub kind: ControlListKind,
    /// 崩す前の綴り。
    pub spelling: &'a str,
    /// **素の綴り**を測った値（ベースラインを引いた後）。
    pub raw: u32,
    /// 崩した形（`SafeText::sanitize` が返したもの。**表に転記しない**）。
    pub neutralized: String,
    /// **崩した形**を測った値（ベースラインを引いた後）。
    pub broken: u32,
}

/// **測った値を欄へ入れる配線。純粋関数**（測る手段は引数で受け取る）。
///
/// 崩した形は**ここで `SafeText::sanitize` から求める**——器具に転記させると、
/// 無害化の実装を変えたときに器具だけ古いまま緑になる。
///
/// 見張り番は `tests/calibration_target.rs` の
/// `the_control_probe_wiring_cannot_swap_the_two_sides`。
pub fn probe_control<'a>(
    kind: ControlListKind,
    spelling: &'a str,
    measure: impl Fn(&str) -> u32,
) -> ControlProbe<'a> {
    let neutralized = SafeText::sanitize(spelling).to_string();
    ControlProbe {
        kind,
        spelling,
        raw: measure(spelling),
        broken: measure(&neutralized),
        neutralized,
    }
}

impl ControlProbe<'_> {
    /// **人が読む1行。** 一覧ごとに文面が違う
    /// （見張り番: `each_control_list_kind_reads_differently`）。
    ///
    /// **相手を名乗らないのは、器具が塊の頭で1度だけ名乗るから**
    /// （`制御トークンの当て直し（実測: …）`）。**同じ行に2度書かない。**
    pub fn report(&self) -> String {
        let (spelling, raw, broken) = (self.spelling, self.raw, self.broken);
        let neutralized = self.neutralized.as_str();

        match self.kind {
            ControlListKind::Measured => format!(
                "{spelling:?} = {raw} トークン / 無害化後 {neutralized:?} = {broken} トークン"
            ),
            ControlListKind::ModelDependent => format!(
                "モデル依存 {spelling:?} = {raw} トークン / 無害化後 {neutralized:?} = {broken} トークン\
                 （素の値は判定しない）"
            ),
        }
    }

    /// **失敗の文面**（無ければ空）。
    ///
    /// | 見るもの | どちらの一覧で | 外れたときの意味 |
    /// |---|---|---|
    /// | 素の綴りがまだ特殊 | [`ControlListKind::Measured`] だけ | モデル側が変わった（一覧を測り直す） |
    /// | 無害化後が 2 トークン以上 | 両方 | **無害化が空振りしている**（規則が覆えていない） |
    /// | 空白を除けば元の綴りに戻る | 両方 | 無害化で文字が落ちた |
    pub fn failures(&self) -> Vec<String> {
        let (spelling, raw, broken) = (self.spelling, self.raw, self.broken);
        let neutralized = self.neutralized.as_str();
        let mut failures: Vec<String> = Vec::new();

        if self.kind == ControlListKind::Measured && raw != SPECIAL_TOKEN_COUNT {
            failures.push(format!(
                "綴りが特殊トークンでなくなった（またはモデルが変わった）: \
                 {spelling:?} 実測 {raw} != {SPECIAL_TOKEN_COUNT}。\
                 src/prompt/safe.rs の MEASURED_CONTROL_TOKEN_SPELLINGS を測り直すこと"
            ));
        }

        if broken < 2 {
            failures.push(format!(
                "無害化が効いていない: {spelling:?} → {neutralized:?} が {broken} トークン。\
                 規則（control_token_shape_len）が覆えていない"
            ));
        }

        if neutralized.replace(' ', "") != spelling {
            failures.push(format!(
                "無害化で文字が失われた: {spelling:?} → {neutralized:?}"
            ));
        }

        failures
    }
}
