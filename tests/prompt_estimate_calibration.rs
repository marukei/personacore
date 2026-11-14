//! `prompt::estimate_tokens` を**実測のトークナイザで校正する**ための道具。
//!
//! # 通常は走らない
//!
//! ここのテストは**すべて `#[ignore]`**。LM Studio は常時稼働ではない
//! （`~/.claude/CLAUDE.md` / docs/04-environment.md）ので、素の `cargo test` で
//! 落ちてはいけない。**落ちている状態を「壊れている」と誤診しないこと。**
//!
//! # 校正の手順
//!
//! 1. LM Studio を起動する（`http://127.0.0.1:1234`）。モデルは JIT ロードされるので
//!    事前ロードは不要だが、初回は数十秒かかる。
//! 2. **どのモデルを測るかを決める。** 何も指定しなければ `DEFAULT_MAIN_MODEL`
//!    （E-3 の前と同じ）。**別のモデルを測るなら `PERSONACORE_CALIBRATION_MODEL`** に
//!    綴りを入れる。PowerShell なら:
//!
//!    ```text
//!    $env:PERSONACORE_CALIBRATION_MODEL = "ministral-3-3b-instruct-2512"
//!    ```
//!
//!    * **⚠ `PERSONACORE_SUB_MODEL` では動かない。** あれは**本番の振り分け**であって
//!      校正の対象ではない（勝手に切り替わると、いまと逆向きの同じ事故になる。
//!      理由は `tests/common/calibration.rs` の `ENV_CALIBRATION_MODEL` の doc）。
//!    * **⚠ 読めない綴りはその場で落ちる。** 黙って既定を測ることはない。
//! 3. `cargo test --test prompt_estimate_calibration -- --ignored --nocapture`
//! 4. 標準出力に**そのまま貼れる形**で実測値が出るので、`GOLDEN` と
//!    `GOLDEN_BASELINE` に貼り替える。
//!    **⚠ 別のモデルを測ったなら、`MEASURED_ON_MODEL` と表の doc も一緒に直すこと**
//!    （器具が注意書きを出す）。
//! 5. 実測値が入った状態でもう一度走らせ、緑になることを確認する。
//! 6. `MESSAGE_OVERHEAD_TOKENS` が実測を下回っていたら、
//!    `src/prompt/estimate.rs` の定数を実測値以上へ引き上げる
//!    （**貼れる形で出る。下げる向きには出ない**）。
//!
//! # ⚠ この器具が確かめていないこと
//!
//! **E-3 で足したのは「対象を指せること」までで、「別のモデルを測れること」ではない。**
//! 指した綴りで上流が本当に答えるか（そのモデルが入っているか、
//! トークナイザが別系列で表が総崩れになるか）は**実機でしか分からず、
//! 2026-08-28 時点で実機で確かめた者はまだ居ない**（LM Studio は落ちている）。
//!
//! **⚠ もう1つ、構造的な限界が残っている** —— **表は1モデル分しか持てない。**
//! **緑にできるのは常に片方のモデルだけ**で、両方を差し替えた人は必ずそこへ来る。
//! **説明は `MEASURED_ON_MODEL` の doc に1箇所だけ置いてある**（写しを作らないこと）。
//!
//! 対象の決め方そのもの（既定・環境変数・不正な綴り）は
//! **`tests/calibration_target.rs` が `#[ignore]` 無しで見張っている。**
//! **環境を読む1行**だけは `tests/calibration_env_wiring.rs`（テスト1本の専用バイナリ）。
//!
//! # 何を確かめているか
//!
//! * **`estimate_tokens` が実測を下回らない**こと。`src/prompt/estimate.rs` は
//!   「UTF-8 バイト数は真のトークン数の上界」というバイトレベル BPE の性質だけを
//!   根拠にしている。**その根拠が現実の上流でも成り立つかは、ここでしか分からない。**
//! * **`MESSAGE_OVERHEAD_TOKENS` が実測のメッセージ1件あたりの増分を下回らない**こと。
//!   本文側の上界は「バイト数」で証明できるが、**テンプレート側は 8 という数字への
//!   依存でしかない。** 単体テストは `> 0` しか見ないので、`8 → 1` の変異も素通りする。
//!   この定数が過小だと、履歴40件で 160 トークン規模の過小計上になり、
//!   LM Studio では前方から黙って切り捨てられる＝**最初に消えるのはシステムプロンプト**という、
//!   `prompt` が最も恐れている壊れ方をする（ADR-0013）。**LM Studio で気づく手段はここにしかない。**
//!   （llama は 400 `exceed_context_size_error` で断るので、窓を超えたこと自体はエラーとして見える
//!   / ADR-0052 M5。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
//!   （`finish_reason=length`）。）
//! * 実測値が `GOLDEN` / `GOLDEN_BASELINE` から動いていないこと。動いたら
//!   **上流のトークナイザかチャットテンプレートが変わった**
//!   （モデル差し替え・LM Studio の更新）ということなので、
//!   見積もりの前提を見直す合図になる。
//!
//! # 測り方
//!
//! `max_tokens = 1` で補完を投げ、`usage.prompt_tokens` を読む。
//! チャットテンプレートの分は本文の長さに依らず一定なので、
//! **空文字を送ったときの値をベースラインとして引く**と本文だけのコストが出る。
//!
//! メッセージ1件あたりの固定分は、**件数だけが違う2点を測って差を取る**。
//! ベースラインを引く方式では「テンプレート全体の固定分」と「1件あたりの分」が
//! 分離できないので、`MESSAGE_OVERHEAD_TOKENS` の検証には使えない。
//!
//! `println!` を使っているのは、**実測値の転記こそがこの実行の成果物**だから。
//! 本番コードのログは `tracing`（CLAUDE.md 5節）。

