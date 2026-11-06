//! **「このモデルで特殊な綴りは何か」を探す器具**（作業単位 E-1）。
//!
//! # なぜ要るのか
//!
//! 既存の2本（`tests/prompt_estimate_calibration.rs`）は
//! **既知の一覧（`MEASURED_CONTROL_TOKEN_SPELLINGS` ほか）を当て直すだけ**なので、
//! **別系列のモデルが持ち込んだ新しい綴りは1件も見つからない。**
//! `<|im_start|>` が `qwen` で本物の制御トークンだったことは、
//! **`ministral` の下ではどの器具からも見えなかった**（B-22 / docs/04-environment.md 手順2）。
//!
//! **ここは上流に訊かない。** gguf の `tokenizer.ggml.token_type` を直接読み、
//! **特殊になる種別の綴りを全部取り出して、無害化がそれを覆えるかを当てる。**
//! 手順は docs/04-environment.md の「⚠ 無いもの（E-1 / 最優先の宿題）」の 1〜4。
//!
//! # ⚠ 「値が 3（CONTROL）」だけでは足りない（**手順の訂正 / E-1 で実測**）
//!
//! doc の手順は **`token_type` が 3 の id を数える**と書いている。**足りない。**
//! **`qwen/qwen3.5-9b` の gguf を読むと、種別は 1 / 3 / 4 / 5 の4種類**あり、
//! **4（USER_DEFINED）も上流では 1 トークンに落ちる。** 実測（2026-09-02 / この機）:
//!
//! | 綴り | 種別 | 素 | 無害化後 |
//! |---|---|---|---|
//! | `<\|im_start\|>` | 3 CONTROL | **1** | 6 |
//! | `<tool_call>` | **4 USER_DEFINED** | **1** | 4 |
//! | `<think>` | **4 USER_DEFINED** | **1** | 3 |
//! | `</think>` | **4 USER_DEFINED** | **1** | 3 |
//! | `[PAD248077]` | 5 UNUSED | 9 | — |
//!
//! **→ 判定する種別は [`JUDGED_TOKEN_TYPES`]（3 と 4）。**
//! **それ以外の「NORMAL でない種別」は、判定せず報告だけする**（[`Survey::watched`]）
//! ——**一覧の外を測らないのは、測っていないのと同じ**（CLAUDE.md）だが、
//! **1 トークンになると実測していない種別で落とすと、直す先の無い警報になる**（B-25 の教訓）。
//!
//! # 何を「覆えている」と呼ぶか（**規則そのものではない。読み違えないこと**）
//!
//! 判定は [`survives_sanitize`] ——**その綴りを普通の文の中に書いて
//! `SafeText::sanitize` を通したとき、綴りが原形のまま残るか。**
//!
//! * **`control_token_shape_len` を直接は呼べない**（`src/prompt/safe.rs` の私的な関数で、
//!   この作業単位は規則を触らない）。**公開 API だけで決まる形にしてある。**
//! * したがって判定は**規則より広い** —— 制御文字の置換や空白の畳み込みで
//!   綴りが崩れた場合も「覆えている」に数える。
//!   **「覆えている」＝「利用者がその綴りを書いても、原形では上流へ届かない」**であり、
//!   **「形の規則が捕まえた」とは限らない。**
//! * **逆向きには嘘をつかない** —— 「覆えていない」と出たものは、
//!   **`sanitize` を通しても原形のまま残る**（＝注入経路がそのまま開いている）。
//!   **報告に出す価値があるのはこちら側で、そこは正確である。**
//!
//! # ⚠ 「覆えている」は「注入できない」ではない（**読み違えやすい**）
//!
//! **この器具は「崩した形が、上流で 1 トークンでないこと」を一切見ていない。**
//! 見ているのは**綴りが原形で残らないこと**だけである。
//! **`[INST ]` が特殊トークンであるようなモデルなら、この器具は「覆えている」と言い、
//! 実際には素通りする。**
//!
//! **それを測るのは実機側の器具**（`tests/prompt_estimate_calibration.rs` の
//! `measured_control_tokens_are_still_special_after_neutralizing` の `broken < 2`）**で、
//! あちらが当てるのは 29 綴りと 1 綴り。** **ここが回す 1000 件には、対応する測定が無い。**
//!
//! # ⚠ この器具では見つけられないもの（**正直に置いておく**）
//!
//! * **上の「崩した形が本当に特殊でなくなったか」**（実機が要る。ここは gguf しか読まない）。
//! * **綴りに空白・改行を含む特殊トークン。** 判定は囲みで前後の trim を避けているので
//!   **答えは正しい**が、**この機の7つの gguf には1件も無いので、実データでは一度も区別が付いていない**
//!   （植えた綴りでだけ確かめてある / `a_spelling_with_surrounding_blanks_still_needs_the_probe_wrapper`）。
//! * **gguf 以外の配布形式**（safetensors ＋ `tokenizer.json` / MLX）。
//! * **`chat_template` 側の注入**（`token_type` に現れない）。
//! * **語彙に無い綴りが、BPE の合成で結果的に制御トークン列になる経路。**
//! * **種別 2（UNKNOWN）が本当に特殊かどうか**（この機のどの gguf にも 1 件も無く、**未実測**）。
//! * **[`WATCHED_LIST_LIMIT`] を上げる向き。** 報告が長くなるだけなので見張っていない
//!   （**下げる向きは `the_watched_list_is_capped` が見る**）。
//! * **ディレクトリのシンボリックリンクの輪。** [`candidate_ggufs`] は辿り続ける。
//!   **Windows でディレクトリのリンクを作るには権限が要り、見張り番を置けなかった。**
//!
//! # ⚠ 規則を動かすのは、この器具の仕事ではない
//!
//! **覆えていない綴りが出たら、名前で報告するところまで。**
//! 規則（`is_shape_body` / `control_token_shape_len`）を広げるかどうかは人が決める
//! （B-22 の前例）。
//!
//! # 走らせ方
//!
//! ```text
//! cargo test --test control_token_survey -- --ignored --nocapture
//! ```
//!
//! **本体1本だけが `#[ignore]`。** 8GB の gguf を開くので普通のテストでは走らせない。
//! **モデルの在処は `PERSONACORE_GGUF_ROOT` で差し替えられる**（既定は
//! `%USERPROFILE%\.lmstudio\models`）。**LM Studio の起動は要らない。**
//!
//! # ⚠ 判定・走査・報告は、すべて `#[ignore]` の外から見張っている
//!
//! **器具の中身は観測対象の外**（B-25 / B-27 の実測: 器具の中の純粋な assert を
//! 骨抜きにしても全緑だった）。だから**この本体には主張を1つも書かない。**
//! 判定も報告の組み立ても関数にしてあり、下の見張り番が
//! **植えた入力と実データの両方を同じ判定に通す**（`15b` の自己成就対策）。

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use personacore::prompt::{
    MEASURED_CONTROL_TOKEN_SPELLINGS, MEASURED_MODEL_DEPENDENT_SPELLINGS,
    MEASURED_ORDINARY_LOOKALIKES, SafeText,
};

// ---------------------------------------------------------------------------
// 1. 判定（純粋関数）
// ---------------------------------------------------------------------------

/// 綴りの前に置く「普通の文字」。**開き括弧でも閉じ括弧でもない**ので、
/// **囲みの側から形が組み上がることは無い**（形が変わるなら、それは綴りの中の形）。
const PROBE_PREFIX: &str = "a";

/// 綴りの後ろに置く「普通の文字」。
///
/// **前後を挟むのは、`SafeText::sanitize` の行頭・行末の trim を避けるため。**
/// 裸で渡すと、空白を含む綴りが「崩された」ように見える
/// （`tests/common/calibration.rs` の `lookalike_side` が同じ前提を doc に書いている）。
const PROBE_SUFFIX: &str = "z";

/// **その綴りが、普通の文の中に書かれても原形のまま生き残るか。**
///
/// 生き残る ＝ **無害化が覆えていない**（利用者がそう書けば、上流へ原形で届く）。
///
/// # 空の綴りは「覆えていない」に数える
///
/// `"".contains` は常に真なので判定が成り立たない。
/// **「覆えている」と言い切るほうが危ないので、覆えていない側へ倒す**
/// （報告に出て人が見る。黙って安全側に数えない）。
fn survives_sanitize(spelling: &str) -> bool {
    if spelling.is_empty() {
        return true;
    }
    let probe = format!("{PROBE_PREFIX}{spelling}{PROBE_SUFFIX}");
    SafeText::sanitize(&probe).as_str().contains(spelling)
}

/// **綴りの一覧を受け取って、規則が覆えていないものを返す。**
///
/// **一覧は引数で受け取る** ——実データ（gguf から読んだ語彙）も、
/// テストが植えた綴りも、**同じ判定を通る**（`15b` の自己成就対策）。
fn uncovered_spellings<'a, I>(spellings: I) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    spellings
        .into_iter()
        .filter(|spelling| survives_sanitize(spelling))
        .collect()
}

/// 語彙の1件（id / 種別 / 綴り）。
type Entry<'a> = (usize, i32, &'a str);

