//! **実測由来の定数が「何で測ったか」を機械可読で持ち、上流と突き合わせる**
//! （[ADR-0033](../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md) §決定1 /
//! 作業単位 E-2）。
//!
//! # 何をするモジュールか
//!
//! **定数が名乗っている「測ったモデル」と、いま上流が応答するモデルがずれていたら、それを言う。**
//! **言うだけ**である——[ADR-0035](../docs/adr/0035-the-watchdog-reports-the-human-decides.md)
//! と同じ形（**見張りは報告する。決めるのは人**）。
//!
//! * **起動を止めない。** ずれは「数が狂う」側であって「壊れる」側ではない
//!   （[ADR-0036](../docs/adr/0036-the-range-of-machines-is-a-constraint.md) §決定1 の判定の問い）。
//!   止める形にすると、**別のモデルを載せた瞬間に製品が起動しなくなる。**
//! * **つまみを足さない。** 突き合わせに設定は要らない
//!   （ADR-0036 §決定3 —— 根拠があり、外しても壊れず、観測できる値はつまみにしない）。
//!
//! # ⚠ ここで言えること・言えないこと（**先に書いておく**）
//!
//! ADR-0033 §決定3 が器具の限界を先に書いたのと同じ作法で、この機構の限界も先に置く。
//!
//! **言えるのは「ずれていたら言う」まで。**
//! **「定数が正しい」でも「実測由来の値が守られている」でもない。**
//!
//! | 主張 | 実際 |
//! |---|---|
//! | 定数が正しい | **言えない。** 突き合わせているのは**綴り**だけで、値は一切見ていない |
//! | 測り直しを強制する | **しない。** 報告するだけで、起動も本番経路も止まらない |
//! | 上流が別実装でも働く | **働かない。** [`crate::llm::LmStudioClient::models`] は LM Studio 独自の
//!   `/api/v0/models` を読む。他の上流では**必ず「聞けなかった」に落ちる**（それでよい） |
//! | 一覧が最新である | **保証しない。** この一覧は ADR-0033 §背景の表の写しで、
//!   **実測由来の定数を増やしてここへ足し忘れれば黙って古びる**（ADR-0033 §結果が書いている穴）。
//!   **落ちるのは綴りが変わったときだけ**（[`missing_mentions`] の走査） |
//!
//! # 3値で扱う（**「聞けなかった」を「ずれ」と言わない**）
//!
//! [`ModelCheck`] は **一致 / ずれ / 聞けなかった** の3値。
//! **上流が答えないのは異常ではない**（LM Studio は常時稼働ではない / `~/.claude/CLAUDE.md`
//! 「落ちている状態を『壊れている』と誤診しない」）。
//! **冷えた起動では現に照会が失敗する**（実測 / 2026-08-28）ので、
//! ここを「ずれ」に潰すと**毎回鳴る警告**になり、読まれなくなる。
//!
//! # 依頼した綴りを「確認」に使わない
//!
//! **LM Studio は存在しないモデル名でもエラーにせず、載っているモデルが黙って答える**
//! （司令塔が実測 / 2026-08-28）。だから
//! **「こちらが `model` に何を書いたか」は、何の確認にもならない。**
//! 判定は**上流が返したモデル一覧に、その綴りが在るか**から始める（[`check`]）。

/// 実測由来の定数が、**どちらのモデルの下で意味を持つか。**
///
/// メインは本文生成、サブは抽出・変容（`PERSONACORE_SUB_MODEL`）。
/// **`Both` は「両方の依頼を組み立てるときに効く」もの**
/// （無害化と見積もりは、プロンプトを組み立てるすべての経路に効く /
/// [docs/04-environment.md](../docs/04-environment.md) §0 の表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasuredRole {
    /// 本文生成のモデルの下でだけ意味を持つ。
    Main,
    /// 裏方（抽出・変容）のモデルの下でだけ意味を持つ。
    Sub,
    /// 両方。**突き合わせは2面ぶん行う。**
    Both,
}

impl MeasuredRole {
    /// ログに載せる綴り。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Sub => "sub",
            Self::Both => "both",
        }
    }

    /// **突き合わせる面。** `Both` はここで2つに割れる。
    ///
    /// 面ごとに突き合わせる理由は**帰属**——
    /// 「載っているか」ではなく「**どちらのモデルの話か**」を出力に残すため。
    pub const fn faces(self) -> &'static [ModelFace] {
        match self {
            Self::Main => &[ModelFace::Main],
            Self::Sub => &[ModelFace::Sub],
            Self::Both => &[ModelFace::Main, ModelFace::Sub],
        }
    }
}

/// 突き合わせる面。**`MeasuredRole::Both` を割った後の単位。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFace {
    /// 本文生成の上流。
    Main,
    /// 裏方（抽出・変容）の上流。
    Sub,
}

impl ModelFace {
    /// **全部の値**（作業単位 P-2a —— 仕様書が `UpstreamView::face` の `enum` をここから作る）。
    /// 数え漏れは `every_model_face_is_listed_in_all` が見る。
    pub const ALL: [Self; 2] = [Self::Main, Self::Sub];

    /// ログに載せる綴り。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Sub => "sub",
        }
    }
}

/// **その面で上流へ依頼する綴りを選ぶ。** 帰属を決めるのはこの1箇所。
///
/// # ⚠ 既定どうしが同じ綴りだと、取り違えは観測できない
///
/// `DEFAULT_MAIN_MODEL` と `DEFAULT_SUB_MODEL` は今日は同じ綴りなので、
/// **既定のまま起動した環境では、ここを取り違えても出力が1文字も変わらない**
/// （CLAUDE.md の最頻の穴 / ADR-0033 §決定4 の追記が書いている限界と同じ形）。
/// **観測できるのは2つの綴りが分かれている環境だけ**で、
/// 見張り番はそちらの側にしか置けない
/// （`tests/server_measured_model.rs` は**メインとサブに別の綴り**を与えて回す）。
pub const fn requested_for<'a>(face: ModelFace, main: &'a str, sub: &'a str) -> &'a str {
    match face {
        ModelFace::Main => main,
        ModelFace::Sub => sub,
    }
}

/// **その面が突き合わせる相手の一覧を選ぶ。**
/// [`requested_for`] と**対**（同じ面で、綴りと一覧が同じ上流から来ること）。
///
/// # なぜ一覧を2つ受けるのか（作業単位 G-3a / ADR-0039 §決定1）
///
/// **上流は2つありうる**（`PERSONACORE_SUB_LLM_ENDPOINT`）。
/// **一覧はモデルに依らない**——これは**上流が1つのときだけ**正しい前提だった。
/// 2つになると、**本文の綴りを抽出側の上流の一覧の中から探す**ことになり、
/// **本文の上流には在るものが [`Drift::Absent`] として鳴る**（＝**嘘の警告**）。
///
/// **実測（2026-09-07 / 本番の入口）: 直す前は
/// `requested_model=<本文のモデル>` の `Absent` が実際に出ていた**
/// （`measured_drifted=9` / `measured_unanswered=0`）。
///
/// **ADR-0039 §決定3 は「`measured` の警告は本当のことを言っているから残す」と決めている。**
/// **嘘を鳴らし始めたら、その決定の前提が壊れる。**
///
/// # 聞いていない面は `None`（＝「聞けなかった」）
///
/// **`None` を渡すのは事実の申告**であって、黙らせる仕掛けではない
/// （[`check`] は `None` を [`ModelCheck::Unanswered`] に倒す）。
///
/// ## ⚠ 「上流が2つなら本文には一度も聞いていない」は**偽だった**
///
/// **G-3a の初版はここにそう書いていた。レビューが実測して否定した**（2026-09-07）——
/// **`server::context::resolve_budget` は `loaded_context_length()` → `models()` を通る**ので、
/// **本文の上流にも現に照会していた**（カウンタを植えて `main /api/v0/models hits = 1`）。
/// **`crate::llm::ModelsReport` は一覧も持っていて、その層が捨てていただけである。**
///
/// **いまは捨てない。** `resolve_budget` も一覧を返し、**本文の面は本文の上流の一覧で判定する。**
/// **往復は1本も増えていない**（同じ照会の本文から2つ読む）。
///
/// ## いま `None` になるのは2つの場合だけ
///
/// | 場合 | 意味 |
/// |---|---|
/// | **往復そのものが失敗した** | 上流の停止・別実装（`/api/v0/models` が無い） |
/// | **`PERSONACORE_CONTEXT_TOKENS` を明示した**（本文の面のみ） | **本当に一度も聞いていない**（`resolve_budget` が早期 return する） |
///
/// **⚠ 2つ目は「明示設定があるときに限る」。** **既定（未設定）では本文の面も一覧を持つ。**
///
/// **⚠ そして流用の条件は「明示設定」ではない**（G-3a の是正2周目 / 2026-09-07。
/// **ここは1周目、明示設定の場合だけを条件として書いていた。コードと違っていた**）。
/// **`server::App::bind` が見ているのは「本文の面が一覧を持たないこと」だけ**で、
/// **上の2つのどちらでも真になる。** そのとき**2つの宛先が同じ綴りなら
/// 抽出側の答えを流用する**（判定は `server::App::bind` の `one_upstream`）。
/// **本文の照会が落ちて抽出の照会が通った回に流用するのは妥当**——
/// **同じ上流が並べた、同じ一覧**だからである。
///
/// **見張り番は `tests/server_measured_model.rs` の2本**
/// ——`two_upstreams_are_each_judged_against_their_own_catalogue`（既定の経路）と
/// `one_upstream_named_twice_still_judges_the_body_face`（明示設定つきの2腕）。
/// **どちらも本番の入口で上流を立てて、両方の面を同じ行で見る。**
const fn catalogue_for<'a>(
    face: ModelFace,
    main: Option<&'a [String]>,
    sub: Option<&'a [String]>,
) -> Option<&'a [String]> {
    match face {
        ModelFace::Main => main,
        ModelFace::Sub => sub,
    }
}