use std::time::Duration;

use chrono::Utc;
use personacore::domain::{
    AxisDiscoverySlots, Character, CharacterId, DisplayName, Handle, Memory, MemoryCategory,
    MemoryContent, MemoryId, Persona, Subject,
};
use personacore::llm::{DEFAULT_ENDPOINT, ModelId};
use personacore::prompt::{
    CountedParts, MEASURED_ORDINARY_LOOKALIKES, MESSAGE_OVERHEAD_TOKENS, PromptInput, SafeText,
    TokenBudget, build_prompt, estimate_tokens,
};
use serde_json::{Value, json};

mod common;

use common::calibration::{
    COMPARED_SHAPE_CATEGORY, CONTROL_LISTS, ControlListKind, ControlProbe, LookalikeProbe,
    assembled_shape, calibration_body, calibration_target_from_env, compared_shape_spelling,
    golden_block, measured_on_note, overhead_block, probe_control, probe_shape, shape_block,
};
use common::fixture_scale;

/// **この表と `GOLDEN_BASELINE` と `MESSAGE_OVERHEAD_TOKENS` を測ったモデル**
/// （[ADR-0033](../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md) §決定1）。
///
/// # なぜ `DEFAULT_MAIN_MODEL` を書かないのか
///
/// **これは「既定のモデル」ではなく「実測したときの相手」という事実である。**
/// 今日はたまたま同じ綴りだが、既定が変わっても**表の値が測り直されるわけではない。**
/// 定数を参照すると、既定を変えた瞬間に**表が黙って別のモデルの値を名乗る。**
///
/// **測る相手（`PERSONACORE_CALIBRATION_MODEL`）がこれと違うと、器具が注意書きを出す。**
/// 貼り替えるときは、ここも一緒に直すこと。
///
/// # ⚠ 表は1モデル分しか持てない（**E-3 で残った限界。ここが唯一の記述**）
///
/// `GOLDEN` / `GOLDEN_BASELINE` / この定数は**1組しかない。**
/// **対象は指せるようになったが、緑にできるのは常に片方のモデルだけである。**
///
/// **メインとサブを両方差し替えると、doc の手順どおりに進めた人が必ずここへ来る**
/// （[docs/04-environment.md](../docs/04-environment.md) §0 の表は、**両方に手順1を要求している**）:
///
/// 1. サブの綴りを入れて回す → 2. 出た値を `GOLDEN` へ貼り、この定数もサブへ直す →
/// 3. **次にメインで回すと `the_estimate_is_an_upper_bound_of_the_real_tokenizer` が
///    「ゴールデン表とずれた」で赤くなり続ける。**
///
/// **⚠ どちらのモデルの値を表に置くかは、人が決めるしかない。**
/// **器具にできるのは「いま測っている相手が表と違う」と言うことまで**（`measured_on_note`）。
/// **両モデル分を持てる形にするのは、E-3 の外である。**
///
/// # ⚠ `MESSAGE_OVERHEAD_TOKENS` は壊れない（**区別すること**）
///
/// あちらは**上界**で、貼る値が `max(実測, いまの定数)` なので、
/// **両方のモデルを回せば大きい側へ収束する**（下げる向きには出ない / `overhead_block` の doc）。
/// **片方しか持てないのは `GOLDEN` / `GOLDEN_BASELINE` だけ。**
const MEASURED_ON_MODEL: &str = "ministral-3-14b-instruct-2512";