/// **id と種別を保ったまま、覆えていないものだけを残す。**
fn uncovered_entries(entries: &[Entry<'_>]) -> Vec<(usize, i32, String)> {
    entries
        .iter()
        .filter(|(_, _, spelling)| survives_sanitize(spelling))
        .map(|(id, kind, spelling)| (*id, *kind, (*spelling).to_owned()))
        .collect()
}

// ---------------------------------------------------------------------------
// 2. gguf のヘッダを読む
// ---------------------------------------------------------------------------

/// gguf の先頭4バイト。
const GGUF_MAGIC: [u8; 4] = *b"GGUF";

/// 読める版。**v1 は長さが u32 なので別物**（この機の LM Studio は v3）。
const SUPPORTED_VERSIONS: [u32; 2] = [2, 3];

/// 綴りの配列を持つメタデータのキー。
const KEY_TOKENS: &str = "tokenizer.ggml.tokens";

/// 種別の配列を持つメタデータのキー。
const KEY_TOKEN_TYPE: &str = "tokenizer.ggml.token_type";

/// 普通の語彙の種別（判定も報告もしない側）。
const NORMAL_TOKEN_TYPE: i32 = 1;

/// 制御トークンの種別（llama.cpp の `CONTROL`）。
const CONTROL_TOKEN_TYPE: i32 = 3;

/// 利用者定義トークンの種別（llama.cpp の `USER_DEFINED`）。
const USER_DEFINED_TOKEN_TYPE: i32 = 4;

/// **1 トークンになると実測した種別**（このファイルの doc の表）。
///
/// **ここに無い種別は判定しない。** 増やすなら、**先に実機で 1 トークンだと測ること**
/// ——測らずに足すと、直す先の無い警報が出る（B-25）。
const JUDGED_TOKEN_TYPES: [i32; 2] = [CONTROL_TOKEN_TYPE, USER_DEFINED_TOKEN_TYPE];

/// 1つの文字列の長さの上限。**壊れたファイルで巨大な確保をしないための門。**
const MAX_STRING_BYTES: u64 = 1 << 20;

/// 配列の要素数の上限。同上。
const MAX_ARRAY_LEN: u64 = 1 << 24;

/// **1回の予約で、1バイトも読まないうちに確保してよい上限（バイト）。**
///
/// # ⚠ [`MAX_ARRAY_LEN`] は、この門の代わりにならない
///
/// あちらは**「件数として有り得るか」の門**であって、**「確保してよいか」の門ではない。**
/// `1 << 24` を通した直後に `Vec::<String>::with_capacity(length)` を呼ぶと、
/// **1要素も読む前に 16,777,216 × 24 ＝ 402,653,184 バイトを要求する**
/// （E-1 のレビュアーが実測 / 2026-09-03）。**細工したヘッダは 70 バイト弱で足りる** ——
/// **要素の中身は1バイトも要らない。**
///
/// # なぜ「まったく予約しない」にしないのか
///
/// **7つの gguf を読む器具**なので、予約を丸ごと捨てると再確保が増える。
/// **申告が天井の中なら、そのまま全部予約する。** 天井を超えた申告のときだけ頭を押さえ、
/// その先は `Vec` の増大に任せる（償却 O(1)）。
///
/// # ⚠ この値は「外すと遅くなるだけ」の側にある（ADR-0036）
///
/// 実測由来ではない。**小さくすれば再確保が増えるだけ、大きくすれば
/// 細工したヘッダで要求できる量が増えるだけ**で、
/// **読める語彙も、判定の答えも、1件も変わらない**
/// （見張り番は [`a_vocabulary_longer_than_the_initial_reservation_is_read_in_full`]）。
const MAX_INITIAL_RESERVE_BYTES: usize = 64 * 1024;

/// **申告された要素数から、最初に予約する件数を決める。純粋関数。**
///
/// 読む前に確かなのは**「1要素あたり何バイト確保するか」だけ**なので、
/// **確保するバイト数のほうに天井を置く。**
/// 申告が天井の中に収まっていれば、そのまま全部予約する。
///
/// **要素の大きさで答えが変わる** ——`String` は 24 バイト、`i32` は 4 バイトなので、
/// **同じ申告でも文字列の枝だけが 6 倍を確保する。**
/// 「同じ形が3つある」ではない（見張り番は
/// [`the_initial_reservation_is_capped_by_bytes_not_by_the_claimed_length`]）。
fn initial_capacity<T>(length: usize) -> usize {
    // ゼロサイズ型で割らないための 1（この器具では使わないが、式として安全側へ倒す）。
    let element_bytes = size_of::<T>().max(1);
    length.min(MAX_INITIAL_RESERVE_BYTES / element_bytes)
}

/// **入れ子の配列の深さの上限。**
///
/// `read_array` と `skip_value` は相互再帰なので、**入れ子の配列だけを並べた壊れたファイルで
/// スタックが飛ぶ**（B-35 のレビューが実測: 深さ 2000 ＝ **24 KB** でテストバイナリが即死）。
/// **1段あたり 12 バイト**なので、上限が無いと小さなファイルで落ちる。
/// 本物の gguf のメタデータは深さ 1（配列の中はスカラか文字列）。
const MAX_NESTING_DEPTH: u32 = 64;

/// gguf のメタデータから取り出した語彙。
#[derive(Debug, Default, PartialEq, Eq)]
struct Vocabulary {
    /// 綴り（id 順）。
    tokens: Vec<String>,
    /// 種別（id 順）。
    token_types: Vec<i32>,
    /// **UTF-8 として読めなかった綴りの件数**（置換文字にして数えた）。
    lossy: usize,
}

impl Vocabulary {
    /// **種別が `types` のどれかである綴り**を id つきで返す。
    fn tokens_with_type(&self, types: &[i32]) -> Vec<Entry<'_>> {
        self.entries()
            .filter(|(_, kind, _)| types.contains(kind))
            .collect()
    }

    /// **NORMAL でも判定対象でもない種別**の綴り（＝報告だけする側）。
    fn other_non_normal_tokens(&self) -> Vec<Entry<'_>> {
        self.entries()
            .filter(|(_, kind, _)| *kind != NORMAL_TOKEN_TYPE && !JUDGED_TOKEN_TYPES.contains(kind))
            .collect()
    }

    fn entries(&self) -> impl Iterator<Item = Entry<'_>> {
        self.tokens
            .iter()
            .zip(self.token_types.iter())
            .enumerate()
            .map(|(id, (spelling, kind))| (id, *kind, spelling.as_str()))
    }

    /// 種別ごとの件数（種別の昇順）。
    fn type_histogram(&self) -> Vec<(i32, usize)> {
        let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
        for kind in &self.token_types {
            *counts.entry(*kind).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }
}

/// **gguf のメタデータを頭から読み、語彙の2つの配列だけを取り出す。**
///
/// **引数は `Read`** ——実ファイルも、テストが植えたバイト列も同じ経路を通る。
fn read_vocabulary(reader: impl Read) -> Result<Vocabulary, String> {
    let mut reader = reader;

    let mut magic = [0_u8; 4];
    read_exact(&mut reader, &mut magic)?;
    if magic != GGUF_MAGIC {
        return Err(format!("gguf ではない（先頭4バイトが {magic:?}）"));
    }

    let version = read_u32(&mut reader)?;
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(format!("読めない gguf の版: {version}"));
    }

    let _tensor_count = read_u64(&mut reader)?;
    let kv_count = read_u64(&mut reader)?;

    let mut tokens: Option<(Vec<String>, usize)> = None;
    let mut token_types: Option<Vec<i32>> = None;

    for _ in 0..kv_count {
        let key = read_string(&mut reader)?;
        let value_type = read_u32(&mut reader)?;

        if key == KEY_TOKENS {
            match read_array(&mut reader, value_type, 0)? {
                ArrayValue::Strings(values, lossy) => tokens = Some((values, lossy)),
                other => return Err(format!("{KEY_TOKENS} が文字列の配列ではない: {other:?}")),
            }
        } else if key == KEY_TOKEN_TYPE {
            match read_array(&mut reader, value_type, 0)? {
                ArrayValue::Ints(values) => token_types = Some(values),
                other => return Err(format!("{KEY_TOKEN_TYPE} が整数の配列ではない: {other:?}")),
            }
        } else {
            skip_value(&mut reader, value_type, 0)?;
        }

        if tokens.is_some() && token_types.is_some() {
            break;
        }
    }

    let (tokens, lossy) = tokens.ok_or_else(|| format!("{KEY_TOKENS} が無い"))?;
    let token_types = token_types.ok_or_else(|| format!("{KEY_TOKEN_TYPE} が無い"))?;

    if tokens.len() != token_types.len() {
        return Err(format!(
            "綴りと種別の件数が食い違う: {} != {}",
            tokens.len(),
            token_types.len()
        ));
    }

    Ok(Vocabulary {
        tokens,
        token_types,
        lossy,
    })
}

/// 配列を読んだ結果。**要る2種類だけを持ち、それ以外は読み飛ばした印。**
#[derive(Debug)]
enum ArrayValue {
    /// 文字列の配列と、UTF-8 として読めなかった件数。
    Strings(Vec<String>, usize),
    /// 32bit 整数の配列。
    Ints(Vec<i32>),
    /// それ以外（読み飛ばした）。
    Skipped,
}

const TYPE_UINT32: u32 = 4;
const TYPE_INT32: u32 = 5;
const TYPE_STRING: u32 = 8;
const TYPE_ARRAY: u32 = 9;