/// **実測由来の定数1つ分の来歴。**
///
/// 値そのものは持たない。**持つのは「どこにあるか」と「何で測ったか」だけ。**
/// 値まで写すと、片方を直したときにもう片方が黙って古びる（CLAUDE.md）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasuredConstant {
    /// 定数の綴り。**[`missing_mentions`] が、この綴りが `location` に実在することを見る。**
    pub name: &'static str,
    /// 定数のあるファイル。**リポジトリ根からの相対パス。**
    pub location: &'static str,
    /// **測ったときの相手。** 「既定のモデル」ではなく「実測した事実」。
    pub measured_on: &'static str,
    /// 何に依存しているか（トークナイザ / チャットテンプレート / 饒舌さ）。
    /// **測り直す手順を選ぶための手掛かり**（[docs/04-environment.md](../docs/04-environment.md)）。
    pub depends_on: &'static str,
    /// どちらのモデルの下で意味を持つか。
    pub role: MeasuredRole,
}

/// 測ったときの相手。**いまのところ全部これ1つ**（ADR-0033 §背景）。
const MINISTRAL_14B: &str = "ministral-3-14b-instruct-2512";

/// **実測由来の定数の一覧**（ADR-0033 §背景の表の、機械可読な側）。
///
/// # 何を入れて、何を入れなかったか
///
/// 入れたのは**「モデルを差し替えると意味が変わる」もの**だけ。
///
/// | 入れなかったもの | 理由 |
/// |---|---|
/// | `QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS` | **機の速さ由来。モデルではない**（04-environment §3） |
/// | `DEFAULT_IDLE_TIMEOUT` / `DEFAULT_CONNECT_TIMEOUT` | **時間の値**。根拠となる計測が無く、
///   外れても壊れない場所へ移してある（ADR-0035 / ADR-0036 §決定2 の第2階層） |
/// | `FALLBACK_EXTRACTION_PROMPT_TOKENS` | **上流に引くようになった**（B-7）。
///   実測ではなく**落とし所**（ADR-0036 §決定2 の第1階層） |
/// | λ / `K` / `DriftForm` | **モデル ＋ 軸の組**で決まる。
///   **軸の組は環境ごとに違う**ので、モデルの綴りだけでは「ずれた」と言えない
///   （当て直しの手順は ADR-0030 が持つ） |
/// | ADR-0021 §15 / §16 の結論 | **定数ではない。** 突き合わせる綴りが無い |
/// | `MEASURED_ORDINARY_LOOKALIKES` | **同じ場所・同じ来歴の `MEASURED_CONTROL_TOKEN_SPELLINGS` と
///   1行にまとめてある**（`src/prompt/safe.rs` / トークナイザ / 両面）。**2行に割っても、
///   走査（[`missing_mentions`]）は同じファイルを2度読むだけで、言えることが増えない。**
///   **⚠ ただし外側の3件だけは 2026-08-28 の実測**（B-22 / 内側の3件は 2026-08-14） |
/// | `MEASURED_MODEL_DEPENDENT_SPELLINGS` | **`measured_on` が1つに決まらない。**
///   `ministral` で 6 トークン・`qwen` で 1 トークンという綴りを持つ一覧で、
///   **どちらの綴りを書いても嘘になる**（理由は定数側の doc / B-22） |
/// | `RequestPolicy::PERSONACORE` のサンプラー4つ<br>（`top_k` / `top_p` / `min_p` / `repeat_penalty`） |
///   **鍵が違う。** ここが突き合わせるのは**モデルの綴り**だが、
///   **あの4つが名乗っている相手はモデルではなく上流**（LM Studio のプロキシが本文へ入れていた既定 /
///   ADR-0038 §決定2 / C-13）。**登録すると、モデルを替えただけで鳴り、
///   上流を替えても黙る**——**鳴ってほしい向きと逆**である。
///   **そして [`check`] は値を見ない**（このモジュールの ⚠ 節）ので、
///   **登録しても「その値がそのモデルに合っているか」は最初から言えない。**
///   **⚠ 足し忘れではなく、判断である**（作業単位 A-7c）。
///   **同じ `PERSONACORE` の `presence_penalty` は実測由来ではない**（利用者の決定 / A-14）
///   **ので、この表の候補ではない** |
/// | `SUB_LLM_TEMPERATURE`<br>（`src/llm/request.rs`） |
///   **上の4欄とまったく同じ理由**——**名乗っている相手はモデルではなく上流**
///   （C-13 が `/slots` で読んだ、この機の上流の実効の既定 / ADR-0038 §決定2）。
///   **⚠ 足し忘れではなく、判断である**（作業単位 A-7d）。
///   **⚠ そして、この行が無くても見張り番は1本も赤くならない**——
///   [`missing_mentions`] は**「登録済み → ソース」の一方向**しか見ないので、
///   **入れなかったものは原理的に走査の外**である（A-7d で実測。
///   **この行を落とす変異は 1748 全緑**）。**だから、ここは人の目のためだけに在る。** |
/// | `RUNAWAY_MIN_COPIES` / `RUNAWAY_MIN_CHARS` / `RUNAWAY_MAX_PERIOD`<br>（`src/domain/runaway.rs`） |
///   **ADR-0062 が求めているのは定数の doc に来歴を書くこと**で、ここへの登録ではない（来歴と境目の下の3本は定数の doc）。
///   判定の規則は文字の並びだけで決まり、トークナイザにもテンプレートにもよらないが、
///   **境目の妥当性はモデルの出力（の分布）に依存する** —— `MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS`
///   （「モデルの饒舌さ」）と同じ種類である（値が外れたときの壊れ方は ADR-0062 §理由）。
///   **それでも登録しないのは、測った相手が、起動の突き合わせが一覧を読める上流ではないから**
///   （その推論サーバの返答。測定の置き場所に残っていたものと、利用者の DB にあったもの）。
///   `measured_on` に書く綴りが [`check`] の相手にならず、突き合わせは「聞けなかった」に落ちる
///   （このモジュールの冒頭の doc の3値）。
///   **⚠ 足し忘れではなく、判断である**（作業単位 A-21a）。 |
///
/// **⚠ この一覧そのものが写しである**（ADR-0033 §結果）。
/// **増やしたときにここへ足し忘れれば、黙って古びる。**
/// **⚠ 上の「入れなかったもの」も同じ**——B-22 のレビューで**2件の抜け**が見つかった
/// （`MEASURED_ORDINARY_LOOKALIKES` は従来から、
/// `MEASURED_MODEL_DEPENDENT_SPELLINGS` は B-22 で足した側）。
/// **表を読む人には「足し忘れ」と「理由があって入れていない」の区別が付かない。**
pub static MEASURED_CONSTANTS: [MeasuredConstant; 5] = [
    MeasuredConstant {
        name: "MEASURED_CONTROL_TOKEN_SPELLINGS",
        location: "src/prompt/safe.rs",
        measured_on: MINISTRAL_14B,
        depends_on: "トークナイザ",
        // **無害化はプロンプトを組み立てるすべての経路に効く**（04-environment §0）。
        role: MeasuredRole::Both,
    },
    MeasuredConstant {
        name: "MESSAGE_OVERHEAD_TOKENS",
        location: "src/prompt/estimate.rs",
        measured_on: MINISTRAL_14B,
        depends_on: "チャットテンプレート",
        // `estimate_tokens` は抽出（`prompt::extraction`）と変容（`prompt::drift`）でも使う。
        //
        // **バイト数の計画では今も上界を担う**（ADR-0052 §見張り番の扱い の `MEASURED_CONSTANTS` の行。
        // だからこの行は残す）。
        // **上流が数えた計画（`prompt::CountedParts` を埋めた表 / B-60c・`server::counted_plan`（B-60d-1））では
        // 安全を担わない** —— 外れても送る前の封（送る本文そのものを上流が数えた実数）が組み直させるだけで、
        // 安全は封が持つ。
        role: MeasuredRole::Both,
    },
    MeasuredConstant {
        name: "GOLDEN",
        location: "tests/prompt_estimate_calibration.rs",
        measured_on: MINISTRAL_14B,
        depends_on: "トークナイザ",
        role: MeasuredRole::Both,
    },
    MeasuredConstant {
        name: "GOLDEN_BASELINE",
        location: "tests/prompt_estimate_calibration.rs",
        measured_on: MINISTRAL_14B,
        depends_on: "チャットテンプレート",
        role: MeasuredRole::Both,
    },
    MeasuredConstant {
        name: "MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS",
        location: "src/server/extract.rs",
        measured_on: MINISTRAL_14B,
        depends_on: "モデルの饒舌さ",
        // 抽出の応答の長さなので、効くのはサブモデルの下だけ。
        role: MeasuredRole::Sub,
    },
];