/// 校正に使う代表文字列と、実測済みのトークン数。
///
/// **`None` は「まだ実測していない」。** 手順3を実行して埋めること。
/// 埋まっている項目は実測値と一致することまで確かめる。
/// 実測日: 2026-08-13 / モデルは [`MEASURED_ON_MODEL`]（LM Studio）
///
/// 括弧内は `estimate_tokens`（= UTF-8 バイト数）との比。
/// **1.0 に近いほど上界が張り付いている**＝係数方式なら破れていた場所。
const GOLDEN: [(&str, Option<u32>); 8] = [
    // 日本語散文（ペルソナ・記憶の典型）。66 バイト → 3.67 倍の余裕。
    ("落ち着いた口調で話す。相手の話を最後まで聞く。", Some(18)),
    // 日本語の短文（記憶1件の典型）。12 バイト → 3.00 倍。
    ("猫が好き", Some(4)),
    // 英文散文（BPE が最もよく圧縮する側）。43 バイト → 4.30 倍。
    ("The quick brown fox jumps over the lazy dog.", Some(10)),
    // 空白区切りの単字。**「N 文字/トークン」の係数が最初に破れる形。**
    // 39 バイト → 1.95 倍。「2文字/トークン」だと 20 で**ちょうど破れる**。
    ("a b c d e f g h i j k l m n o p q r s t", Some(20)),
    // UUID。語彙に乗らずバイトへ落ちる。36 バイト → **1.09 倍**。
    // 「2文字/トークン」だと 18 で、実測 33 の**半分強しか見積もらない**。
    ("f47ac10b-58cc-4372-a567-0e02b2c3d479", Some(33)),
    // base64 風。40 バイト → 1.25 倍。
    ("aGVsbG8gd29ybGQhIHRoaXMgaXMgYSB0ZXN0Lg==", Some(32)),
    // アストラル面（1文字 = 4バイト）。人名で現実に来る。
    // 12 バイト → **1.00 倍。上界に完全に張り付く。**
    // 旧実装の「2トークン/文字」だと 6 で、実測 12 の半分。
    ("𠮷𩸽𠀋", Some(12)),
    // 記号列。27 バイト → 1.93 倍。
    ("①②③Ω±§→←↑↓", Some(14)),
];

/// 空の user メッセージ1件を送ったときの `usage.prompt_tokens`。
///
/// 実測日: 2026-08-13 / モデルは [`MEASURED_ON_MODEL`]（LM Studio）
///
/// **この 536 はチャットテンプレートの固定分ではない。**
/// **LM Studio は system メッセージを省略すると、モデル既定のシステムプロンプトを注入する。**
/// 536 のほぼ全部がそれ。実測:
///
/// | リクエスト | `prompt_tokens` |
/// |---|---|
/// | user のみ（空文字） | **536** |
/// | system「あ」+ user「い」 | **7** |
///
/// **自前の system を送ると置き換わる。** personacore は必ずペルソナを system で送るので
/// 本番でこの 536 は載らない（→ `docs/archive/04-environment-lm-studio.md`）。
///
/// ベースラインとしては引き算で消えるので、`GOLDEN` の校正には影響しない。
/// ここが動いたら**テンプレートか既定システムプロンプトが変わった**ので、
/// `MESSAGE_OVERHEAD_TOKENS` を測り直すこと。
const GOLDEN_BASELINE: Option<u32> = Some(536);