/// 配列を読む。`value_type` が配列でなければ `Err`。
fn read_array(reader: &mut impl Read, value_type: u32, depth: u32) -> Result<ArrayValue, String> {
    if value_type != TYPE_ARRAY {
        return Err(format!("配列ではない値の型: {value_type}"));
    }
    // **入れ子の深さの門。** `read_array` と `skip_value` は相互再帰なので、
    // 入れ子の配列だけを並べた 24 KB のファイルでスタックが飛ぶ（B-35 のレビューが実測）。
    // **クラッシュは `passed=N failed=0` と出て、件数を読む道具が緑と読む**（CLAUDE.md）ので、
    // `MAX_STRING_BYTES` / `MAX_ARRAY_LEN` と同じく**名前つきの `Err`** で止める。
    if depth > MAX_NESTING_DEPTH {
        return Err(format!("配列の入れ子が深すぎる: {depth}"));
    }

    let element_type = read_u32(reader)?;
    let length = read_u64(reader)?;
    if length > MAX_ARRAY_LEN {
        return Err(format!("配列が長すぎる: {length}"));
    }
    let length = usize::try_from(length).map_err(|_| "配列の長さが usize に収まらない")?;

    match element_type {
        TYPE_STRING => {
            // **予約は申告そのままではなく、確保してよいバイト数で頭を押さえる**（E-3）。
            // **`String` は 1 要素 24 バイト**なので、押さえないとここが最も大きく振れる。
            let mut values = Vec::with_capacity(initial_capacity::<String>(length));
            let mut lossy = 0;
            for _ in 0..length {
                let bytes = read_string_bytes(reader)?;
                match String::from_utf8(bytes) {
                    Ok(text) => values.push(text),
                    Err(err) => {
                        lossy += 1;
                        values.push(String::from_utf8_lossy(err.as_bytes()).into_owned());
                    }
                }
            }
            Ok(ArrayValue::Strings(values, lossy))
        }
        // **INT32 は二の補数でそのまま戻す**（負の値はこの型では正当な復元）。
        TYPE_INT32 => {
            // **同上**（E-3）。**`i32` は 1 要素 4 バイト**なので、同じ申告でも文字列の枝の 1/6。
            let mut values = Vec::with_capacity(initial_capacity::<i32>(length));
            for _ in 0..length {
                values.push(read_u32(reader)? as i32);
            }
            Ok(ArrayValue::Ints(values))
        }
        // **UINT32 で符号が反転する値は、黙って通さない。**
        // `as i32` だと `0x8000_0000` が種別 `-2147483648` に化けて、
        // **判定の対象から静かに外れる**（`MAX_ARRAY_LEN` と同じ「門」の考え方）。
        TYPE_UINT32 => {
            // **同上**（E-3）。**入れ物は `Vec<i32>`** ——`u32` で読んでも確保するのは 4 バイト。
            let mut values = Vec::with_capacity(initial_capacity::<i32>(length));
            for _ in 0..length {
                let raw = read_u32(reader)?;
                values.push(
                    i32::try_from(raw).map_err(|_| format!("種別が i32 に収まらない: {raw}"))?,
                );
            }
            Ok(ArrayValue::Ints(values))
        }
        other => {
            for _ in 0..length {
                skip_value(reader, other, depth + 1)?;
            }
            Ok(ArrayValue::Skipped)
        }
    }
}

/// 値を1つ読み飛ばす。
fn skip_value(reader: &mut impl Read, value_type: u32, depth: u32) -> Result<(), String> {
    match value_type {
        // UINT8 / INT8 / BOOL
        0 | 1 | 7 => skip_bytes(reader, 1),
        // UINT16 / INT16
        2..=3 => skip_bytes(reader, 2),
        // UINT32 / INT32 / FLOAT32
        4..=6 => skip_bytes(reader, 4),
        // UINT64 / INT64 / FLOAT64
        10..=12 => skip_bytes(reader, 8),
        TYPE_STRING => {
            read_string_bytes(reader)?;
            Ok(())
        }
        TYPE_ARRAY => {
            read_array(reader, TYPE_ARRAY, depth + 1)?;
            Ok(())
        }
        other => Err(format!("読めない値の型: {other}")),
    }
}

fn skip_bytes(reader: &mut impl Read, count: u64) -> Result<(), String> {
    let mut sink = std::io::sink();
    let copied = std::io::copy(&mut reader.take(count), &mut sink)
        .map_err(|err| format!("読み飛ばしに失敗: {err}"))?;
    if copied != count {
        return Err(format!("途中で終わった（{copied} / {count} バイト）"));
    }
    Ok(())
}

fn read_exact(reader: &mut impl Read, buffer: &mut [u8]) -> Result<(), String> {
    reader
        .read_exact(buffer)
        .map_err(|err| format!("読み取りに失敗: {err}"))
}

fn read_u32(reader: &mut impl Read) -> Result<u32, String> {
    let mut buffer = [0_u8; 4];
    read_exact(reader, &mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64(reader: &mut impl Read) -> Result<u64, String> {
    let mut buffer = [0_u8; 8];
    read_exact(reader, &mut buffer)?;
    Ok(u64::from_le_bytes(buffer))
}

/// **申告された長さぶんを、実際に読めた量に合わせて少しずつ確保する。**
///
/// # ⚠ 以前は `vec![0_u8; length]` だった（E-3 のレビューによる是正）
///
/// **門（[`MAX_STRING_BYTES`]）を通った長さを、1バイトも読まないうちに確保して 0 で埋めていた** ——
/// **8 バイトの申告で 1 MiB。** 記憶の観点では門の中（上限は門の値そのもの）だが、
/// **押さえが無いせいで「門の値を上げる変異」が名前つきの赤ではなく abort になっていた**
/// （`memory allocation of … failed` ＝ `passed=N failed=0` / exit 101。
/// **配列の枝では同じ変異がクリーンな赤になる**）。
/// **押さえの有無が、そのまま「落ち方の質」の差になっていた。**
///
/// **`16e-b` の「同じ危険に入口が2つあるとき、片方だけ塞ぐと反対側から入られる」の対称の形にする** ——
/// **配列の枝とまったく同じ [`initial_capacity`] を通し、伸ばすのは実際に読めた後だけ。**
///
/// # ⚠ エラーの文面は変えていない
///
/// 途中で尽きたときは、いままでどおり `read_exact` の `読み取りに失敗` で落ちる。
/// **文面を変えると、それを見ている既存の見張り番の主張が黙って変わる**（`B-6`）。
fn read_string_bytes(reader: &mut impl Read) -> Result<Vec<u8>, String> {
    let claimed = read_u64(reader)?;
    if claimed > MAX_STRING_BYTES {
        return Err(format!("文字列が長すぎる: {claimed}"));
    }
    let length = usize::try_from(claimed).map_err(|_| "文字列の長さが usize に収まらない")?;

    // **最初の一歩は天井まで。** 実トークンはここで収まるので、
    // **`vec![0_u8; length]` だった頃と1命令も変わらない**（下の `while` に入らない）。
    let mut buffer = vec![0_u8; initial_capacity::<u8>(length)];
    read_exact(reader, &mut buffer)?;

    // **天井を超える申告は、読めて初めて次の一歩へ進む。**
    while buffer.len() < length {
        let filled = buffer.len();
        let next = length.min(filled + MAX_INITIAL_RESERVE_BYTES);
        buffer.resize(next, 0);
        read_exact(reader, &mut buffer[filled..next])?;
    }
    Ok(buffer)
}

fn read_string(reader: &mut impl Read) -> Result<String, String> {
    let bytes = read_string_bytes(reader)?;
    String::from_utf8(bytes).map_err(|err| format!("キーが UTF-8 でない: {err}"))
}

// ---------------------------------------------------------------------------
// 3. 報告
// ---------------------------------------------------------------------------

/// 報告に並べる「判定していない側」の上限。**多すぎると塊が読めなくなる。**
const WATCHED_LIST_LIMIT: usize = 20;

/// 1つの gguf を調べた結果。
#[derive(Debug)]
struct Survey {
    /// 調べた gguf の名前（**パスは出さない** ——報告に内部パスを載せない）。
    name: String,
    /// 語彙の総数。
    total: usize,
    /// 種別ごとの件数。
    histogram: Vec<(i32, usize)>,
    /// CONTROL の件数。
    control_count: usize,
    /// **判定した件数**（[`JUDGED_TOKEN_TYPES`] の合計）。
    judged_count: usize,
    /// **判定した種別のうち、無害化を素通りする綴り**（id / 種別 / 綴り）。
    uncovered: Vec<(usize, i32, String)>,
    /// **判定していない種別のうち、無害化を素通りする綴り。**
    ///
    /// **落としても失敗にしない** ——その種別が 1 トークンになるとは実測していない
    /// （測っていないもので落とすと、直す先の無い警報になる / B-25）。
    watched: Vec<(usize, i32, String)>,
    /// UTF-8 として読めなかった綴りの件数。
    lossy: usize,
}

/// 語彙を調べる。**純粋関数**（ファイルを開かない）。
fn survey(name: &str, vocabulary: &Vocabulary) -> Survey {
    let judged = vocabulary.tokens_with_type(&JUDGED_TOKEN_TYPES);
    let others = vocabulary.other_non_normal_tokens();

    Survey {
        name: name.to_owned(),
        total: vocabulary.tokens.len(),
        histogram: vocabulary.type_histogram(),
        control_count: vocabulary.tokens_with_type(&[CONTROL_TOKEN_TYPE]).len(),
        judged_count: judged.len(),
        uncovered: uncovered_entries(&judged),
        watched: uncovered_entries(&others),
        lossy: vocabulary.lossy,
    }
}

/// **人が読む塊。** 1回の `println!` で出す（並行実行で割れないように）。
///
/// # ⚠ 報告を出すこと自体が、この関数の主張である
///
/// **器具の中に `println!` を並べると、落とす変異が誰にも見えない**
/// （B-25 で実測: 1749 全緑）。**だから塊の組み立てをここに出して、
/// `the_block_names_every_uncovered_spelling` が見る。**
fn survey_block(survey: &Survey) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!("--- {} ---", survey.name));
    lines.push(format!("語彙 {} 件", survey.total));
    lines.push(format!(
        "種別ごとの件数: {}",
        survey
            .histogram
            .iter()
            .map(|(kind, count)| format!("{kind}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    ));
    lines.push(format!(
        "CONTROL（token_type={CONTROL_TOKEN_TYPE}）= {} 件 / 判定した件数（種別 {}）= {} 件",
        survey.control_count,
        JUDGED_TOKEN_TYPES
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join("・"),
        survey.judged_count
    ));

    if survey.lossy > 0 {
        lines.push(format!(
            "⚠ UTF-8 として読めなかった綴りが {} 件ある（置換して数えた）",
            survey.lossy
        ));
    }

    if survey.uncovered.is_empty() {
        lines
            .push("覆えている: 判定した綴りは、普通の文に書いても 1 件も原形で残らない".to_owned());
    } else {
        lines.push(format!(
            "⚠ 覆えていない綴りが {} 件（普通の文に書くと原形のまま上流へ届く）:",
            survey.uncovered.len()
        ));
        for (id, kind, spelling) in &survey.uncovered {
            lines.push(format!("  - id {id}（種別 {kind}）: {spelling:?}"));
        }
    }

    if survey.watched.is_empty() {
        lines.push("判定していない種別に、原形で残る綴りは無い".to_owned());
    } else {
        lines.push(format!(
            "⚠ 判定していない種別に、原形で残る綴りが {} 件ある\
             （1 トークンになると実測していないので落とさない。人が見ること）:",
            survey.watched.len()
        ));
        for (id, kind, spelling) in survey.watched.iter().take(WATCHED_LIST_LIMIT) {
            lines.push(format!("  - id {id}（種別 {kind}）: {spelling:?}"));
        }
        if survey.watched.len() > WATCHED_LIST_LIMIT {
            lines.push(format!(
                "  … ほか {} 件",
                survey.watched.len() - WATCHED_LIST_LIMIT
            ));
        }
    }

    lines.join("\n")
}

/// **失敗の文面**（判定した種別に覆えていない綴りがあるときだけ）。
///
/// **直す先を名乗る。ただし「規則を広げろ」とは言い切らない** ——
/// 広げるかどうかは人の判断（B-22 の前例）で、この器具の仕事ではない。
fn survey_failure(survey: &Survey) -> Option<String> {
    if survey.uncovered.is_empty() {
        return None;
    }

    let names = survey
        .uncovered
        .iter()
        .map(|(id, kind, spelling)| format!("{spelling:?}(id {id} / 種別 {kind})"))
        .collect::<Vec<_>>()
        .join(" ");

    Some(format!(
        "{}: 無害化が覆えていない綴りが {} 件ある: {names}。\
         このモデルを上流にすると、記憶や発言にこの綴りを書くだけで注入できる。\
         規則（src/prompt/safe.rs の is_shape_body / control_token_shape_len）を広げるかどうかは\
         人が決めること（B-22 の前例 / docs/04-environment.md 手順2）",
        survey.name,
        survey.uncovered.len()
    ))
}

/// **読み終わった全部の gguf から、報告の塊と失敗の一覧を組む。純粋関数。**
///
/// # なぜ器具の外にあるのか
///
/// **器具は `#[ignore]`** なので、**中に書いた組み立ては素の `cargo test` では1行も走らない。**
/// **器具に残るのは「ファイルを開いて `println!` して `assert!` する」だけ**にして、
/// **読めなかったものを黙って捨てていないか**まで、ここで見張れるようにしてある。
fn survey_all(read: &[(String, Result<Vocabulary, String>)]) -> (String, Vec<String>) {
    let mut blocks: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for (name, outcome) in read {
        match outcome {
            Ok(vocabulary) => {
                let result = survey(name, vocabulary);
                blocks.push(survey_block(&result));
                if let Some(failure) = survey_failure(&result) {
                    failures.push(failure);
                }
            }
            // **読めなかったものは黙って捨てない。** 名前と理由を塊に出す。
            Err(error) => blocks.push(format!("--- {name} ---\n⚠ 読めなかった: {error}")),
        }
    }

    (blocks.join("\n"), failures)
}

// ---------------------------------------------------------------------------
// 4. 在処
// ---------------------------------------------------------------------------

/// gguf を探す根を差し替える環境変数。
const ENV_GGUF_ROOT: &str = "PERSONACORE_GGUF_ROOT";

/// **どこを探すか。純粋関数**（参照関数を受け取る / `src/server/config.rs` と同じ作法）。
fn gguf_search_root<F>(lookup: F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    let named = |key: &str| lookup(key).filter(|value| !value.trim().is_empty());

    if let Some(root) = named(ENV_GGUF_ROOT) {
        return Some(PathBuf::from(root));
    }

    let home = named("USERPROFILE").or_else(|| named("HOME"))?;
    Some(Path::new(&home).join(".lmstudio").join("models"))
}

/// **調べる対象のファイルか。**
///
/// `mmproj-*.gguf` は画像側の射影で**トークナイザを持たない**ので外す
/// （入れると「キーが無い」で毎回落ちる）。
fn is_candidate_gguf(file_name: &str) -> bool {
    file_name.ends_with(".gguf") && !file_name.starts_with("mmproj-")
}

/// 根の下の gguf を集める。**I/O はここだけ。**
fn candidate_ggufs(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_candidate_gguf)
            {
                found.push(path);
            }
        }
    }

    found.sort();
    found
}

// ---------------------------------------------------------------------------
// 5. 見張り番（**`#[ignore]` 無し**）
// ---------------------------------------------------------------------------

/// 植えた gguf を組む。
///
/// # ⚠ キーの綴りは**この関数の中のリテラル**である
///
/// 走査側の定数（[`KEY_TOKENS`] / [`KEY_TOKEN_TYPE`]）を参照しないこと。
/// **参照すると、綴りを壊す変異が両側を同時に動かして緑のまま通る**
/// （`15b` の「対照が自己成就する」形そのもの）。
fn planted_gguf(entries: &[(&str, i32)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&GGUF_MAGIC);
    out.extend_from_slice(&3_u32.to_le_bytes()); // 版
    out.extend_from_slice(&0_u64.to_le_bytes()); // テンソル数
    out.extend_from_slice(&4_u64.to_le_bytes()); // メタデータの件数

    // 1. 読み飛ばされる文字列。
    push_gguf_string(&mut out, "general.architecture");
    out.extend_from_slice(&8_u32.to_le_bytes());
    push_gguf_string(&mut out, "llama");

    // 2. 種別の配列を**先に**置く（順序に寄りかかっていないこと）。
    push_gguf_string(&mut out, "tokenizer.ggml.token_type");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&5_u32.to_le_bytes()); // INT32
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (_, kind) in entries {
        out.extend_from_slice(&kind.to_le_bytes());
    }

    // 3. 読み飛ばされる配列（間に挟む）。
    push_gguf_string(&mut out, "tokenizer.ggml.merges");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&8_u32.to_le_bytes()); // STRING
    out.extend_from_slice(&2_u64.to_le_bytes());
    push_gguf_string(&mut out, "a b");
    push_gguf_string(&mut out, "c d");

    // 4. 綴りの配列。
    push_gguf_string(&mut out, "tokenizer.ggml.tokens");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&8_u32.to_le_bytes()); // STRING
    out.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for (spelling, _) in entries {
        push_gguf_string(&mut out, spelling);
    }

    out
}