/// **突き合わせの結果。3値**（一致 / ずれ / 聞けなかった）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCheck {
    /// **一致。** 上流が応答する綴りが、測ったときの相手と同じ。
    Match,
    /// **ずれ。** 内訳は [`Drift`]。
    Mismatch(Drift),
    /// **聞けなかった。** 上流のモデル一覧が取れていない。
    ///
    /// **これを「ずれ」と言わないこと。** 上流の停止・別実装（`/api/v0/models` が無い）・
    /// 冷えた起動は、どれも異常ではない。
    Unanswered,
}

/// ずれの内訳。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// 上流が応答するのは `upstream` で、測ったときの相手とは別。
    Other {
        /// **上流が応答する綴り。**
        upstream: String,
    },
    /// **依頼する綴りが上流の一覧に無い。**
    ///
    /// LM Studio は**存在しないモデル名でもエラーにせず、載っているモデルが黙って答える**ので、
    /// この形は「誰が答えるか分からない」状態そのもの。
    Absent {
        /// **こちらが依頼する綴り。**
        requested: String,
    },
}

/// **測ったときの相手と、上流が応答する相手を突き合わせる。純粋関数。**
///
/// * `catalogue` が `None`（照会できなかった）か**空**なら **聞けなかった。**
///   **空を「ずれ」に倒さない**——上流が何も言っていないのに「別のモデルだ」とは言えない。
/// * 依頼する綴りが一覧に無ければ **ずれ**（[`Drift::Absent`]）。
/// * 一覧に在れば、その綴りが応答する。測ったときの相手と比べて **一致 / ずれ。**
///
/// # ⚠ `state` を見ていない（**未ロードを「聞けなかった」にしない**）
///
/// 一覧には未ロードのモデルも並ぶ（`state` が `not-loaded`）。**それでも判定は変えない。**
/// LM Studio は JIT ロードなので、**一覧に在る綴りを依頼すれば、その綴りが載って答える**
/// （docs/archive/04-environment-lm-studio.md）。**未ロードは「これから載る」であって「別のモデルが答える」ではない。**
///
/// **見ていたら、冷えた起動のたびに「ずれ」か「聞けなかった」を鳴らすことになる。**
/// 見張り番は `an_unloaded_model_still_counts_as_the_one_that_answers`。
pub fn check(measured_on: &str, requested: &str, catalogue: Option<&[String]>) -> ModelCheck {
    let Some(models) = catalogue.filter(|models| !models.is_empty()) else {
        return ModelCheck::Unanswered;
    };

    if !models.iter().any(|id| id == requested) {
        return ModelCheck::Mismatch(Drift::Absent {
            requested: requested.to_owned(),
        });
    }

    if requested == measured_on {
        ModelCheck::Match
    } else {
        ModelCheck::Mismatch(Drift::Other {
            upstream: requested.to_owned(),
        })
    }
}

/// 突き合わせ1行分。**定数 × 面**（`Both` は2行になる）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// どの定数の話か。
    pub constant: &'static MeasuredConstant,
    /// どちらのモデルの話か。**帰属。**
    pub face: ModelFace,
    /// 突き合わせの結果。
    pub check: ModelCheck,
}

/// **突き合わせ1行を作る。判定（[`check`]）を呼ぶのはここだけ。**
///
/// [`check_all`]（起動時の両面）と [`check_face`]（引き直しの片面）が
/// **同じ答えを返すこと**を、写しではなく構造で保つ
/// （見張り番は `judging_one_face_alone_agrees_with_judging_both`）。
fn row_for(
    constant: &'static MeasuredConstant,
    face: ModelFace,
    requested: &str,
    catalogue: Option<&[String]>,
) -> Row {
    Row {
        constant,
        face,
        check: check(constant.measured_on, requested, catalogue),
    }
}

/// **1つの面だけを突き合わせる。純粋関数**（作業単位 G-3b）。
///
/// 引き直し（`server::context`）は**その面の上流にしか聞かない**ので、
/// 両面ぶんの綴りも一覧も持っていない。**それでも判定は [`check_all`] と同じ**
/// （どちらも [`row_for`] を通る）。
pub fn check_face(face: ModelFace, requested: &str, catalogue: Option<&[String]>) -> Vec<Row> {
    MEASURED_CONSTANTS
        .iter()
        .filter(|constant| constant.role.faces().contains(&face))
        .map(|constant| row_for(constant, face, requested, catalogue))
        .collect()
}

/// **一覧を丸ごと突き合わせる。純粋関数。**
///
/// `main` / `sub` は**上流へ依頼する綴り**（`ModelId` の中身）。
///
/// **一覧も面ごとに受ける**（作業単位 G-3a / [`catalogue_for`]）——
/// **上流は2つありうる**ので、1つに畳むと**片方の綴りをもう片方の一覧で探す**ことになる。
///
/// **並びは定数の順 × 面の順。** [`report`] の行ごとの `warn` はこの順に出るので、
/// **同じ定数の2面が隣り合う。**
/// 見張り番は `the_rows_come_out_in_constant_order_then_face_order`
/// （**面ごとにまとめる形へ組み替える変異**を赤にする。**実際に当てて測った**
/// ——1 赤。**その1本を外すと 1994 全緑**なので、赤にしているのはその1本である）。
///
/// **⚠ 「[`absent_groups`] の並びがここから決まる」は誤りだった**
/// （作業単位 G-3b のレビュー指摘で測り直した / 2026-09-08）。
/// **今日の一覧は5件とも同じ `measured_on` を名乗る**ので、
/// **どちらの入れ子でも「`Main` の1本 → `Sub` の1本」になり、
/// 代弁する定数の並びも同じ。** **組み替えても [`absent_groups`] の出力は変わらない**
/// （**実測**——両方の入れ子で `absent_groups` を組み立てて、同じ値になることを見た）。
/// **着手前のこの doc 行は、何も守っていなかった。**
pub fn check_all(
    main: &str,
    sub: &str,
    main_catalogue: Option<&[String]>,
    sub_catalogue: Option<&[String]>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for constant in &MEASURED_CONSTANTS {
        for face in constant.role.faces() {
            let requested = requested_for(*face, main, sub);
            let catalogue = catalogue_for(*face, main_catalogue, sub_catalogue);
            rows.push(row_for(constant, *face, requested, catalogue));
        }
    }
    rows
}

/// **面1つぶんの判定の内訳**（作業単位 G-3b）。
///
/// # なぜ数を持つのか
///
/// **`GET /admin/upstream`（作業単位 G-10）と画面（D-9）が読む。**
/// 突き合わせの結果は起動ログにしか無く、**5分ごとの引き直しで変わりうる**
/// （上流が別のモデルを載せ直せば、`Match` は `Absent` になる）。
///
/// # ⚠ ずれの2種類を1つに潰さないこと
///
/// [`Drift::Other`] は「測り直す先が決まっている」、
/// [`Drift::Absent`] は「誰が答えるかまだ分からない」。**調査先がまるで違う**
/// （[`report`] の doc）。**潰すと、その違いを画面から復元できない。**
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FaceTally {
    /// 測ったときの相手と、いま応答する相手が同じ。
    pub matched: usize,
    /// 一覧には在るが、測ったときの相手ではない（[`Drift::Other`]）。
    pub drifted_other: usize,
    /// 依頼する綴りが一覧に無い（[`Drift::Absent`]）。
    pub drifted_absent: usize,
    /// 聞けなかった（上流の停止・別実装・明示設定）。
    pub unanswered: usize,
}

impl FaceTally {
    /// 見た行の総数。
    #[must_use]
    pub const fn checked(self) -> usize {
        self.matched + self.drifted_other + self.drifted_absent + self.unanswered
    }

    /// ずれの合計。**内訳は捨てない**（2つの欄はそのまま残る）。
    #[must_use]
    pub const fn drifted(self) -> usize {
        self.drifted_other + self.drifted_absent
    }
}

/// **行を面で絞って数える。純粋関数。数える場所はここ1つ**（作業単位 G-3b）。
///
/// [`report`] のまとめの1行も、`server::upstream` が持つ観測も、
/// **どちらもこの関数の答えから作る。** 別々に数えると、片方を直したときに
/// もう片方が黙って古びる（CLAUDE.md「同じ発見を2箇所に書かない」）。
///
/// 見張り番は `the_summary_line_and_the_returned_tally_count_the_same_rows`。
#[must_use]
pub fn tally(rows: &[Row], face: ModelFace) -> FaceTally {
    let mut counted = FaceTally::default();
    for row in rows.iter().filter(|row| row.face == face) {
        match &row.check {
            ModelCheck::Match => counted.matched += 1,
            ModelCheck::Unanswered => counted.unanswered += 1,
            ModelCheck::Mismatch(Drift::Other { .. }) => counted.drifted_other += 1,
            ModelCheck::Mismatch(Drift::Absent { .. }) => counted.drifted_absent += 1,
        }
    }
    counted
}

/// **1つの面だけを判定して数える。純粋関数**（作業単位 G-3b）。
///
/// 引き直しが使う入口。**[`report`] と同じ答えを返す**
/// （どちらも [`row_for`] で判定し、[`tally`] で数える）。
#[must_use]
pub fn judge_face(face: ModelFace, requested: &str, catalogue: Option<&[String]>) -> FaceTally {
    tally(&check_face(face, requested, catalogue), face)
}