/// メッセージ1件あたりの固定分を測るための本文。**1バイトの ASCII 1文字。**
///
/// 本文を空にすると、テンプレートによっては空メッセージごと畳まれて増分が測れない。
/// 1文字分（1トークン）は増分に乗るが、**その分だけ厳しく判定される側**なので問題ない。
const PROBE: &str = "a";

/// 上流の応答を待つ上限。**JIT ロードを飲むために長めに取る。**
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// 校正用の HTTP クライアント。
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("HTTP クライアントを構築できること")
}

/// **測る相手。** 未設定なら `DEFAULT_MAIN_MODEL`（手順2）。
///
/// **決め方は `tests/common/calibration.rs` の `calibration_target`** で、
/// **そちらは `#[ignore]` 無しで見張られている**（`tests/calibration_target.rs`）。
fn target() -> ModelId {
    calibration_target_from_env().unwrap_or_else(|err| {
        panic!(
            "PERSONACORE_CALIBRATION_MODEL の綴りが読めません（未設定なら既定を測ります）: {err}"
        )
    })
}

/// user メッセージ1件で `usage.prompt_tokens` を測る。
async fn prompt_tokens(client: &reqwest::Client, target: &ModelId, content: &str) -> u32 {
    prompt_tokens_of(
        client,
        target,
        json!([{"role": "user", "content": content}]),
    )
    .await
}