fn push_gguf_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u64).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// **植えた語彙から、判定する種別（3 と 4）の綴りだけが出てくる。**
///
/// **囮は「特殊に見えるのに判定の対象ではない」綴り** ——
/// `[INST]` を NORMAL に、`<s>` を UNKNOWN に、`[PAD1]` を UNUSED に置いた。
/// **種別を見ずに綴りで拾う実装なら、ここで赤くなる。**
#[test]
fn only_the_judged_token_types_come_out() {
    let vocabulary = read_vocabulary(
        planted_gguf(&[
            ("<pad>", 3),
            ("[INST]", 1),
            ("ねこ", 1),
            ("<|im_start|>", 3),
            ("<s>", 2),
            ("<tool_call>", 4),
            ("[PAD1]", 5),
            ("<0x0A>", 6),
        ])
        .as_slice(),
    )
    .expect("植えた gguf が読めること");

    assert_eq!(
        vocabulary.tokens_with_type(&JUDGED_TOKEN_TYPES),
        vec![
            (0, 3, "<pad>"),
            (3, 3, "<|im_start|>"),
            (5, 4, "<tool_call>"),
        ]
    );
    assert_eq!(
        vocabulary.tokens_with_type(&[CONTROL_TOKEN_TYPE]),
        vec![(0, 3, "<pad>"), (3, 3, "<|im_start|>")]
    );
    // **NORMAL でも判定対象でもない種別**だけが、報告だけの側へ回る。
    assert_eq!(
        vocabulary.other_non_normal_tokens(),
        vec![(4, 2, "<s>"), (6, 5, "[PAD1]"), (7, 6, "<0x0A>")]
    );
}

/// **種別ごとの件数を数える。** 判定しない種別も出す（＝拾う側の取り違えが読める）。
#[test]
fn the_histogram_counts_every_type() {
    let vocabulary = read_vocabulary(
        planted_gguf(&[("a", 1), ("b", 1), ("c", 3), ("d", 6), ("e", 3), ("f", 4)]).as_slice(),
    )
    .expect("植えた gguf が読めること");

    assert_eq!(
        vocabulary.type_histogram(),
        vec![(1, 2), (3, 2), (4, 1), (6, 1)]
    );
}

/// **綴りと種別の件数が食い違ったら止まる。**
#[test]
fn a_mismatched_vocabulary_is_an_error() {
    let mut bytes = planted_gguf(&[("a", 3), ("b", 3)]);
    // 綴りの配列の要素数だけを 1 にする（key の後ろは value_type(4) + element_type(4)）。
    let key = "tokenizer.ggml.tokens";
    let length_at = find_subslice(&bytes, key.as_bytes()).expect("キーがあること") + key.len() + 8;
    bytes[length_at..length_at + 8].copy_from_slice(&1_u64.to_le_bytes());

    let error = read_vocabulary(bytes.as_slice()).expect_err("食い違いは Err になること");
    assert!(error.contains("食い違う"), "{error}");
}

/// **gguf でないものは読まない。**
#[test]
fn a_file_that_is_not_gguf_is_an_error() {
    let error = read_vocabulary(b"NOPE....".as_slice()).expect_err("Err になること");
    assert!(error.contains("gguf ではない"), "{error}");
}

/// **読めない版は止まる。**
#[test]
fn an_unsupported_gguf_version_is_an_error() {
    let mut bytes = planted_gguf(&[("a", 3)]);
    bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());

    let error = read_vocabulary(bytes.as_slice()).expect_err("Err になること");
    assert!(error.contains("読めない gguf の版"), "{error}");
}

/// **綴りの配列が無ければ止まる**（黙って 0 件にしない）。
#[test]
fn a_vocabulary_without_the_token_list_is_an_error() {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(&1_u64.to_le_bytes());
    push_gguf_string(&mut out, "tokenizer.ggml.token_type");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&5_u32.to_le_bytes());
    out.extend_from_slice(&1_u64.to_le_bytes());
    out.extend_from_slice(&3_i32.to_le_bytes());

    let error = read_vocabulary(out.as_slice()).expect_err("Err になること");
    assert!(error.contains("tokenizer.ggml.tokens"), "{error}");
}