/// **同じ事実を言う [`Drift::Absent`] の行を、1本に畳んだもの**（作業単位 G-5）。
///
/// 鍵は **面 × 測ったときの相手。** `Absent` の判定は
/// [`check`] の中で `measured_on` を一度も読まないので、
/// **定数の綴りは「その事実の影響範囲」であって「事実そのもの」ではない。**
/// だから定数は欄に並べる。
///
/// **⚠ `measured_on` を鍵に含めてあるのは、事実の文面に載るから。**
/// 今日は一覧の5件が全部同じ相手を名乗っている（[`MINISTRAL_14B`]）ので
/// **面ごとに1本になる**が、**別の相手で測った定数が増えれば、そこで自然に割れる。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbsentGroup {
    /// どちらのモデルの話か。**帰属。**
    pub face: ModelFace,
    /// **こちらが依頼する綴り。** 上流の一覧にこれが無い。
    pub requested: String,
    /// **測ったときの相手。**
    pub measured_on: &'static str,
    /// この1本が代弁している定数の綴り。**一覧に並んだ順。**
    pub constants: Vec<&'static str>,
}

/// **`Absent` の行を、面 × 測ったときの相手ごとに畳む。純粋関数。**
///
/// 並びは**最初に現れた順**（[`check_all`] の並び＝一覧に並んだ順 × 面の順）。
/// 畳んだ中身も同じ順で並ぶので、出力は入力から一意に決まる。
///
/// 見張り番は `the_absent_groups_name_every_constant_they_speak_for`
/// （**植えた列**で `assert_eq!`。実物の一覧で見ると、一覧が緩んだときに
/// 期待値も一緒に動いて緑のままになる / CLAUDE.md）。
pub fn absent_groups(rows: &[Row]) -> Vec<AbsentGroup> {
    let mut groups: Vec<AbsentGroup> = Vec::new();
    for row in rows {
        let ModelCheck::Mismatch(Drift::Absent { requested }) = &row.check else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|group| group.face == row.face && group.measured_on == row.constant.measured_on)
        {
            Some(group) => group.constants.push(row.constant.name),
            None => groups.push(AbsentGroup {
                face: row.face,
                requested: requested.clone(),
                measured_on: row.constant.measured_on,
                constants: vec![row.constant.name],
            }),
        }
    }
    groups
}

/// **報告する。決めるのは人**（ADR-0035）。**起動も本番経路も止めない。**
///
/// * [`Drift::Other`] は**行ごとに `warn` を1本。** **その行だけで「どの定数が・
///   どちらの面で・何で測ったのに・いま何が答えるのか」が読めること**——
///   帰属は必ず同じ行に載せる（CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
/// * [`Drift::Absent`] は**面 × 測ったときの相手ごとに1本**（[`absent_groups`]）。
/// * 最後に**まとめの1行**（`info`）。**3値の内訳を同じ行に並べる**ので、
///   「一致に潰す」「ずれに潰す」「聞けなかったをずれに潰す」のどれも、この行が違う値になる。
///
/// **ずれが無くてもまとめは出す。** 出さないと「突き合わせが走ったこと」自体が
/// 観測できず、丸ごと消す変異が緑で通る。
///
/// # ⚠ なぜ `Absent` だけ畳むのか（作業単位 G-5）
///
/// **`llama-server` を上流にすると 9 行が全部 `Absent` になる**
/// （一覧は gguf の絶対パス1件なので、設定の綴りとは永久に一致しない /
/// G-2 と G-6 の実測 / 2026-09-05）。**判定は事実として正確**だが、
/// **9本は「1つの事実 × 9」であって「9つの事実」ではない。**
/// **毎回鳴る警告は読まれなくなる**（このモジュールの doc が自分でそう書いている）。
///
/// **`Other` は畳まない。** あちらは `measured_on` で決まる判定なので、
/// **定数ごとに答えが割れうる**うえ、`measured_at` / `measured_depends_on`
/// （どこを直しに行くか・どう測り直すか）が**定数ごとに違う。**
/// **`Absent` にはその手掛かりが要らない**——どのモデルが答えるのかがまだ分からない以上、
/// 測り直す先も決まらない。**まず綴りを合わせるかどうかを決めるのが先。**
///
/// **⚠ 深刻度は下げていない。** 畳んだのは「読まれる警告だけを残す」ためで、
/// **黙らせるためではない**（見張り番は `the_drift_lines_stay_warnings_and_the_summary_stays_info`）。
/// # ⚠ 一覧は**面ごと**に受ける（作業単位 G-3a / ADR-0039 §決定1）
///
/// **その面の上流へ聞いた答えだけを渡すこと。** 片方の一覧を両方の面へ流用すると、
/// **在るはずの綴りが `Drift::Absent` として鳴る**（＝嘘の警告）。
/// **聞いていない面には `None`**（＝「聞けなかった」）。
/// 理由と実測は [`catalogue_for`] の doc。
/// # **面ごとの内訳を返す**（作業単位 G-3b）
///
/// **`server::upstream` が実行中も持つ**（起動ログにしか無い状態をやめる）。
/// **返すのは [`tally`] の答えそのもの**で、まとめの1行もそこから作る
/// ——**数える場所を2つにしない**（CLAUDE.md「同じ発見を2箇所に書かない」）。
///
/// 引き直しは [`judge_face`] を通る。**判定も数え方も同じ関数**なので、
/// 起動時と5分後で答えの作り方が割れない。
pub fn report(
    main: &str,
    sub: &str,
    main_catalogue: Option<&[String]>,
    sub_catalogue: Option<&[String]>,
) -> (FaceTally, FaceTally) {
    let rows = check_all(main, sub, main_catalogue, sub_catalogue);

    for row in &rows {
        // **`Absent` は畳んでから出す**（下のループ）。ここでは `Other` だけ。
        if let ModelCheck::Mismatch(Drift::Other { upstream }) = &row.check {
            tracing::warn!(
                measured_constant = %row.constant.name,
                measured_at = %row.constant.location,
                measured_face = %row.face.label(),
                // **定数の側**（何で測ったか）。
                measured_on = %row.constant.measured_on,
                // **上流の側**（いま何が答えるか）。**取り違えないこと。**
                upstream_model = %upstream,
                measured_depends_on = %row.constant.depends_on,
                "実測由来の定数は、いま応答するモデルとは別のモデルで測ったものです"
            );
        }
    }

    // **数えるのは [`tally`] ただ1つ。** まとめの3つの欄はここから導く。
    let main_tally = tally(&rows, ModelFace::Main);
    let sub_tally = tally(&rows, ModelFace::Sub);
    let matched = main_tally.matched + sub_tally.matched;
    let drifted = main_tally.drifted() + sub_tally.drifted();
    let unanswered = main_tally.unanswered + sub_tally.unanswered;

    for group in absent_groups(&rows) {
        tracing::warn!(
            measured_face = %group.face.label(),
            measured_on = %group.measured_on,
            // **依頼する側の綴り。** 上流の一覧にこれが無い。
            requested_model = %group.requested,
            // **この1本が代弁している範囲。** 件数と綴りを同じ行に置く。
            measured_constants = group.constants.len(),
            measured_constant_names = %group.constants.join(","),
            "依頼する綴りが上流のモデル一覧にありません。別のモデルが黙って応答します"
        );
    }

    tracing::info!(
        measured_checked = main_tally.checked() + sub_tally.checked(),
        measured_matched = matched,
        measured_drifted = drifted,
        // **「聞けなかった」を独立した数として出す。** ずれと混ぜない。
        measured_unanswered = unanswered,
        // **この2つの欄名は、もともと回避のために付いた**（作業単位 E-2）——
        // `tests/server_context_refresh.rs` が引き直しの行を**素の `contains`** で
        // 拾っていたので、`checked_sub_model=…` を足すと**そちらを掴んで赤くなった。**
        // **⚠ もう回避ではない。B-21 が呼び出し側を `has_field` へ寄せて根治した**
        // （2026-08-28 に実測: 同じ囮を足しても全緑。`has_field` の doc の表）。
        // **`..._model` で終わる欄名を避ける理由は、もう無い。**
        // **この綴りのままにしてあるのは、戻す利得が無いから**（E-2 の見張り番を測り直すことになる）。
        checked_main = %main,
        checked_sub = %sub,
        "実測由来の定数と、上流が応答するモデルを突き合わせました"
    );

    (main_tally, sub_tally)
}

/// **一覧の綴りが、実際にその場所に在るか。** 走査の本体（**ソースは引数で受け取る**）。
///
/// 返るのは**食い違いの説明**（空なら食い違い無し）。
///
/// # なぜ引数でソースを受け取るのか
///
/// 実ファイルだけを読む対照は**自己成就する**——走査の綴りを変える変異は、
/// **その綴りを走査対象のファイルへ書き込む**ので、実ファイル走査は
/// 「1件見つけた」と言って緑のまま通る（CLAUDE.md / 15b の実測）。
/// **植えたソースを同じ判定に通せる形**にしておくこと。
///
/// # 何を見ているか
///
/// 1. `location` のファイルが読めること。
/// 2. そこに定数の綴り（`name`）が在ること。
/// 3. そこに測ったモデルの綴り（`measured_on`）が在ること。
///    **この一覧が現場の doc と同じモデルを名乗っていること**を見る唯一の手段。
///
/// # ⚠ 部分一致では務まらない（**レビューが実測 / 2026-08-28**）
///
/// `contains` で見ていたとき、一覧の `name` を **`GOLDEN_BASELIN`（どこにも存在しない綴り）**
/// に変える変異が **1685 全緑**を通った——`GOLDEN_BASELINE` に部分一致するため。
///
/// **実害の形は改名。** `GOLDEN` を `GOLDEN_PROMPT` へ改名すると、
/// 一覧は `GOLDEN` のまま、走査は部分一致で緑。
/// **「一覧と現場が食い違ったら落ちる」というこの走査の唯一の存在理由が、その改名では働かない。**
///
/// **だから [`mentions_word`] で語境界まで見る。**
pub fn missing_mentions<F>(entries: &[MeasuredConstant], read: F) -> Vec<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut missing = Vec::new();
    for entry in entries {
        let Some(source) = read(entry.location) else {
            missing.push(format!("{}: 読めません", entry.location));
            continue;
        };
        if !mentions_word(&source, entry.name) {
            missing.push(format!(
                "{}: {} が見つかりません",
                entry.location, entry.name
            ));
        }
        if !mentions_word(&source, entry.measured_on) {
            missing.push(format!(
                "{}: {} を名乗っていません（{} の来歴）",
                entry.location, entry.measured_on, entry.name
            ));
        }
    }
    missing
}