/// 任意のメッセージ列で `usage.prompt_tokens` を測る。
///
/// **`model` は決め打たない**（E-3 の前はここが `DEFAULT_MAIN_MODEL` だった）。
/// 組み立ては `common::calibration::calibration_body` にあり、
/// **`the_request_body_names_the_selected_target` が見張っている。**
async fn prompt_tokens_of(client: &reqwest::Client, target: &ModelId, messages: Value) -> u32 {
    let body = calibration_body(target, messages);

    let response = client
        .post(format!("{DEFAULT_ENDPOINT}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .expect("LM Studio が起動していること（手順1）");
    assert!(
        response.status().is_success(),
        "上流が {} を返した",
        response.status()
    );

    let payload: Value = response.json().await.expect("JSON が返ること");
    let tokens = payload["usage"]["prompt_tokens"]
        .as_u64()
        .expect("usage.prompt_tokens が載っていること");

    u32::try_from(tokens).expect("トークン数が u32 に収まること")
}

/// **見積もりが実測を下回らない**ことを、実際のトークナイザで確かめる。
#[tokio::test]
#[ignore = "LM Studio の起動が要る。手順はこのファイルの doc を参照"]
async fn the_estimate_is_an_upper_bound_of_the_real_tokenizer() {
    let target = target();
    let client = client();

    // チャットテンプレートの固定分。本文の長さに依らないので、これを引く。
    let baseline = prompt_tokens(&client, &target, "").await;

    let mut failures: Vec<String> = Vec::new();
    let mut rows: Vec<(&str, u32)> = Vec::new();

    if let Some(expected) = GOLDEN_BASELINE
        && expected != baseline
    {
        failures.push(format!(
            "ベースラインがずれた（チャットテンプレートが変わった可能性）: \
             実測 {baseline} != 表 {expected}。MESSAGE_OVERHEAD_TOKENS も測り直すこと"
        ));
    }

    for (text, golden) in GOLDEN {
        let measured = prompt_tokens(&client, &target, text)
            .await
            .saturating_sub(baseline);
        let estimated = estimate_tokens(text);
        rows.push((text, measured));

        if estimated < measured {
            failures.push(format!(
                "過小評価: {text:?} 見積 {estimated} < 実測 {measured}"
            ));
        }
        if let Some(expected) = golden
            && expected != measured
        {
            failures.push(format!(
                "ゴールデン表とずれた（上流のトークナイザが変わった可能性）: \
                 {text:?} 実測 {measured} != 表 {expected}"
            ));
        }
    }

    // **貼れる塊は1回で出す。** 4本は並行に走るので、行ごとに書くと出力が混ざって
    // 「そのまま貼れる」が崩れる（`golden_block` の doc）。
    println!(
        "{}",
        golden_block(&target, MEASURED_ON_MODEL, baseline, &rows)
    );

    // **判定は変えない。** ずれの理由が「別のモデルを測っている」ことかもしれない、
    // という手掛かりだけを添える（ADR-0033 §決定1）。
    let report = match measured_on_note(MEASURED_ON_MODEL, &target) {
        Some(note) => format!("{note}\n{}", failures.join("\n")),
        None => failures.join("\n"),
    };
    assert!(failures.is_empty(), "{report}");
}

/// **実測した制御トークンが「いまも特殊」で、かつ「無害化が効いている」こと。**
///
/// # 規則が正、実測データは検証データ
///
/// 防御の実体は `src/prompt/safe.rs` の**構造的な規則**
/// （`[`…`]` / `<`…`>` の中身が ASCII 英数字・`_`・`/`・`|` だけなら閉じ括弧の手前へ空白）。
/// **ループは規則ではなく実測データ側（`MEASURED_CONTROL_TOKEN_SPELLINGS`）を回す。**
/// 規則の定義を回してしまうと「規則に載っていない綴り」を永遠に検出できない
/// ——列挙方式が破綻したのと同じ理由で、検証まで自己言及にしてはならない。
///
/// # なぜ両方向を見るのか
///
/// * **綴りがいまも 1 トークンであること**だけを見ると、無害化が
///   空振りしていても気付けない（綴りが特殊なまま素通りしている状態）。
/// * **無害化後が 2 トークン以上であること**だけを見ると、
///   モデル差し替えで綴りが特殊でなくなった後も、無害化し続けていることに気付けない。
/// * **対照（`MEASURED_ORDINARY_LOOKALIKES`）が 1 トークンでないこと**を見ると、
///   「新たに特殊になった」方向のドリフトも捕まる。
///
/// # ⚠ 3つ目の一覧だけは、素の綴りを判定しない（作業単位 B-22）
///
/// `MEASURED_MODEL_DEPENDENT_SPELLINGS` は
/// **`ministral` で 6 トークン・`qwen` で 1 トークン**という綴りを持つ。
/// **上の2つのどちらに入れても、片方のモデルで必ず落ちる。**
/// **落ちても直すものが無い**ので、ここでは
/// **「無害化後が特殊でないこと」だけ**を当てる（モデルに依らない性質）。
/// 理由は `src/prompt/safe.rs` のその定数の doc（**ここに写しを作らない**）。
///
/// # 測り方
///
/// `GOLDEN` と同じく、空の user メッセージ1件のベースラインを引く。
/// 綴りが 1 トークンなら差は 1 になる。無害化後の形は
/// **`SafeText::sanitize` に通して求める**（表に転記しない）。転記すると、
/// 無害化の実装を変えたときに表だけ古いまま緑になる。
///
/// # ⚠ 判定も文面も、この本体には無い（作業単位 B-35）
///
/// **3つのループはどれも `common::calibration` の型に判定を委ねている**
/// （[`ControlProbe`] / [`LookalikeProbe`]）。**ここは `#[ignore]` なので、
/// 中に書いた主張は素の `cargo test` では1行も走らない**
/// ——B-25 / B-27 の実測では、器具の中の純粋な assert を骨抜きにしても全緑だった。
/// **この本体に判定を書き足さないこと。**
#[tokio::test]
#[ignore = "LM Studio の起動が要る。手順はこのファイルの doc を参照"]
async fn measured_control_tokens_are_still_special_after_neutralizing() {
    let target = target();
    let client = client();
    let baseline = prompt_tokens(&client, &target, "").await;

    // **どのモデルに当てた結果なのかを名乗る**（ADR-0033 §決定1）。
    // ここは貼る値ではなく検証データなので、注意書きは出さず相手だけを言う。
    println!("制御トークンの当て直し（実測: {target}）");

    let mut failures: Vec<String> = Vec::new();

    // **どの一覧をどちらの判定で回すかは、共有の表が決める**（作業単位 B-35 の是正）。
    // **ここで `kind` を選ぶと、2行を入れ替える変異が誰にも見えない**
    // （レビュアーが実測: 両向きとも 1867 全緑）。表の見張り番は
    // `tests/calibration_target.rs` の `each_control_list_is_paired_with_its_own_judgement`。
    for (kind, spellings) in CONTROL_LISTS {
        for &spelling in spellings {
            let probe = measure_control(&client, &target, baseline, kind, spelling).await;
            println!("{}", probe.report());
            failures.extend(probe.failures());
        }
    }

    // **対照。** 特殊に見えるが特殊でないもの。ここが 1 になったら、
    // 「新たに特殊になった」方向のドリフト（＝その一覧の帰属が崩れた合図）。
    //
    // **⚠ 直す先は「規則の内側の針か、外側の針か」で分かれる**（作業単位 B-27）。
    // **判定も文面も `common::calibration` が持つ**——ここは `#[ignore]` なので、
    // **中に書いた主張は素の `cargo test` では1行も走らない。**
    for spelling in MEASURED_ORDINARY_LOOKALIKES {
        let probe = LookalikeProbe {
            spelling,
            raw: prompt_tokens(&client, &target, spelling)
                .await
                .saturating_sub(baseline),
        };
        println!("{}", probe.report(&target));

        if let Some(failure) = probe.failure() {
            failures.push(failure);
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// 綴りと**その無害化後**を測って [`ControlProbe`] へ入れる。
///
/// **配線も判定も文面も `common::calibration` が持つ**（作業単位 B-35）——
/// ここは `#[ignore]` なので、**中に書いた主張は素の `cargo test` では1行も走らない。**
async fn measure_control<'a>(
    client: &reqwest::Client,
    target: &ModelId,
    baseline: u32,
    kind: ControlListKind,
    spelling: &'a str,
) -> ControlProbe<'a> {
    // **測るのは2つ。** どちらがどの欄かは `probe_control` が決める
    // （器具の中に配線を置くと、欄の入れ替えが緑を通る / B-25 のレビュー）。
    let neutralized = SafeText::sanitize(spelling).to_string();
    let mut measured: Vec<(String, u32)> = Vec::new();
    for text in [spelling, neutralized.as_str()] {
        let tokens = prompt_tokens(client, target, text)
            .await
            .saturating_sub(baseline);
        measured.push((text.to_owned(), tokens));
    }

    probe_control(kind, spelling, |text| {
        measured
            .iter()
            .find(|(measured_text, _)| measured_text == text)
            .map(|(_, tokens)| *tokens)
            .unwrap_or_else(|| panic!("測っていない文字列を判定に使っている: {text:?}"))
    })
}

/// **分類に `INST` を入れた記憶が、プロンプトで `[INST]` にならないこと**を実測で確かめる。
///
/// # なぜ組み上がった行で測るのか
///
/// `SafeText` は `"INST"` を無変換で通す（括弧を含まないので当然）。
/// 括弧は `Seg::Lit` 側にあり、**`- [` + `INST` + `] ` の連結で綴りが組み上がる。**
/// 単体テストは「原形が載らない」ことまでしか言えないので、
/// **本当に特殊トークンでなくなったか**はここでしか分からない。
///
/// # 測り方
///
/// 実際に `build_prompt` で組んだ行と、**その空白を取り除いた行**（＝防御が無かった
/// 場合の形）の2点を測る。防御が効いていれば、崩した側の方が**トークン数が多い**
/// （特殊トークン1個が普通の文字列に戻るため）。
///
/// 実測（2026-08-14 / `ministral-3-14b-instruct-2512`）:
/// `> - [INST] 猫が好き` = 9 / `> - [INST ] 猫が好き` = 11。
///
/// # ⚠ 2点の差は、綴りが特殊な上流でしか読めない（作業単位 B-25 / 2026-08-29）
///
/// **`qwen/qwen3.5-9b` では `[INST]` がただの文字列**（2〜7 トークン）なので、
/// 崩しても数はほとんど動かず、**`8` 対 `8` になった**（C-14 が実測）。
/// **当時はここで落ち、「規則と一覧を見直すこと」という直す先の無い指示が出た**
/// ——規則は正しく崩し続けており、直すものは何も無かった。
///
/// **いまは素の綴りも一緒に測り、特殊トークンでない上流では判定しない**（報告はする）。
/// **`MEASURED_MODEL_DEPENDENT_SPELLINGS` で B-22 が採った形の「片方」**
/// ——**素の値は判定しない**側だけを採っている。
/// **もう片方（無害化後が 1 トークンでないこと）はここに無く、
/// 兄弟のループが同じ綴り・同じ上流で当てている**
/// （理由と前提は `common::calibration::COMPARED_SHAPE_CATEGORY` の doc）。
///
/// * **判定も、行の組み立ても、報告の塊も `common::calibration`** にあり、
///   **`#[ignore]` 無しで見張られている**（`tests/calibration_target.rs`）。
///   **この本体に主張を書き足さないこと**——素の `cargo test` では1行も走らない。
/// * **形が崩れること自体**は、実機の要らない単体テストが実測データ全件で見ている
///   （`src/prompt/safe.rs` の `the_rule_covers_every_measured_control_token`）。
#[tokio::test]
#[ignore = "LM Studio の起動が要る。手順はこのファイルの doc を参照"]
async fn a_category_named_after_a_control_token_is_not_special_in_the_prompt() {
    // 分類と、そこから組み上がる綴り。**どちらも共有の定数から来る**——
    // **「その綴りを兄弟のループも測っている」という前提を、`#[ignore]` の外で見張るため**
    // （`the_compared_spelling_is_also_measured_by_the_sibling_loop`）。
    let spelling = compared_shape_spelling();

    let character = Character {
        id: CharacterId::from_raw_for_tests(1),
        handle: Handle::parse("aoi").expect("正当な名前であること"),
        display_name: DisplayName::parse("葵").expect("正当な表示名であること"),
        base_persona: Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
        output_format_instruction: None,
        stat_scale: fixture_scale(),
        stat_drift_mode: None,
        axis_discovery_slots: AxisDiscoverySlots::NONE,
        created_at: Utc::now(),
        archived_at: None,
    };
    let memories = [Memory {
        id: MemoryId::from_raw_for_tests(1),
        character_id: CharacterId::from_raw_for_tests(1),
        subject: Subject::User,
        content: MemoryContent::parse("猫が好き").expect("正当な本文であること"),
        // **括弧を含まない分類。** `MemoryCategory::parse` は当然これを通す。
        category: Some(
            MemoryCategory::parse(COMPARED_SHAPE_CATEGORY).expect("正当な分類であること"),
        ),
        created_at: Utc::now(),
        locked: false,
    }];

    let built = build_prompt(
        &PromptInput {
            character: &character,
            stats: &[],
            attributes: &[],
            agreements: &[],
            memories: &memories,
            history: &[],
            history_turns_before: None,
            user_message: "こんにちは",
        },
        TokenBudget::default(),
        // 何も数えていない表 = バイト数の計画（ADR-0052 / B-60c）。この器具はバイト数の側を測る。
        &CountedParts::default(),
    )
    .expect("既定の予算に収まること");

    let system = &built
        .messages
        .first()
        .expect("システムプロンプトがあること")
        .content;
    let line = system
        .lines()
        .find(|line| line.contains("猫が好き"))
        .expect("記憶の行があること")
        .to_owned();

    // **行の組み立ても判定も、上流を呼ばない部分は共有の純粋関数**（作業単位 B-25）。
    // **ここは `#[ignore]` なので、この中に書いた主張は誰も見ていない。**
    let shape = assembled_shape(&line, &spelling).unwrap_or_else(|err| panic!("{err}"));

    let target = target();
    let client = client();
    let baseline = prompt_tokens(&client, &target, "").await;

    // **測るだけ。** どれをどの欄へ入れるかは `probe_shape` が決める
    // （欄の入れ替えは、器具の中に配線を置くと 1749 全緑を通り抜ける / B-25 のレビュー）。
    let mut measured: Vec<(String, u32)> = Vec::new();
    for text in shape.texts() {
        let tokens = prompt_tokens(&client, &target, text)
            .await
            .saturating_sub(baseline);
        measured.push((text.to_owned(), tokens));
    }

    let probe = probe_shape(&shape, |text| {
        measured
            .iter()
            .find(|(measured_text, _)| measured_text == text)
            .map(|(_, tokens)| *tokens)
            .unwrap_or_else(|| panic!("測っていない文字列を判定に使っている: {text:?}"))
    });

    // **1回で出す**（並行実行で割れないように）。中身は `shape_block` が持つ。
    println!("{}", shape_block(&target, &shape, &probe));

    if let Some(failure) = probe.failure() {
        panic!("{failure}");
    }
}

/// **`MESSAGE_OVERHEAD_TOKENS` が実測のメッセージ1件あたりの増分を下回らない。**
///
/// # なぜこの検証がここにしか置けないか
///
/// 本文側の見積もりは「UTF-8 バイト数はバイトレベル BPE の上界」という**証明**に
/// 支えられているが、**テンプレート側にそれに相当する根拠は無い。** 8 という数字は
/// 実測して転記した値であり、それを保つ仕組みが無ければただの願望になる。
///
/// 過小だったときの壊れ方: 履歴40件で 160 トークンの過小計上 → プロンプトが上流の
/// コンテキストを超える → LM Studio では**前方から黙って切り捨てられる** → 最初に消えるのは
/// システムプロンプト（ペルソナ・性格・記憶）。エラーも出ず応答も返るので、
/// **「性格を変えても応答が変わらない」という形だけが現れる**（ADR-0013）。
/// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
/// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
///
/// # 測り方
///
/// メッセージ**件数だけ**が違う2点を測る。本文は両方とも `PROBE` なので、
/// 差分は「1件あたりのテンプレート固定分 ＋ `PROBE` の1トークン」× 2 件。
///
/// 割り算は**切り上げ**（`div_ceil`）。切り捨てると、実際の増分が 8.5 のときに 8 と出て
/// 過小評価を見逃す。`PROBE` の1トークンを引かないのと合わせて、
/// **判定は常に厳しい側へ倒してある**（余裕が無いなら鳴ってよい）。
#[tokio::test]
#[ignore = "LM Studio の起動が要る。手順はこのファイルの doc を参照"]
async fn the_message_overhead_constant_is_not_below_the_measured_one() {
    let target = target();
    let client = client();

    let one = prompt_tokens_of(
        &client,
        &target,
        json!([{"role": "user", "content": PROBE}]),
    )
    .await;
    let three = prompt_tokens_of(
        &client,
        &target,
        json!([
            {"role": "user", "content": PROBE},
            {"role": "assistant", "content": PROBE},
            {"role": "user", "content": PROBE},
        ]),
    )
    .await;

    assert!(
        three >= one,
        "メッセージを増やしたのに prompt_tokens が減った: {one} → {three}"
    );

    let per_message = (three - one).div_ceil(2);

    // **貼れる塊は1回で出す。** 引き上げが要るときは
    // `pub const MESSAGE_OVERHEAD_TOKENS: u32 = …;` の形で出る
    // （**下げる向きには出ない**——`overhead_block` の doc）。
    println!(
        "{}",
        overhead_block(
            &target,
            MEASURED_ON_MODEL,
            one,
            three,
            per_message,
            MESSAGE_OVERHEAD_TOKENS,
        )
    );

    assert!(
        per_message <= MESSAGE_OVERHEAD_TOKENS,
        "MESSAGE_OVERHEAD_TOKENS が実測を下回った: 定数 {MESSAGE_OVERHEAD_TOKENS} < 実測 {per_message}。\
         src/prompt/estimate.rs の定数を実測値以上へ引き上げること"
    );
}