/// **規則が覆っている綴りは、1件も返らない**（判定が「常に全部返す」なら赤）。
///
/// **植えた綴りは、実データの一覧とは別に書いてある** ——
/// 一覧を参照すると、一覧が空になる変異でこのテストが自己成就する。
#[test]
fn spellings_the_rule_breaks_are_not_reported() {
    let covered = [
        "[INST]",
        "[/SYSTEM_PROMPT]",
        "<s>",
        "</s>",
        "<pad>",
        "<SPECIAL_999>",
        "<|im_start|>",
        "<|endoftext|>",
        "<tool_call>",
        "</think>",
        "[PAD248077]",
        "[]",
    ];

    assert_eq!(uncovered_spellings(covered), Vec::<&str>::new());
}

/// **規則の外側の形は、名前で返る**（判定が「常に空を返す」なら赤）。
///
/// 選んだ綴りは**別系列のモデルが現実に持つ形**:
/// `▁`（U+2581）や全角の `｜`（U+FF5C）は ASCII ではないので形の本体に入らず、
/// `>>ABSTRACT<<` は括弧が `[` / `<` でない。
#[test]
fn spellings_outside_the_rule_are_reported_by_name() {
    let outside = [
        "<|begin▁of▁sentence|>",
        "<｜im_start｜>",
        ">>ABSTRACT<<",
        "▁▁▁",
        "<|im start|>",
    ];

    assert_eq!(uncovered_spellings(outside), outside.to_vec());
}

/// **同じ判定に、実データを通す**（植えた綴りだけで測らない / `15b`）。
///
/// `src/prompt/safe.rs` の2つの一覧は**どちらも規則が崩す形**である
/// （`MEASURED_ORDINARY_LOOKALIKES` は**外側の針を含む**ので、こちらは別のテスト）。
#[test]
fn every_measured_control_spelling_is_covered_by_the_same_judgement() {
    assert_eq!(
        uncovered_spellings(MEASURED_CONTROL_TOKEN_SPELLINGS),
        Vec::<&str>::new()
    );
    assert_eq!(
        uncovered_spellings(MEASURED_MODEL_DEPENDENT_SPELLINGS),
        Vec::<&str>::new()
    );
}

/// **対照の一覧の「外側の針」は、この判定でも覆えていない側に出る。**
///
/// **`MEASURED_ORDINARY_LOOKALIKES` は2種類の針でできている**
/// （`src/prompt/safe.rs` のその定数の doc）。**この判定が、その2種類を
/// 実際に分けられることを確かめる** ——分けられなければ、
/// 器具は「規則の外」か「規則の内」のどちらかを一度も通らない。
#[test]
fn the_judgement_separates_both_kinds_of_lookalike() {
    let outside = uncovered_spellings(MEASURED_ORDINARY_LOOKALIKES);

    assert!(
        !outside.is_empty(),
        "対照の一覧に規則の外側の針が1本も無い: {MEASURED_ORDINARY_LOOKALIKES:?}"
    );
    assert!(
        outside.len() < MEASURED_ORDINARY_LOOKALIKES.len(),
        "対照の一覧に規則の内側の針が1本も無い: {MEASURED_ORDINARY_LOOKALIKES:?}"
    );
}

/// **空の綴りは「覆えている」に数えない。**
#[test]
fn an_empty_spelling_is_reported_rather_than_assumed_safe() {
    assert_eq!(uncovered_spellings([""]), vec![""]);
}

/// **報告の塊が、覆えていない綴りを1件残らず名指しする。**
#[test]
fn the_block_names_every_uncovered_spelling() {
    let vocabulary = read_vocabulary(
        planted_gguf(&[
            ("<s>", 3),
            ("<|begin▁of▁sentence|>", 3),
            ("ねこ", 1),
            (">>ABSTRACT<<", 4),
        ])
        .as_slice(),
    )
    .expect("植えた gguf が読めること");
    let result = survey("planted.gguf", &vocabulary);

    assert_eq!(result.control_count, 2);
    assert_eq!(result.judged_count, 3);
    assert_eq!(
        result.uncovered,
        vec![
            (1, 3, "<|begin▁of▁sentence|>".to_owned()),
            (3, 4, ">>ABSTRACT<<".to_owned()),
        ]
    );

    let block = survey_block(&result);
    assert!(block.contains("planted.gguf"), "{block}");
    assert!(block.contains("語彙 4 件"), "{block}");
    assert!(
        block.contains("CONTROL（token_type=3）= 2 件 / 判定した件数（種別 3・4）= 3 件"),
        "{block}"
    );
    assert!(block.contains("覆えていない綴りが 2 件"), "{block}");
    assert!(
        block.contains("id 1（種別 3）: \"<|begin▁of▁sentence|>\""),
        "{block}"
    );
    assert!(
        block.contains("id 3（種別 4）: \">>ABSTRACT<<\""),
        "{block}"
    );
    // **覆えている綴りは、覆えていない側に混ざらない。**
    assert!(!block.contains("id 0"), "{block}");
}

/// **判定していない種別で原形が残っても、失敗にはしない。報告には出す。**
///
/// **1 トークンになると実測していない種別で落とすと、直す先の無い警報になる**（B-25）。
#[test]
fn an_unjudged_type_is_reported_but_does_not_fail() {
    let vocabulary =
        read_vocabulary(planted_gguf(&[("[INST]", 3), (">>ABSTRACT<<", 5), ("▁▁▁", 2)]).as_slice())
            .expect("植えた gguf が読めること");
    let result = survey("watched.gguf", &vocabulary);

    assert!(result.uncovered.is_empty(), "{:?}", result.uncovered);
    assert_eq!(
        result.watched,
        vec![(1, 5, ">>ABSTRACT<<".to_owned()), (2, 2, "▁▁▁".to_owned()),]
    );

    let block = survey_block(&result);
    assert!(
        block.contains("判定していない種別に、原形で残る綴りが 2 件ある"),
        "{block}"
    );
    assert!(
        block.contains("id 1（種別 5）: \">>ABSTRACT<<\""),
        "{block}"
    );
    assert!(block.contains("id 2（種別 2）: \"▁▁▁\""), "{block}");
    assert_eq!(survey_failure(&result), None);
}

/// **穴が無いときの塊は、穴があるときと違う文面になる。**
#[test]
fn a_survey_without_a_gap_reads_differently() {
    let vocabulary =
        read_vocabulary(planted_gguf(&[("<s>", 3), ("[INST]", 4)]).as_slice()).expect("読めること");
    let result = survey("clean.gguf", &vocabulary);

    let block = survey_block(&result);
    assert!(block.contains("覆えている:"), "{block}");
    assert!(!block.contains("覆えていない綴りが"), "{block}");
    assert!(
        block.contains("判定していない種別に、原形で残る綴りは無い"),
        "{block}"
    );
    assert_eq!(survey_failure(&result), None);
}

/// **失敗の文面は、綴りと直す先の候補を名乗る。**
#[test]
fn the_failure_names_the_spellings_and_where_to_look() {
    let vocabulary =
        read_vocabulary(planted_gguf(&[(">>ABSTRACT<<", 3)]).as_slice()).expect("読めること");
    let result = survey("gap.gguf", &vocabulary);

    let failure = survey_failure(&result).expect("穴があれば文面が出ること");
    assert!(failure.contains("gap.gguf"), "{failure}");
    assert!(failure.contains(">>ABSTRACT<<"), "{failure}");
    assert!(failure.contains("src/prompt/safe.rs"), "{failure}");
    assert!(failure.contains("人が決める"), "{failure}");
}

/// **UTF-8 として読めなかった綴りの件数は、黙って捨てない。**
#[test]
fn tokens_that_are_not_utf8_are_counted_in_the_block() {
    // 綴りの配列に不正なバイト列を1件入れる。
    let mut bytes = planted_gguf(&[("ok", 3)]);
    let position =
        find_subslice(&bytes, b"\x02\x00\x00\x00\x00\x00\x00\x00ok").expect("綴りがあること");
    bytes[position + 8] = 0xFF;
    bytes[position + 9] = 0xFE;

    let vocabulary = read_vocabulary(bytes.as_slice()).expect("読めること");
    assert_eq!(vocabulary.lossy, 1);

    let block = survey_block(&survey("lossy.gguf", &vocabulary));
    assert!(
        block.contains("UTF-8 として読めなかった綴りが 1 件"),
        "{block}"
    );
}

/// **全部ぶんの塊が、読めたものと読めなかったものを両方名乗る。**
///
/// **器具に残っているのは `println!` と `assert!` だけ**なので、
/// **「何を出すか」と「何で落ちるか」はここが権威。**
#[test]
fn the_whole_report_names_every_file_including_the_unreadable_one() {
    let clean = read_vocabulary(planted_gguf(&[("[INST]", 3)]).as_slice()).expect("読めること");
    let gap = read_vocabulary(planted_gguf(&[(">>ABSTRACT<<", 4)]).as_slice()).expect("読めること");

    let (block, failures) = survey_all(&[
        ("clean.gguf".to_owned(), Ok(clean)),
        ("broken.gguf".to_owned(), Err("gguf ではない".to_owned())),
        ("gap.gguf".to_owned(), Ok(gap)),
    ]);

    // 3件とも塊に出る。**読めなかったものを黙って捨てない。**
    assert!(block.contains("--- clean.gguf ---"), "{block}");
    assert!(block.contains("--- broken.gguf ---"), "{block}");
    assert!(block.contains("⚠ 読めなかった: gguf ではない"), "{block}");
    assert!(block.contains("--- gap.gguf ---"), "{block}");
    assert!(block.contains(">>ABSTRACT<<"), "{block}");

    // **落ちるのは穴のあった1件だけ**（読めなかった件では落ちない）。
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].starts_with("gap.gguf:"), "{failures:?}");
}

/// **探す根は、環境変数が最優先。**
#[test]
fn the_search_root_prefers_the_environment_variable() {
    let root = gguf_search_root(|key| match key {
        "PERSONACORE_GGUF_ROOT" => Some("D:\\models".to_owned()),
        "USERPROFILE" => Some("C:\\Users\\someone".to_owned()),
        _ => None,
    });

    assert_eq!(root, Some(PathBuf::from("D:\\models")));
}