/// **その綴りが「語として」在るか。** 前後が英数字か `_` なら、それは別の綴りの一部。
///
/// `GOLDEN_BASELIN` が `GOLDEN_BASELINE` に当たる、という形を塞ぐのがここ
/// （[`missing_mentions`] の doc）。**1件でも語として在れば真**——
/// 同じ綴りは doc と定義の両方に現れるので、隣に日本語が付いた出方が
/// 混じっていても構わない。
fn mentions_word(source: &str, word: &str) -> bool {
    source.match_indices(word).any(|(at, found)| {
        let before = source[..at].chars().next_back();
        let after = source[at + found.len()..].chars().next();
        !is_word_char(before) && !is_word_char(after)
    })
}

/// 語を構成しうる文字か。**綴りの端に接していたら「別の綴りの一部」と見なす。**
fn is_word_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

/// リポジトリ根からの相対パスでソースを読む。**走査の入口。テストにしか無い。**
///
/// **絶対パスに直してから読む**——相対パスはプロセスの作業ディレクトリで解決され、
/// 別のツリーを読みうる（CLAUDE.md / `A-1a` の実測）。
///
/// **`#[cfg(test)]`。** ここだけがファイルを読むので、本番ビルドに残さない
/// （このモジュールが通信もファイルも持たない、という主張をコンパイラ側で保つ）。
#[cfg(test)]
#[must_use]
fn read_repository_file(relative: &str) -> Option<String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path: &std::path::Path = relative.as_ref();
    std::fs::read_to_string(root.join(path)).ok()
}

#[cfg(test)]
mod face_list_tests {
    use super::ModelFace;