/// **未設定なら LM Studio の既定の在処へ落ちる。**
#[test]
fn the_search_root_falls_back_to_the_lmstudio_directory() {
    let root = gguf_search_root(|key| match key {
        "USERPROFILE" => Some("C:\\Users\\someone".to_owned()),
        _ => None,
    });

    assert_eq!(
        root,
        Some(
            Path::new("C:\\Users\\someone")
                .join(".lmstudio")
                .join("models")
        )
    );
}

/// **空白だけの指定は「指定していない」と同じ**（`PERSONACORE_SUB_MODEL` と同じ作法）。
#[test]
fn a_blank_search_root_falls_through() {
    let root = gguf_search_root(|key| match key {
        "PERSONACORE_GGUF_ROOT" => Some("   ".to_owned()),
        "HOME" => Some("/home/someone".to_owned()),
        _ => None,
    });

    assert_eq!(
        root,
        Some(Path::new("/home/someone").join(".lmstudio").join("models"))
    );
}

/// **何も無ければ根は決まらない。**
#[test]
fn without_any_hint_there_is_no_search_root() {
    assert_eq!(gguf_search_root(|_| None), None);
}

/// **画像側の射影は調べない**（トークナイザを持たない）。
#[test]
fn the_projection_files_are_not_surveyed() {
    assert!(is_candidate_gguf("Qwen3.5-9B-Q4_K_M.gguf"));
    assert!(!is_candidate_gguf("mmproj-Qwen3.5-9B-BF16.gguf"));
    assert!(!is_candidate_gguf("Qwen3.5-9B-Q4_K_M.gguf.tmp"));
    assert!(!is_candidate_gguf("README.md"));
}

// ---------------------------------------------------------------------------
// 6. 器具（**`#[ignore]`。主張を1つも書かないこと**）
// ---------------------------------------------------------------------------

/// **この機に入っている gguf を全部調べる**（docs/04-environment.md 手順2 の 1〜4）。
///
/// ```text
/// cargo test --test control_token_survey -- --ignored --nocapture
/// ```
#[test]
#[ignore = "この機の gguf を読む。手順はこのファイルの doc を参照"]
fn every_installed_gguf_is_covered_by_the_neutralizing_rule() {
    let root = gguf_search_root(|key| std::env::var(key).ok())
        .unwrap_or_else(|| panic!("gguf の在処が決まらない（{ENV_GGUF_ROOT} を指すこと）"));

    let files = candidate_ggufs(&root);
    assert!(!files.is_empty(), "gguf が1件も見つからない: {root:?}");

    // **読むだけ。** 組み立ても判定も `survey_all` が持つ
    // （器具の中に置くと、素の `cargo test` では1行も走らない / B-25）。
    let mut read: Vec<(String, Result<Vocabulary, String>)> = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("(名前が読めない)")
            .to_owned();

        let file = File::open(&path).unwrap_or_else(|err| panic!("{name} を開けない: {err}"));
        read.push((
            name,
            read_vocabulary(BufReader::with_capacity(1 << 20, file)),
        ));
    }

    let (block, failures) = survey_all(&read);
    println!("{block}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------
// 7. B-35 のレビューが「緑を通り抜けた」と名指しした穴の見張り番
// ---------------------------------------------------------------------------

/// **囲みが無ければ答えが変わる綴りを1本通す**（要修正3 / レビュアーの実測をそのまま使う）。
///
/// # なぜ囲みが要るのか（doc の条件を、テストが作る）
///
/// `SafeText::sanitize` は**行の前後の空白を落とす**ので、
/// **前後に空白を持つ綴りを裸で渡すと、規則が何もしていなくても綴りが消える。**
/// **消えると判定は「覆えている」と答える** ——**安全側の嘘**である。
///
/// レビュアーの実測（2026-09-02）:
///
/// ```text
/// 綴り " >>ABSTRACT<<" → 囲みあり=覆えていない（正しい） / 裸=覆えている（嘘）
/// ```
///
/// **`PROBE_PREFIX` / `PROBE_SUFFIX` を外す変異は、この1本でだけ赤くなる。**
#[test]
fn a_spelling_with_surrounding_blanks_still_needs_the_probe_wrapper() {
    // 前後に空白を持ち、**かつ規則が崩さない形**（括弧が `[` / `<` でない）。
    let spelling = " >>ABSTRACT<<";

    assert_eq!(
        uncovered_spellings([spelling]),
        vec![spelling],
        "前後に空白のある綴りが「覆えている」に落ちた（囲みが効いていない）"
    );

    // **なぜ嘘になるのか**を、同じ入力で見せる（囲みを外した場合の値）。
    assert!(
        !SafeText::sanitize(spelling).as_str().contains(spelling),
        "裸で渡すと綴りが消える、という前提そのものが変わった"
    );
}

/// **入れ子の配列は、落ちるのではなく `Err` になる**（要修正4 / レビュアーの実測）。
///
/// **クラッシュは「失敗したテスト」に数えられない**（`passed=N failed=0` と出る / CLAUDE.md）ので、
/// **敵対的入力は全部名前つきの `Err` へ寄せる。**
#[test]
fn a_deeply_nested_array_is_an_error_rather_than_a_crash() {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(&1_u64.to_le_bytes());
    push_gguf_string(&mut out, "deep");
    out.extend_from_slice(&9_u32.to_le_bytes()); // 値そのものが配列
    // **1段 12 バイト**（要素の型 4 ＋ 要素数 8）。レビュアーの再現と同じ深さまで積む。
    for _ in 0..2_000 {
        out.extend_from_slice(&9_u32.to_le_bytes()); // 要素も配列
        out.extend_from_slice(&1_u64.to_le_bytes()); // 要素1件
    }

    let error = read_vocabulary(out.as_slice()).expect_err("Err になること");
    assert!(error.contains("入れ子が深すぎる"), "{error}");
}

/// **本物の深さ（1段）は通る**（門が深すぎるほうへ効きすぎていないことの対照）。
#[test]
fn an_ordinary_one_level_array_still_reads() {
    let vocabulary =
        read_vocabulary(planted_gguf(&[("<s>", 3)]).as_slice()).expect("普通の gguf は読めること");

    assert_eq!(vocabulary.tokens, vec!["<s>".to_owned()]);
}

/// **UINT32 の種別で符号が反転する値は `Err`**（`as i32` の穴）。
///
/// **`0x8000_0000` を `as i32` で読むと種別 `-2147483648` になり、
/// どの一覧にも入らないまま「判定の対象ではない」側へ静かに落ちる。**
#[test]
fn a_token_type_that_does_not_fit_in_i32_is_an_error() {
    let readable = read_vocabulary(planted_gguf_with_uint32_types(&[3, 4]).as_slice())
        .expect("普通の UINT32 は読めること");
    assert_eq!(
        readable.tokens_with_type(&JUDGED_TOKEN_TYPES),
        vec![(0, 3, "<t0>"), (1, 4, "<t1>")]
    );

    let error = read_vocabulary(planted_gguf_with_uint32_types(&[3, 0x8000_0000]).as_slice())
        .expect_err("符号が反転する値は Err になること");
    assert!(error.contains("i32 に収まらない"), "{error}");
}

/// 種別の配列を **UINT32** で書いた植えた gguf。
///
/// **キーの綴りは `planted_gguf` と同じくこの関数の中のリテラル**（走査側の定数を参照しない）。
fn planted_gguf_with_uint32_types(types: &[u32]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(&2_u64.to_le_bytes());

    push_gguf_string(&mut out, "tokenizer.ggml.token_type");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&4_u32.to_le_bytes()); // UINT32
    out.extend_from_slice(&(types.len() as u64).to_le_bytes());
    for kind in types {
        out.extend_from_slice(&kind.to_le_bytes());
    }

    push_gguf_string(&mut out, "tokenizer.ggml.tokens");
    out.extend_from_slice(&9_u32.to_le_bytes());
    out.extend_from_slice(&8_u32.to_le_bytes()); // STRING
    out.extend_from_slice(&(types.len() as u64).to_le_bytes());
    for index in 0..types.len() {
        push_gguf_string(&mut out, &format!("<t{index}>"));
    }

    out
}

/// **報告だけの一覧が長すぎるときは、上限で切って「ほか N 件」と言う。**
///
/// **見ているのは下げる向きだけ**（切ったのに件数を言わない／切らずに全部出す）。
/// **上げる向きは報告が長くなるだけ**なので見張っていない（ファイル先頭の doc）。
#[test]
fn the_watched_list_is_capped_and_says_how_many_were_left_out() {
    // 判定しない種別（5）で、上限より 3 件多く植える。**どれも規則の外側の形。**
    let planted: Vec<(String, i32)> = (0..WATCHED_LIST_LIMIT + 3)
        .map(|index| (format!(">>ABSTRACT{index}<<"), 5))
        .collect();
    let entries: Vec<(&str, i32)> = planted
        .iter()
        .map(|(spelling, kind)| (spelling.as_str(), *kind))
        .collect();

    let vocabulary =
        read_vocabulary(planted_gguf(&entries).as_slice()).expect("植えた gguf が読めること");
    let result = survey("capped.gguf", &vocabulary);

    assert_eq!(result.watched.len(), WATCHED_LIST_LIMIT + 3);

    let block = survey_block(&result);
    let listed = block
        .lines()
        .filter(|line| line.starts_with("  - id "))
        .count();
    assert_eq!(listed, WATCHED_LIST_LIMIT, "上限で切っていない: {block}");
    assert!(
        block.contains("… ほか 3 件"),
        "切った件数を言っていない: {block}"
    );
}

// ---------------------------------------------------------------------------
// 8. 「予約が門の外にいた」穴の見張り番（作業単位 E-3）
// ---------------------------------------------------------------------------

thread_local! {
    /// **このスレッドが割り当て器へ要求した累計バイト数**（解放では減らさない）。
    static REQUESTED_BYTES: Cell<usize> = const { Cell::new(0) };
}

/// 要求されたバイト数を数える。
///
/// **TLS の破棄後は数えないだけ**で、割り当てはそのまま通す。
fn record_request(bytes: usize) {
    let _ = REQUESTED_BYTES.try_with(|counter| counter.set(counter.get().saturating_add(bytes)));
}

/// **要求されたバイト数を数えてから `System` に委ねる割り当て器。**
///
/// # なぜソースの走査ではなく、これなのか
///
/// **観測したいのは「`Vec::with_capacity` に何を渡したか」ではなく、
/// 「1バイトも読まないうちに何バイト要求したか」**である。
/// 綴りを走査すると、**呼び出し側を書き換える変異が走査の綴りごと動いて緑になりうる**
/// （`15b` の自己成就）。**ここは実際の [`read_vocabulary`] を通し、
/// 割り当て器が数えた実数を見る。**
///
/// # ⚠ 実測由来の値ではない（ADR-0036）
///
/// 数えているのは **RSS でも時間でもなく、プログラム自身が要求したバイト数**。
/// **他の機でも同じ数になる**（要素の大きさは [`size_of`] が決める）。
/// 閾値も [`MAX_INITIAL_RESERVE_BYTES`] から導いていて、生の数字では書いていない。
///
/// # ⚠ 数える側が壊れたら、上の見張り番は全部空振りする
///
/// だから対照が要る ——[`the_allocation_counter_sees_a_reservation_that_would_trip_the_watchdog`]
/// が**両向き**（大きい確保が見えること / 何もしなければ数えないこと）を当てている。
struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_request(layout.size());
        // SAFETY: 呼び出し側の契約をそのまま `System` へ渡すだけ。
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_request(layout.size());
        // SAFETY: 同上。
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: 同上（数えない ——見ているのは「要求した量」）。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // 伸びたぶんだけ数える（縮んだときは 0）。
        record_request(new_size.saturating_sub(layout.size()));
        // SAFETY: 同上。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static COUNTING_ALLOCATOR: CountingAllocator = CountingAllocator;

/// **その処理が、このスレッドで要求したバイト数を数える。**
fn bytes_requested<T>(action: impl FnOnce() -> T) -> (T, usize) {
    REQUESTED_BYTES.with(|counter| counter.set(0));
    let value = action();
    let used = REQUESTED_BYTES.with(Cell::get);
    (value, used)
}

/// **細工したヘッダ** ——「要素数 `length` の配列」だと名乗るだけで、**要素は1バイトも無い。**
///
/// # ⚠ キーと値の型は、この関数の中のリテラル
///
/// [`planted_gguf`] と同じ理由 ——走査側の定数（[`KEY_TOKENS`] / [`TYPE_ARRAY`] ほか）を
/// 参照すると、綴りや数字を壊す変異が両側を同時に動かして緑のまま通る（`15b`）。
fn hostile_array_header(key: &str, element_type: u32, length: u64) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3_u32.to_le_bytes()); // 版
    out.extend_from_slice(&0_u64.to_le_bytes()); // テンソル数
    out.extend_from_slice(&1_u64.to_le_bytes()); // メタデータの件数
    push_gguf_string(&mut out, key);
    out.extend_from_slice(&9_u32.to_le_bytes()); // 値の型 ＝ 配列
    out.extend_from_slice(&element_type.to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    out
}

/// **キーの文字列長だけを名乗るヘッダ**（**中身は1バイトも無い**）。
fn claimed_string_header(length: u64) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"GGUF");
    out.extend_from_slice(&3_u32.to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes());
    out.extend_from_slice(&1_u64.to_le_bytes());
    out.extend_from_slice(&length.to_le_bytes());
    out
}

/// **予約は、申告ではなく「確保してよいバイト数」で決まる**（E-3 の存在理由 / 純粋関数の側）。
#[test]
fn the_initial_reservation_is_capped_by_bytes_not_by_the_claimed_length() {
    // **この機の要素の大きさ。** ここが変われば下の数も変わるので、前提として先に当てる。
    assert_eq!((size_of::<String>(), size_of::<i32>()), (24, 4));

    let claimed = usize::try_from(MAX_ARRAY_LEN).expect("64bit で収まること");

    // **押さえが無ければ、1要素も読む前にこれだけ要求していた**（E-1 のレビュアーの実測）。
    assert_eq!(claimed * size_of::<String>(), 402_653_184);
    assert_eq!(claimed * size_of::<i32>(), 67_108_864);

    // **押さえた先。文字列の枝と整数の枝で違う**（要素の大きさが 6 倍違う）。
    assert_eq!(initial_capacity::<String>(claimed), 2_730);
    assert_eq!(initial_capacity::<i32>(claimed), 16_384);

    // どちらも天井の中に収まっている。
    assert!(initial_capacity::<String>(claimed) * size_of::<String>() <= MAX_INITIAL_RESERVE_BYTES);
    assert!(initial_capacity::<i32>(claimed) * size_of::<i32>() <= MAX_INITIAL_RESERVE_BYTES);
}

/// **有り得る申告は、そのまま全部予約する**（＝「いつでも 0」でも「いつでも天井」でもない）。
#[test]
fn a_plausible_length_is_reserved_in_full() {
    assert_eq!(initial_capacity::<String>(0), 0);
    assert_eq!(initial_capacity::<String>(1), 1);
    assert_eq!(initial_capacity::<i32>(1), 1);

    // **境目のちょうど内側と外側。**
    assert_eq!(initial_capacity::<String>(2_730), 2_730);
    assert_eq!(initial_capacity::<String>(2_731), 2_730);
    assert_eq!(initial_capacity::<i32>(16_384), 16_384);
    assert_eq!(initial_capacity::<i32>(16_385), 16_384);
}

/// **数える側が本当に見えていること**（対照 / これが空振りなら下の1本は何も測っていない）。
///
/// # ⚠ 腕は3本ある。1本だけ当てると、残り2本が死ぬ
///
/// **`alloc` しか当てていなかった**（E-3 のレビューが実測: `realloc` を数えない変異も
/// `alloc_zeroed` を数えない変異も、**1910 全緑 / exit 0**）。
///
/// **死んだ `realloc` の腕は、この作業単位の穴をそのまま作り直せる**
/// （CLAUDE.md「第5の分類 —— 2つの変異が互いを隠し合う」）:
///
/// ```text
/// let mut values = Vec::with_capacity(1);
/// values.reserve_exact(length);   // ← 1要素も読む前に約 402MB。realloc を通る
/// ```
///
/// **`realloc` を数えない変異と組にすると、また 1910 全緑になる。**
/// だから**確保のしかたを3通り**（新規 / 伸ばす / 0 埋め）**別々に当てる。**
#[test]
fn the_allocation_counter_sees_a_reservation_that_would_trip_the_watchdog() {
    let limit = MAX_INITIAL_RESERVE_BYTES * 2;

    // **確保する量は定数から作らない**（`16b`）。**天井を動かす変異が対照の確保まで
    // 一緒に押し上げ、8 TiB を要求して abort になった**（是正中に実測: `passed=1876 failed=0` / exit 101）。
    // **abort は「失敗したテスト」に数えられない**（CLAUDE.md）ので、生の数字で書く。
    let big = 1_048_576_usize;
    assert!(
        big > limit,
        "対照の確保が天井以下で、見張り番の閾値を超えられない\
         （確保 {big} / 天井 {limit}）。MAX_INITIAL_RESERVE_BYTES が上がっていないか"
    );

    // 1. 新規の確保（`alloc`）。
    let ((), requested) = bytes_requested(|| {
        let doomed: Vec<u8> = Vec::with_capacity(big);
        std::hint::black_box(&doomed);
    });
    assert!(
        requested >= big,
        "割り当て器が新規の確保を数えていない: {requested}"
    );

    // 2. **伸ばす確保**（`realloc`）。**`reserve_exact` で穴を作り直せる経路。**
    let ((), grown) = bytes_requested(|| {
        let mut doomed: Vec<u8> = Vec::with_capacity(1);
        doomed.reserve_exact(big);
        std::hint::black_box(&doomed);
    });
    assert!(grown >= big, "伸ばした確保を数えていない: {grown}");

    // 3. **0 埋めの確保**（`alloc_zeroed`）。`vec![0_u8; n]` はこちらを通る。
    let (_, zeroed) = bytes_requested(|| std::hint::black_box(vec![0_u8; big]).len());
    assert!(zeroed >= big, "0 埋めの確保を数えていない: {zeroed}");

    // **何もしなければ、天井を超えない**（＝大きい数に張り付いた常数ではない）。
    let ((), idle) = bytes_requested(|| ());
    assert!(idle < limit, "何もしていないのに {idle} バイト数えた");
}