    /// **`ModelFace::ALL` に数え漏れが無い**（網羅の `match`。ワイルドカードを足さないこと）。
    #[test]
    fn every_model_face_is_listed_in_all() {
        let listed = |face: ModelFace| match face {
            ModelFace::Main | ModelFace::Sub => ModelFace::ALL.contains(&face),
        };
        for face in [ModelFace::Main, ModelFace::Sub] {
            assert!(listed(face), "{face:?} が ALL に無い");
        }
        assert_eq!(ModelFace::ALL.len(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testlog::{
        captured_under_default_filter, has_field, line_containing, line_with_field,
    };

    /// 測ったときの相手。**フィクスチャは必ず3つを別の綴りにする**
    /// （同値にすると取り違えが検出できない / CLAUDE.md の最頻の穴）。
    const MEASURED: &str = "measured-model";
    /// 本文生成のモデル。**測ったときの相手とは別。**
    const MAIN: &str = "main-model";
    /// 裏方のモデル。**メインとも測ったときの相手とも別。**
    const SUB: &str = "sub-model";

    /// 一覧の綴りを組む。
    fn catalogue(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    /// フィクスチャの3つが本当に違うこと。**同値なら以下は何も守っていない。**
    #[test]
    fn the_fixture_never_makes_two_spellings_equal() {
        assert_ne!(MEASURED, MAIN);
        assert_ne!(MEASURED, SUB);
        assert_ne!(MAIN, SUB);
    }

    /// **一致**——依頼する綴りが一覧に在り、測ったときの相手と同じ。
    #[test]
    fn a_model_that_answers_and_was_measured_on_is_a_match() {
        let models = catalogue(&[MAIN, MEASURED]);
        assert_eq!(check(MEASURED, MEASURED, Some(&models)), ModelCheck::Match);
    }

    /// **ずれ**——一覧に在るが、測ったときの相手ではない。
    #[test]
    fn a_model_that_answers_but_was_not_measured_on_is_a_mismatch() {
        let models = catalogue(&[MAIN, MEASURED]);
        assert_eq!(
            check(MEASURED, MAIN, Some(&models)),
            ModelCheck::Mismatch(Drift::Other {
                upstream: MAIN.to_owned()
            })
        );
    }

    /// **ずれ**——依頼する綴りが一覧に無い（別のモデルが黙って答える）。
    #[test]
    fn a_spelling_the_upstream_does_not_list_is_a_mismatch() {
        let models = catalogue(&[MEASURED]);
        assert_eq!(
            check(MEASURED, MAIN, Some(&models)),
            ModelCheck::Mismatch(Drift::Absent {
                requested: MAIN.to_owned()
            })
        );
    }

    /// **聞けなかった**——一覧そのものが取れていない。**ずれではない。**
    #[test]
    fn an_upstream_that_did_not_answer_is_not_a_mismatch() {
        assert_eq!(check(MEASURED, MAIN, None), ModelCheck::Unanswered);
    }

    /// **空の一覧も「聞けなかった」。** 何も言っていない上流を「別のモデルだ」と言わない。
    #[test]
    fn an_empty_catalogue_is_not_a_mismatch() {
        let models: Vec<String> = Vec::new();
        assert_eq!(
            check(MEASURED, MEASURED, Some(&models)),
            ModelCheck::Unanswered
        );
        assert_eq!(check(MEASURED, MAIN, Some(&models)), ModelCheck::Unanswered);
    }

    /// **未ロードでも「答えるのはその綴り」。** 判定は `state` を見ない
    /// （一覧に在るかどうかだけ）。**冷えた起動で鳴らさないための性質。**
    ///
    /// ここでは「一覧に在る」ことしか渡していない——
    /// **`state` を判定に持ち込むと、この形が通らなくなる。**
    #[test]
    fn an_unloaded_model_still_counts_as_the_one_that_answers() {
        // 上流の一覧は「載っているもの」ではなく「持っているもの」。
        let models = catalogue(&[MEASURED]);
        assert_eq!(check(MEASURED, MEASURED, Some(&models)), ModelCheck::Match);
    }

    /// **面ごとに依頼する綴りが違う。** 帰属の1点。
    #[test]
    fn each_face_asks_for_its_own_model() {
        assert_eq!(requested_for(ModelFace::Main, MAIN, SUB), MAIN);
        assert_eq!(requested_for(ModelFace::Sub, MAIN, SUB), SUB);
    }

    /// **面ごとに、自分の上流の一覧で判定する**（作業単位 G-3a / ADR-0039 §決定1）。
    ///
    /// # なぜ要るのか
    ///
    /// **上流が2つのとき、片方の一覧を両方の面へ流用すると、
    /// 相手の上流には在る綴りが `Drift::Absent` として鳴る**（＝嘘の警告）。
    /// **`Absent` の判定は `measured_on` を一度も読まない**ので、
    /// **どの定数でも同じように鳴る**——実測では 9 行のうち 4 行が丸ごとそうなった。
    ///
    /// # フィクスチャは、2つの一覧を同値にしない
    ///
    /// **本文の一覧にはメインの綴りだけ、抽出の一覧にはサブの綴りだけ。**
    /// 同値にすると、どちらから読んでも同じ答えになり、取り違えが観測できない。
    ///
    /// **`None` の側は「聞けなかった」に落ちること**も同じ回で見る
    /// ——**呼び出し側が「その面へは聞いていない」と申告した場合**
    /// （`PERSONACORE_CONTEXT_TOKENS` を明示した本文の面／往復が失敗した面）。
    #[test]
    fn each_face_is_judged_against_its_own_upstreams_catalogue() {
        let only_main = catalogue(&[MAIN]);
        let only_sub = catalogue(&[SUB]);
        assert_ne!(only_main, only_sub, "前提: 2つの一覧は別であること");

        // ---- 2つの上流。**それぞれの面が、自分の上流の一覧で判定される。** ----
        let rows = check_all(MAIN, SUB, Some(&only_main), Some(&only_sub));
        for row in &rows {
            assert!(
                !matches!(row.check, ModelCheck::Mismatch(Drift::Absent { .. })),
                "自分の上流に在る綴りを「一覧に無い」と判定しないこと: {row:?}"
            );
        }

        // ---- 取り違え。**入れ替えると、両方の面が `Absent` になる。** ----
        let swapped = check_all(MAIN, SUB, Some(&only_sub), Some(&only_main));
        for row in &swapped {
            assert!(
                matches!(row.check, ModelCheck::Mismatch(Drift::Absent { .. })),
                "対照: 一覧を入れ替えれば全部「一覧に無い」になること: {row:?}"
            );
        }

        // ---- 聞いていない面。**「聞けなかった」であって「ずれ」ではない。** ----
        let unheard = check_all(MAIN, SUB, None, Some(&only_sub));
        for row in &unheard {
            match row.face {
                ModelFace::Main => assert_eq!(
                    row.check,
                    ModelCheck::Unanswered,
                    "一覧を聞いていない面は「聞けなかった」であること: {row:?}"
                ),
                ModelFace::Sub => assert_ne!(
                    row.check,
                    ModelCheck::Unanswered,
                    "聞けている面まで黙らせないこと: {row:?}"
                ),
            }
        }
    }

    /// **`Both` は2面に割れ、`Main` / `Sub` は1面。**
    #[test]
    fn a_constant_for_both_models_is_checked_on_both_faces() {
        assert_eq!(
            MeasuredRole::Both.faces(),
            &[ModelFace::Main, ModelFace::Sub]
        );
        assert_eq!(MeasuredRole::Main.faces(), &[ModelFace::Main]);
        assert_eq!(MeasuredRole::Sub.faces(), &[ModelFace::Sub]);
    }

    /// **サブ専用の定数は、サブの綴りとだけ突き合わせる**（帰属）。
    ///
    /// メインの綴りで見てしまう取り違えは、**2つの綴りが違うここでしか観測できない。**
    #[test]
    fn a_sub_only_constant_is_checked_against_the_sub_model() {
        // サブだけが一覧に在り、メインは無い。
        // **上流が1つの形**（同じ一覧を両方の面へ渡す）。2つの上流の形は
        // `each_face_is_judged_against_its_own_upstreams_catalogue` が見る。
        let models = catalogue(&[SUB]);
        let rows = check_all(MAIN, SUB, Some(&models), Some(&models));

        let sub_only: Vec<&Row> = rows
            .iter()
            .filter(|row| row.constant.role == MeasuredRole::Sub)
            .collect();
        assert!(
            !sub_only.is_empty(),
            "サブ専用の定数が一覧に無いなら、この検査は何も守っていない"
        );
        for row in sub_only {
            assert_eq!(row.face, ModelFace::Sub, "サブ専用はサブの面だけで見ること");
            assert_eq!(
                row.check,
                ModelCheck::Mismatch(Drift::Other {
                    upstream: SUB.to_owned()
                }),
                "サブの綴り（{SUB}）と突き合わせること: {row:?}"
            );
        }
    }

    /// **数える場所は [`tally`] ただ1つ**（作業単位 G-3b）。
    ///
    /// [`report`] が返す内訳と、**まとめの1行が言っている数**が同じであること。
    /// 別々に数えていると、片方を直したときにもう片方が黙って古びる
    /// （CLAUDE.md「同じ発見を2箇所に書かない」）。
    ///
    /// **面ごとの内訳が入れ替わらないこと**も同じ回で見る——
    /// フィクスチャは**本文の面だけが `Match` を出す**形にしてあるので、
    /// 取り違えると `main.matched` と `sub.matched` が入れ替わる。
    #[test]
    fn the_summary_line_and_the_returned_tally_count_the_same_rows() {
        // **本文の面だけが「測ったときの相手」を依頼する**（＝ `Match` が出るのは本文だけ）。
        let only_main = catalogue(&[MINISTRAL_14B]);
        // **抽出の面は一覧に在るが、測ったときの相手ではない**（＝ `Other`）。
        let only_sub = catalogue(&[SUB]);

        let mut returned = None;
        let output = captured_under_default_filter(|| {
            returned = Some(report(
                MINISTRAL_14B,
                SUB,
                Some(&only_main),
                Some(&only_sub),
            ));
        });
        let (main, sub) = returned.expect("報告が内訳を返すこと");

        assert_ne!(
            main, sub,
            "前提: 2つの面の内訳が別であること（同値なら取り違えを検出できない）"
        );
        assert!(main.matched > 0, "前提: 本文の面に一致が出ること");
        assert_eq!(sub.matched, 0, "前提: 抽出の面には一致が出ないこと");
        assert!(sub.drifted_other > 0, "前提: 抽出の面はずれること");

        let line = line_containing(&output, "上流が応答するモデルを突き合わせました");
        for (key, value) in [
            ("measured_checked", main.checked() + sub.checked()),
            ("measured_matched", main.matched + sub.matched),
            ("measured_drifted", main.drifted() + sub.drifted()),
            ("measured_unanswered", main.unanswered + sub.unanswered),
        ] {
            assert!(
                has_field(line, &format!("{key}={value}")),
                "まとめの {key} は内訳の合計（{value}）であること: {line:?}"
            );
        }
    }

    /// **1つの面だけを判定しても、両面まとめて判定したときと同じ答えになる**
    /// （作業単位 G-3b）。
    ///
    /// 引き直し（`server::context`）はその面の上流にしか聞かないので、
    /// [`judge_face`] を通る。**起動時（[`report`]）と別の答えを出したら、
    /// 5分後に観測だけが黙って変わる。**
    #[test]
    fn judging_one_face_alone_agrees_with_judging_both() {
        let only_main = catalogue(&[MINISTRAL_14B]);
        let only_sub = catalogue(&[SUB]);

        let mut returned = None;
        captured_under_default_filter(|| {
            returned = Some(report(
                MINISTRAL_14B,
                SUB,
                Some(&only_main),
                Some(&only_sub),
            ));
        });
        let (main, sub) = returned.expect("報告が内訳を返すこと");

        assert_eq!(
            judge_face(ModelFace::Main, MINISTRAL_14B, Some(&only_main)),
            main,
            "本文の面だけを判定しても同じ内訳になること"
        );
        assert_eq!(
            judge_face(ModelFace::Sub, SUB, Some(&only_sub)),
            sub,
            "抽出の面だけを判定しても同じ内訳になること"
        );

        // **対照: 面を取り違えたら別の答えになること**（同値なら上の2本は何も守っていない）。
        assert_ne!(
            judge_face(ModelFace::Sub, MINISTRAL_14B, Some(&only_main)),
            sub,
            "対照: 面と綴りを取り違えたら別の内訳になること"
        );
    }

    /// **聞けなかった面は、全部「聞けなかった」に数える。**
    #[test]
    fn a_face_that_was_not_asked_counts_every_row_as_unanswered() {
        let tally = judge_face(ModelFace::Main, MAIN, None);
        assert!(tally.checked() > 0, "本文の面に見る行があること");
        assert_eq!(tally.unanswered, tally.checked());
        assert_eq!(tally.matched, 0);
        assert_eq!(tally.drifted(), 0);
    }

    /// **一覧に無い綴りは `drifted_absent`、在るが別の相手は `drifted_other`。**
    /// **2つを1つの数に潰さない**（調査先がまるで違う / [`report`] の doc）。
    #[test]
    fn the_two_kinds_of_drift_are_counted_apart() {
        let absent = judge_face(ModelFace::Main, MAIN, Some(&catalogue(&[MINISTRAL_14B])));
        assert!(
            absent.drifted_absent > 0,
            "一覧に無い綴りは absent に数える"
        );
        assert_eq!(absent.drifted_other, 0);

        let other = judge_face(ModelFace::Main, MAIN, Some(&catalogue(&[MAIN])));
        assert!(
            other.drifted_other > 0,
            "一覧に在るが別の相手は other に数える"
        );
        assert_eq!(other.drifted_absent, 0);

        assert_ne!(
            absent, other,
            "前提: 2つの内訳が別であること（潰すと取り違えを検出できない）"
        );
    }

    /// **一覧の全項目が、面ごとに1行ずつ出る。**
    #[test]
    fn every_constant_appears_once_per_face() {
        let expected: usize = MEASURED_CONSTANTS
            .iter()
            .map(|constant| constant.role.faces().len())
            .sum();
        let rows = check_all(MAIN, SUB, None, None);
        assert_eq!(rows.len(), expected);
    }

    /// **行の並びは「定数の順 × 面の順」**（[`check_all`] の doc / 作業単位 G-3b のレビュー指摘）。
    ///
    /// # なぜ要るのか
    ///
    /// [`report`] の行ごとの `warn` はこの順に出るので、**同じ定数の2面が隣り合う。**
    /// **面ごとにまとめる形へ組み替えると、その2行が一覧の長さぶん離れる。**
    ///
    /// **着手前、この並びを見る者は1人も居なかった**
    /// （[`every_constant_appears_once_per_face`] は件数しか見ていない）。
    ///
    /// # 期待値は [`check_all`] の外で組み立てる
    ///
    /// 一覧と [`MeasuredRole::faces`] から作るので、
    /// **入れ子を組み替える変異では期待値が一緒に動かない**
    /// （CLAUDE.md「関数の外にある値で押さえる」）。
    /// **対照も置く**——面ごとにまとめた並びが実際に別の値になることを見る。
    /// 同じなら、この見張り番は組み替えを1つも見ていない。
    #[test]
    fn the_rows_come_out_in_constant_order_then_face_order() {
        let rows = check_all(MAIN, SUB, None, None);
        let actual: Vec<(&str, ModelFace)> = rows
            .iter()
            .map(|row| (row.constant.name, row.face))
            .collect();

        let constant_major: Vec<(&str, ModelFace)> = MEASURED_CONSTANTS
            .iter()
            .flat_map(|constant| {
                constant
                    .role
                    .faces()
                    .iter()
                    .map(move |face| (constant.name, *face))
            })
            .collect();
        let face_major: Vec<(&str, ModelFace)> = [ModelFace::Main, ModelFace::Sub]
            .into_iter()
            .flat_map(|face| {
                MEASURED_CONSTANTS
                    .iter()
                    .filter(move |constant| constant.role.faces().contains(&face))
                    .map(move |constant| (constant.name, face))
            })
            .collect();

        assert_ne!(
            constant_major, face_major,
            "対照: 2つの入れ子が別の並びを作ること（同値なら以下は何も守っていない）"
        );
        assert_eq!(actual, constant_major, "定数の順 × 面の順で並ぶこと");
    }

    /// **ずれた行は、定数側と上流側の綴りを取り違えずに載せる**（帰属）。
    #[test]
    fn the_warning_names_the_constant_side_and_the_upstream_side_apart() {
        let models = catalogue(&[MAIN, SUB]);
        let output = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, Some(&models), Some(&models));
        });

        let line = line_containing(&output, "別のモデルで測ったものです");
        assert!(
            has_field(line, &format!("measured_on={MINISTRAL_14B}")),
            "定数の側は測ったときの相手であること: {line:?}"
        );
        assert!(
            has_field(line, &format!("upstream_model={MAIN}"))
                || has_field(line, &format!("upstream_model={SUB}")),
            "上流の側はいま応答する綴りであること: {line:?}"
        );
        assert!(
            !has_field(line, &format!("measured_on={MAIN}")),
            "定数の側に上流の綴りを載せないこと: {line:?}"
        );
        assert!(
            !has_field(line, &format!("upstream_model={MINISTRAL_14B}")),
            "上流の側に定数の綴りを載せないこと: {line:?}"
        );
    }

    /// **まとめの1行に3値の内訳が並ぶ。**（同じ行の中で見る）
    #[test]
    fn the_summary_line_counts_the_three_verdicts_on_one_line() {
        let models = catalogue(&[MINISTRAL_14B, SUB]);
        let output = captured_under_default_filter(|| {
            let _ = report(MINISTRAL_14B, SUB, Some(&models), Some(&models));
        });
        let line = line_containing(&output, "突き合わせました");

        // メインは測ったときの相手そのもの、サブは別。
        let main_faces = MEASURED_CONSTANTS
            .iter()
            .filter(|constant| constant.role.faces().contains(&ModelFace::Main))
            .count();
        let sub_faces = MEASURED_CONSTANTS
            .iter()
            .filter(|constant| constant.role.faces().contains(&ModelFace::Sub))
            .count();

        assert!(
            has_field(line, &format!("measured_matched={main_faces}")),
            "一致の数はメインの面の数であること: {line:?}"
        );
        assert!(
            has_field(line, &format!("measured_drifted={sub_faces}")),
            "ずれの数はサブの面の数であること: {line:?}"
        );
        assert!(
            has_field(line, "measured_unanswered=0"),
            "聞けた回に「聞けなかった」を数えないこと: {line:?}"
        );
        assert!(
            has_field(
                line,
                &format!("measured_checked={}", main_faces + sub_faces)
            ),
            "突き合わせた総数であること: {line:?}"
        );
    }

    /// **聞けなかった回は、警告を出さずに「聞けなかった」だけを数える。**
    #[test]
    fn an_unanswered_upstream_is_counted_but_never_warned() {
        let output = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, None, None);
        });
        let line = line_containing(&output, "突き合わせました");

        let total: usize = MEASURED_CONSTANTS
            .iter()
            .map(|constant| constant.role.faces().len())
            .sum();
        assert!(
            has_field(line, &format!("measured_unanswered={total}")),
            "全部が「聞けなかった」であること: {line:?}"
        );
        assert!(
            has_field(line, "measured_drifted=0"),
            "聞けなかったをずれに数えないこと: {line:?}"
        );
        assert!(
            !output.contains("別のモデルで測ったものです"),
            "聞けなかった回に警告を出さないこと: {output:?}"
        );
        assert!(
            !output.contains("モデル一覧にありません"),
            "聞けなかった回に警告を出さないこと: {output:?}"
        );
    }

    /// **依頼する綴りが一覧に無い回は、その形の警告が出る。**
    #[test]
    fn a_spelling_the_upstream_does_not_list_is_warned_with_the_requested_spelling() {
        let models = catalogue(&["someone-else"]);
        let output = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, Some(&models), Some(&models));
        });

        let line = line_containing(&output, "モデル一覧にありません");
        assert!(
            has_field(line, &format!("requested_model={MAIN}"))
                || has_field(line, &format!("requested_model={SUB}")),
            "依頼した綴りを載せること: {line:?}"
        );
        assert!(
            !has_field(line, "requested_model=someone-else"),
            "上流が持っている綴りを「依頼した綴り」として載せないこと: {line:?}"
        );
    }

    // ------------------------------- 1つの事実を1本で言う（作業単位 G-5）
    //
    // **`llama-server` を上流にすると、9行が全部 `Drift::Absent` になる**
    // （一覧は gguf の絶対パス1件なので、設定の綴りとは永久に一致しない /
    // G-2 と G-6 が実測 / 2026-09-05）。**判定は事実として正確**だが、
    // **9本は「1つの事実 × 9」であって「9つの事実」ではない。**
    //
    // 根拠は [`check`] そのもの——**`Absent` の枝は `measured_on` を一度も読まない。**
    // つまりその判定は**定数の性質ではない。**（[`Drift::Other`] は違う。
    // あちらは `measured_on` で決まるので、定数ごとに答えが割れうる。）

    /// 面ごとに1本へ畳んだ行を選ぶ。
    fn absent_lines(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| line.contains("モデル一覧にありません"))
            .collect()
    }

    /// **依頼した綴りが無い回の警告は、面ごとに1本**（定数ごとではない）。
    ///
    /// **⚠ 「1本赤くなった」を「守られている」と読まないこと。**
    /// 本数だけでなく、**その1本がどちらの面の話で、何を依頼したか**まで見る。
    #[test]
    fn an_absent_spelling_is_warned_once_per_face_not_once_per_constant() {
        // 上流は**別のモデルだけ**を持っている（`llama-server` と同じ形）。
        let models = catalogue(&["someone-else"]);
        let output = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, Some(&models), Some(&models));
        });

        let lines = absent_lines(&output);
        assert_eq!(
            lines.len(),
            2,
            "面ごとに1本であること（定数ごとに1本ではない）: {output:?}"
        );

        // 面ごとの定数の綴り。**一覧から作るが、畳み方の実装は通していない**
        // （[`absent_groups`] を通すと、畳み方が緩んだときに期待値も一緒に動く）。
        let names_for = |face: ModelFace| -> Vec<&'static str> {
            MEASURED_CONSTANTS
                .iter()
                .filter(|constant| constant.role.faces().contains(&face))
                .map(|constant| constant.name)
                .collect()
        };
        let main_names = names_for(ModelFace::Main);
        let sub_names = names_for(ModelFace::Sub);
        assert_ne!(
            main_names.len(),
            sub_names.len(),
            "前提: 面ごとの件数が違うこと（同値だと取り違えが観測できない）"
        );
        assert_ne!(
            MINISTRAL_14B, MAIN,
            "前提: 測ったときの相手と、依頼する綴りが違うこと（同値だと取り違えが観測できない）"
        );
        assert_ne!(
            MINISTRAL_14B, SUB,
            "前提: 測ったときの相手と、依頼する綴りが違うこと（同値だと取り違えが観測できない）"
        );

        for (face, requested, other, expected) in [
            ("main", MAIN, SUB, &main_names),
            ("sub", SUB, MAIN, &sub_names),
        ] {
            let line = line_with_field(&output, &format!("measured_face={face}"));
            assert!(
                line.contains("モデル一覧にありません"),
                "面 {face} の行は、一覧に無いことを告げる行であること: {line:?}"
            );
            assert!(
                has_field(line, &format!("requested_model={requested}")),
                "面 {face} は自分の綴りを載せること: {line:?}"
            );
            assert!(
                !has_field(line, &format!("requested_model={other}")),
                "面 {face} にもう一方の綴りを載せないこと: {line:?}"
            );
            // **測ったときの相手**（来歴）。**依頼する綴りと同じ欄に潰さないこと。**
            //
            // この2本が無いと、`measured_on` を `requested` に塗り潰す変異が通る
            // （G-5 の是正で実測: 着手前は 1945 全緑）。**純粋関数の側
            // （[`absent_groups`]）は守られていて、行の上だけが無防備だった。**
            //
            // 実害は**自己矛盾した行**——「その綴りで測ったのに、その綴りが一覧に無い」。
            // **本当の来歴が消え、直す先を判断する材料そのものが失われる。**
            // このモジュールの doc が「`measured_on` を鍵に含めてあるのは、
            // 事実の文面に載るから」と書いている、その文面がここ。
            assert!(
                has_field(line, &format!("measured_on={MINISTRAL_14B}")),
                "面 {face} は、測ったときの相手を同じ行に載せること: {line:?}"
            );
            assert!(
                !has_field(line, &format!("measured_on={requested}")),
                "面 {face} の来歴の欄に、依頼する綴りを載せないこと: {line:?}"
            );
            assert!(
                has_field(line, &format!("measured_constants={}", expected.len())),
                "その面の定数の件数を同じ行に載せること: {line:?}"
            );
            // **畳んだ以上、代弁している範囲は行の上に無ければ意味がない**
            // （作業単位 G-5 の変異検査で実測: この欄を空にする変異が 1945 全緑だった。
            // **純粋関数の側だけを見ていて、行の上を誰も見ていなかった。**）。
            assert!(
                has_field(
                    line,
                    &format!("measured_constant_names={}", expected.join(","))
                ),
                "その面が代弁する定数の綴りを、並んだ順に同じ行へ載せること: {line:?}"
            );
        }
    }

    /// **畳んだ行は、自分が代弁している定数を全部名乗る**（純粋関数 / 植えた列）。
    ///
    /// 実物の一覧で見ると、**一覧が緩めば期待値も一緒に動く**
    /// （CLAUDE.md「関数の外にある値で押さえる」）。**ここは植えた列で `assert_eq!`。**
    #[test]
    fn the_absent_groups_name_every_constant_they_speak_for() {
        /// 植えた一覧。**`measured_on` を2種類にして、鍵が面だけでないことを見る。**
        static PLANTED: [MeasuredConstant; 3] = [
            MeasuredConstant {
                name: "ALPHA",
                location: "planted/a.rs",
                measured_on: "model-a",
                depends_on: "トークナイザ",
                role: MeasuredRole::Both,
            },
            MeasuredConstant {
                name: "BETA",
                location: "planted/b.rs",
                measured_on: "model-a",
                depends_on: "チャットテンプレート",
                role: MeasuredRole::Sub,
            },
            MeasuredConstant {
                name: "GAMMA",
                location: "planted/c.rs",
                measured_on: "model-b",
                depends_on: "モデルの饒舌さ",
                role: MeasuredRole::Main,
            },
        ];

        let models = catalogue(&["someone-else"]);
        let mut rows = Vec::new();
        for constant in &PLANTED {
            for face in constant.role.faces() {
                let requested = requested_for(*face, MAIN, SUB);
                rows.push(Row {
                    constant,
                    face: *face,
                    check: check(constant.measured_on, requested, Some(&models)),
                });
            }
        }

        assert_eq!(
            absent_groups(&rows),
            vec![
                AbsentGroup {
                    face: ModelFace::Main,
                    requested: MAIN.to_owned(),
                    measured_on: "model-a",
                    constants: vec!["ALPHA"],
                },
                AbsentGroup {
                    face: ModelFace::Sub,
                    requested: SUB.to_owned(),
                    measured_on: "model-a",
                    constants: vec!["ALPHA", "BETA"],
                },
                AbsentGroup {
                    face: ModelFace::Main,
                    requested: MAIN.to_owned(),
                    measured_on: "model-b",
                    constants: vec!["GAMMA"],
                },
            ],
            "面 × 測ったときの相手ごとに畳み、代弁する定数を並んだ順に全部並べること"
        );
    }

    /// **`Drift::Other` は畳まない**（定数ごとに答えが割れうるので、行も定数ごと）。
    ///
    /// 畳んでよいのは `Absent` の側だけ、という区別そのものを押さえる。
    #[test]
    fn a_drift_that_depends_on_the_constant_is_still_reported_per_constant() {
        // **依頼する綴りは一覧に在る**ので `Absent` ではなく `Other` になる。
        let models = catalogue(&[MAIN, SUB]);
        let output = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, Some(&models), Some(&models));
        });

        let total: usize = MEASURED_CONSTANTS
            .iter()
            .map(|constant| constant.role.faces().len())
            .sum();
        assert_eq!(
            output
                .lines()
                .filter(|line| line.contains("別のモデルで測ったものです"))
                .count(),
            total,
            "定数 × 面のぶんだけ出ること: {output:?}"
        );
        assert!(
            absent_lines(&output).is_empty(),
            "一覧に在る綴りを「一覧にありません」と言わないこと: {output:?}"
        );
    }

    /// **深刻度**——ずれは `warn`、まとめは `info`（作業単位 G-5 / 両向き）。
    ///
    /// **`Absent` を畳んだのは「読まれる警告だけを残す」ため**であって、
    /// **黙らせるためではない。** 深刻度を下げる変異はここで赤になる。
    /// 逆に、まとめの行を `warn` へ上げる変異も赤になる。
    #[test]
    fn the_drift_lines_stay_warnings_and_the_summary_stays_info() {
        let stranger = catalogue(&["someone-else"]);
        let absent = captured_under_default_filter(|| {
            report(MAIN, SUB, Some(&stranger), Some(&stranger));
        });
        let line = line_containing(&absent, "モデル一覧にありません");
        assert!(
            has_field(line, "WARN"),
            "一覧に無いことは警告のままであること: {line:?}"
        );

        let only_main = catalogue(&[MAIN]);
        let other = captured_under_default_filter(|| {
            let _ = report(MAIN, SUB, Some(&only_main), Some(&only_main));
        });
        let line = line_containing(&other, "別のモデルで測ったものです");
        assert!(
            has_field(line, "WARN"),
            "測ったときの相手と違うことは警告のままであること: {line:?}"
        );

        let line = line_containing(&other, "突き合わせました");
        assert!(
            has_field(line, "INFO"),
            "まとめは警告ではないこと（毎回鳴る警告にしない）: {line:?}"
        );
    }

    /// **一覧の綴りは、実際にその場所に在る**（走査 / 実ファイル）。
    #[test]
    fn every_measured_constant_is_where_the_registry_says_it_is() {
        let missing = missing_mentions(&MEASURED_CONSTANTS, read_repository_file);
        assert!(
            missing.is_empty(),
            "一覧と現場が食い違っています:\n{}",
            missing.join("\n")
        );
    }

    /// **走査本体が、植えたソースで実際に落ちる**（対照）。
    ///
    /// 実ファイルだけを読む対照は自己成就する（[`missing_mentions`] の doc）ので、
    /// **テストが用意した文字列を同じ判定に通す。**
    #[test]
    fn the_scan_notices_a_registry_that_drifted_from_the_source() {
        let entry = [MeasuredConstant {
            name: "SOME_CONSTANT",
            location: "somewhere.rs",
            measured_on: MEASURED,
            depends_on: "トークナイザ",
            role: MeasuredRole::Main,
        }];

        // 綴りも来歴も揃っている。
        let complete = format!("const SOME_CONSTANT: u32 = 1; // 実測: {MEASURED}");
        assert!(missing_mentions(&entry, |_| Some(complete.clone())).is_empty());

        // 定数の綴りだけが無い。
        let renamed = format!("const OTHER: u32 = 1; // 実測: {MEASURED}");
        assert_eq!(
            missing_mentions(&entry, |_| Some(renamed.clone())).len(),
            1,
            "定数の綴りが無いことに気付くこと"
        );

        // 来歴だけが無い。
        let unnamed = "const SOME_CONSTANT: u32 = 1;".to_owned();
        assert_eq!(
            missing_mentions(&entry, |_| Some(unnamed.clone())).len(),
            1,
            "測ったモデルを名乗っていないことに気付くこと"
        );

        // ファイルそのものが読めない。
        assert_eq!(
            missing_mentions(&entry, |_| None).len(),
            1,
            "読めないファイルに気付くこと"
        );
    }

    /// **接頭辞を共有する別の綴りしか無い状態を、在ると読まない**
    /// （レビューの実測 / 2026-08-28 / [`missing_mentions`] の doc）。
    ///
    /// # 囮は、実際に狙っている相手と同じ形にする
    ///
    /// 狙っている相手は**改名**（`GOLDEN` → `GOLDEN_PROMPT`）。
    /// だから囮も**「一覧の綴りを、実在する長い綴りの接頭辞にした」形**にする
    /// ——ここを「まったく無関係な綴り」にすると、部分一致でも落ちてしまい、
    /// **語境界を見ているかどうかを一切区別しない。**
    #[test]
    fn a_spelling_that_is_only_a_prefix_of_another_name_does_not_count_as_present() {
        let entry = [MeasuredConstant {
            // **実在するのは `SOME_CONSTANT_EXTRA` のほうだけ**、という状態を作る。
            name: "SOME_CONSTANT",
            location: "somewhere.rs",
            measured_on: MEASURED,
            depends_on: "トークナイザ",
            role: MeasuredRole::Main,
        }];

        // 後ろに続く（改名された）形。
        let renamed = format!("const SOME_CONSTANT_EXTRA: u32 = 1; // 実測: {MEASURED}");
        assert_eq!(
            missing_mentions(&entry, |_| Some(renamed.clone())).len(),
            1,
            "接頭辞に部分一致しただけで「在る」と読まないこと"
        );

        // 前に付く形（接尾辞として埋もれている）。
        let prefixed = format!("const EXTRA_SOME_CONSTANT: u32 = 1; // 実測: {MEASURED}");
        assert_eq!(
            missing_mentions(&entry, |_| Some(prefixed.clone())).len(),
            1,
            "接尾辞に部分一致しただけで「在る」と読まないこと"
        );

        // **来歴の側も同じ規則で見る。** 長い綴りの一部に埋もれていたら「名乗っていない」。
        let longer_model = format!("const SOME_CONSTANT: u32 = 1; // 実測: {MEASURED}9");
        assert_eq!(
            missing_mentions(&entry, |_| Some(longer_model.clone())).len(),
            1,
            "来歴の綴りも、長い綴りの一部なら名乗っていないこと"
        );

        // **1件でも語として在れば良い。** 埋もれた出方が混じっていても落ちない。
        let both =
            format!("SOME_CONSTANT_EXTRA と const SOME_CONSTANT: u32 = 1; // 実測: {MEASURED}");
        assert!(
            missing_mentions(&entry, |_| Some(both.clone())).is_empty(),
            "語として在る出方が1件あれば良いこと"
        );
    }
}