/// **細工したヘッダで、読む前に大きな確保をさせられない**（E-3 の存在理由 / 実際の経路）。
///
/// # ⚠ 3つの枝を別々に通す
///
/// **文字列の枝だけが 1 要素 24 バイト**で、同じ申告でも整数の枝の 6 倍を確保する。
/// **「同じ形が3つある」ように見えて危なさが違う**ので、代表を1つ選ばない（`D-6` の再演を避ける）。
///
/// # ⚠ 上限だけでは足りない —— **下限が「予約の場所まで届いた」の権威**
///
/// **`読み取りに失敗` は `read_exact` の文面で、読み手のあらゆる段階が同じ文面で落ちる**
/// （先頭4バイト / 版 / テンソル数 / kv 件数 / キーの長さ / キーの本体 / 値の型 /
/// 要素の型 / 配列の長さ）。**「門で弾かれていない」までしか言えない。**
///
/// **E-3 のレビューが実測**: [`hostile_array_header`] の最後の1行（配列の長さ）を落とすと、
/// **読み手は `read_array` の予約に一度も到達しないのに、上限だけの検査は緑**
/// （要求は数十バイト）。**手前に検査が1つ入った日に、この1本は静かに空振りへ変わる**（`B-6`）。
///
/// **だから「押さえた予約ちょうど」を下限で当てる** ——
/// 文字列の枝は 2,730 × 24 ＝ **65,520**、整数の枝は 16,384 × 4 ＝ **65,536**。
#[test]
fn a_crafted_header_cannot_make_the_reader_reserve_before_reading() {
    let limit = MAX_INITIAL_RESERVE_BYTES * 2;
    let claimed = usize::try_from(MAX_ARRAY_LEN).expect("64bit で収まること");

    // **値の型は走査側の定数ではなく数字で書く**（`planted_gguf` と同じ理由 / `15b`）。
    // **`floor` は押さえた予約そのもの**（2,730 / 16,384 と同じくベタ書きで一貫させる）。
    for (key, element_type, label, without_the_cap, floor) in [
        (
            "tokenizer.ggml.tokens",
            8_u32,
            "文字列",
            claimed * size_of::<String>(),
            65_520_usize,
        ),
        (
            "tokenizer.ggml.token_type",
            5_u32,
            "INT32",
            claimed * size_of::<i32>(),
            65_536_usize,
        ),
        (
            "tokenizer.ggml.token_type",
            4_u32,
            "UINT32",
            claimed * size_of::<i32>(),
            65_536_usize,
        ),
    ] {
        let bytes = hostile_array_header(key, element_type, MAX_ARRAY_LEN);

        // **細工の側は小さい。** 要素の中身は1バイトも無い。
        assert!(
            bytes.len() < 100,
            "{label}: 細工したヘッダが {} バイトある",
            bytes.len()
        );
        // **押さえが無ければ、桁で上を要求していた。**
        assert!(
            without_the_cap > 64 * limit,
            "{label}: 細工の側が小さすぎて、押さえの有無を区別できない\
             （押さえが無いときの要求 {without_the_cap} / 天井 {limit}）。\
             細工したヘッダの申告か MAX_INITIAL_RESERVE_BYTES を疑うこと"
        );

        let (outcome, requested) = bytes_requested(|| read_vocabulary(bytes.as_slice()));

        // **門で弾かれていないこと。** ただし文面は読み手のどの段階でも同じなので、
        // **これは「予約まで届いた」の権威ではない**（下の下限がその権威）。
        let error = outcome.expect_err("要素が1件も無いので Err になること");
        assert!(
            error.contains("読み取りに失敗"),
            "{label}: 門で弾かれていて、予約まで届いていない: {error}"
        );
        // **下限 —— 予約の場所まで実際に届いた。**
        assert!(
            requested >= floor,
            "{label}: 予約まで届いていない（{requested} バイトしか要求していない / \
             押さえた予約は {floor}）。細工したヘッダが読み手の手前で落ちていないか疑うこと"
        );
        // **上限 —— 届いたうえで、押さえが効いている。**
        assert!(
            requested <= limit,
            "{label}: 1バイトも読む前に {requested} バイトを要求した\
             （天井 {limit} / 押さえが無ければ {without_the_cap}）"
        );
    }
}

/// **天井より長い語彙でも、1件残らず読める。**
///
/// 押さえは**予約の頭**であって、**読む件数の上限ではない。**
/// `initial_capacity` を `for _ in 0..` の側にも使う変異は、この1本でだけ赤くなる。
#[test]
fn a_vocabulary_longer_than_the_initial_reservation_is_read_in_full() {
    // **整数の枝の天井を1件超える**（文字列の枝の天井は、そのはるか下）。
    //
    // **件数は定数から作らない**（`16b`）——**天井を上げる変異が件数まで押し上げ、
    // 2748 億件の語彙を組もうとして abort になった**（是正中に実測: `passed=1876 failed=0` / exit 101）。
    // **16,385 は 64 KiB ÷ 4 ＋ 1 である。**
    let count = 16_385_usize;
    assert!(
        count > initial_capacity::<i32>(count),
        "16,385 件が整数の枝の天井を超えていない（MAX_INITIAL_RESERVE_BYTES が上がっていないか）"
    );
    assert!(
        count > initial_capacity::<String>(count),
        "16,385 件が文字列の枝の天井を超えていない（MAX_INITIAL_RESERVE_BYTES が上がっていないか）"
    );

    let planted: Vec<(String, i32)> = (0..count)
        .map(|index| (format!("<t{index}>"), if index % 2 == 0 { 3 } else { 1 }))
        .collect();
    let entries: Vec<(&str, i32)> = planted
        .iter()
        .map(|(spelling, kind)| (spelling.as_str(), *kind))
        .collect();

    let vocabulary =
        read_vocabulary(planted_gguf(&entries).as_slice()).expect("植えた gguf が読めること");

    assert_eq!(vocabulary.tokens.len(), count);
    assert_eq!(vocabulary.token_types.len(), count);
    // **端が落ちていない。**
    assert_eq!(vocabulary.tokens[0], "<t0>");
    assert_eq!(vocabulary.tokens[count - 1], format!("<t{}>", count - 1));
    // **種別も件数ぶん読めている**（偶数が 3・奇数が 1）。
    assert_eq!(vocabulary.type_histogram(), vec![(1, 8_192), (3, 8_193)]);
}

/// **有り得ない要素数は、名前つきの `Err`**（[`MAX_ARRAY_LEN`] の門）。
///
/// # ⚠ E-3 のあと、この門が守っているのは「記憶」ではなく「名前」である
///
/// 予約に天井が付いたので、**門を外しても要求は増えない**
/// （読む前の要求は [`MAX_INITIAL_RESERVE_BYTES`] で頭打ちになり、
/// あとは要素を読みに行って尽きる）。**残る値打ちは、直す先を名乗ること** ——
/// **`配列が長すぎる` は「壊れた／敵対的なファイル」を指し、
/// `読み取りに失敗` は「途中で終わったファイル」を指す。**
/// **止まる場所ではなく、名指しする先を比べること**（`B-38`）。
#[test]
fn an_array_length_beyond_the_gate_is_a_named_error() {
    let over = hostile_array_header("tokenizer.ggml.tokens", 8, MAX_ARRAY_LEN + 1);
    let error = read_vocabulary(over.as_slice()).expect_err("Err になること");
    assert!(error.contains("配列が長すぎる"), "{error}");

    // **境目のちょうど内側は門を通る**（門が広く効きすぎていないことの対照）。
    let at = hostile_array_header("tokenizer.ggml.tokens", 8, MAX_ARRAY_LEN);
    let error = read_vocabulary(at.as_slice()).expect_err("要素が無いので Err にはなること");
    assert!(error.contains("読み取りに失敗"), "{error}");
}

/// **有り得ない文字列長は、名前つきの `Err`**（[`MAX_STRING_BYTES`] の門）。
///
/// # ⚠ この門も、予約より先に効く。ただし予約のほうにも押さえが要った
///
/// **以前は `vec![0_u8; length]` で、門を通った長さを読む前に確保して 0 で埋めていた。**
/// 記憶の観点では門の中（上限は門の値そのもの）だが、
/// **押さえが無いと、この門の値を上げる変異が名前つきの赤ではなく abort になる**
/// （`passed=N failed=0` / exit 101 —— **件数を読む道具はこれを緑と読む**）。
/// **いまは配列の枝と同じ [`initial_capacity`] を通す**ので、
/// **同じ変異がこのテストのクリーンな赤になる。**
///
/// # ⚠ 申告は定数から作らない（`16b`）
///
/// **以前は `MAX_STRING_BYTES + 1` と `MAX_STRING_BYTES` を渡していた。**
/// **門の値を上げる変異が申告も一緒に押し上げるので、両側が動いて緑のまま通った**
/// （E-3 の是正中に実測: `1 << 40` にして **1910 全緑 / exit 0**。
/// **abort が唯一の検出手段だったので、abort を消した瞬間に無防備になった**）。
/// **だから申告は生の数字で書く。** ここの `1_048_576` は `1 << 20` である。
#[test]
fn a_string_length_beyond_the_gate_is_a_named_error() {
    let error =
        read_vocabulary(claimed_string_header(1_048_577).as_slice()).expect_err("Err になること");
    assert!(
        error.contains("文字列が長すぎる"),
        "1 MiB を1バイト超える申告が門を通った（MAX_STRING_BYTES が上がっていないか）: {error}"
    );

    // **境目のちょうど内側は門を通る**（門が広く効きすぎていないことの対照）。
    let error = read_vocabulary(claimed_string_header(1_048_576).as_slice())
        .expect_err("中身が無いので Err にはなること");
    assert!(
        error.contains("読み取りに失敗"),
        "ちょうど 1 MiB の申告が門で弾かれた（MAX_STRING_BYTES が下がっていないか）: {error}"
    );
}

/// **文字列の枝でも、読む前に大きな確保をさせられない**（配列の枝と対称 / `16e-b`）。
///
/// **申告はちょうど 1 MiB（＝門の内側）**なので門は通る。**32 バイトのヘッダで起こせる。**
/// 兄弟の [`a_crafted_header_cannot_make_the_reader_reserve_before_reading`] と同じく、
/// **下限で「予約の場所まで届いた」を、上限で「押さえが効いている」を当てる**
/// ——`読み取りに失敗` は読み手のどの段階でも出るので、文面は権威にならない。
#[test]
fn a_crafted_string_length_cannot_make_the_reader_reserve_before_reading() {
    let limit = MAX_INITIAL_RESERVE_BYTES * 2;
    let without_the_cap = 1_048_576_usize;

    let bytes = claimed_string_header(1_048_576);
    assert!(
        bytes.len() < 100,
        "細工したヘッダが {} バイトある",
        bytes.len()
    );

    let (outcome, requested) = bytes_requested(|| read_vocabulary(bytes.as_slice()));

    let error = outcome.expect_err("中身が1バイトも無いので Err になること");
    assert!(
        error.contains("読み取りに失敗"),
        "門で弾かれていて、予約まで届いていない: {error}"
    );
    assert!(
        requested >= MAX_INITIAL_RESERVE_BYTES,
        "予約まで届いていない（{requested} バイトしか要求していない / \
         押さえた予約は {MAX_INITIAL_RESERVE_BYTES}）"
    );
    assert!(
        requested <= limit,
        "1バイトも読む前に {requested} バイトを要求した\
         （天井 {limit} / 押さえが無ければ {without_the_cap}）"
    );
}
