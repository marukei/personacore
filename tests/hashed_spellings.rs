//! **綴りそのものを木に書けない名前**（第三者の製品の名前・利用者の実名）が、追跡ファイルに残っていないことを見る
//! （作業単位 O-11 / Issue origin#105 / ADR-0048 の追記（2026-11-04）の2）。
//!
//! # なぜ要るか
//!
//! 公開側には、第三者の製品の名前と利用者の実名を残さない（ADR-0048 の追記の2）。
//! 一度片付けても、次の作業単位が書き戻す —— だから見張る。ところが、**探す綴りをそのまま書くと、
//! 見張り番自身がその綴りを公開する**（`tests/private_spellings.rs` は断片から組み立てて書いているが、
//! 断片を並べれば読める）。そこで綴りの持ち方を2つに分ける。
//!
//! | 綴り | 持ち方 |
//! |---|---|
//! | 第三者の製品の名前 | 畳んだ綴りの **SHA-256** を表 [`HASHED`] に持つ |
//! | 利用者の実名 | **木には何も持たない。** **実行する人が旗 [`CHECK_USER_NAME_ENV`]`=1` を付けたときだけ**、テストの実行時に `git config user.name` を読み、その語と突き合わせる。漢字などの追加の綴りは、環境変数 [`EXTRA_SPELLINGS_ENV`] が指す、**どの Git の作業ツリーにも入っていない**ファイルから読む |
//!
//! 実名をハッシュでも持たないのは、**ハッシュと「姓（ローマ字）」という説明と長さがあれば、
//! 姓と名の一覧に当てるだけで実名が確かめられる**から（O-11 のレビュー）。
//!
//! # 何を見ているか
//!
//! `git ls-files` が挙げる**追跡されたファイル**を読み（UTF-8 として不正なバイトは置き換え文字にして、
//! 残りの部分を見る）、1行ずつ次の順に見る。
//!
//! 1. **全角英数字を ASCII に畳む**（`Ｑ` → `Q`、`１` → `1`。U+FF10–FF19 / FF21–FF3A / FF41–FF5A）。
//! 2. 語を切り出す。切り出し方は綴りの文字の種類（[`Form`]）ごとに違う。
//!
//! | 種類 | 切り出す語 | 畳み方 |
//! |---|---|---|
//! | `AsciiWord` | ASCII の英字と数字だけの連なり（記号・空白・日本語などで切れる。`_` `-` `.` でも切れる）。**加えて**、その連なりを小文字→大文字・英字↔数字・大文字の連なり→「大文字＋小文字」の境目で割った断片（`keyProtectedRelay` → `key` / `Protected` / `Relay`、`HTTPClient` → `HTTP` / `Client`） | ASCII の大文字を小文字に |
//! | `Katakana` | 全角カタカナ（長音符 `ー` を含む）の連なりの中の、**表の文字数と同じ長さの窓すべて** | 畳まない |
//! | `Kanji` | 漢字（CJK 統合漢字・拡張 A・互換漢字・`々`）の連なりの中の、**表の文字数と同じ長さの窓すべて** | 畳まない |
//!
//! 3. 表 [`HASHED`] とは、語の UTF-8 のバイト列の SHA-256（16 進の小文字）で突き合わせる。
//!    実行時の平文の綴り（[`Plain`]）とは、ASCII だけの綴りなら上の `AsciiWord` の語と等しいかで、
//!    それ以外の綴り（漢字など）なら、畳んだ行の中に部分として現れるかで突き合わせる。
//!
//! 当たったら `ファイル:行:種類:何の綴りか` で報告する。
//! **報告にも assert の文面にも、当たった語・実名の綴り・追加の綴りのファイルのパスと中身は載せない。**
//!
//! **速さ**: `AsciiWord` は長さで絞らず、全部の語をハッシュする。一度ハッシュした語は覚えておいて二度とハッシュしない。
//! 窓の種類は、表にその種類の行があるときだけ、表の文字数の窓だけを切り出す。
//!
//! # 実名の検査は、実行する人が選んだときだけ（**旗が無ければ、利用者の手元でも見ない**）
//!
//! 実名の検査は**既定では行わない。** 公開した後に他の人が `cargo test` を回すと、その人の `user.name` の語が
//! ふつうの英単語として木にあるだけで赤になり、直す手段が無い（`user.name` が `Claude` なら `CLAUDE.md` に当たる）。
//! だから、**この開発機の利用者の手元でも、旗を付けない限り実名は見ない。**
//!
//! | 旗・設定 | 値 | 振る舞い |
//! |---|---|---|
//! | [`CHECK_USER_NAME_ENV`] | 無い・空 | `user.name` の語の検査を飛ばし、そのことを標準エラーに1行出す |
//! | | `1` | `git config user.name` を読み、その語と突き合わせる（`user.name` が無い・空なら、飛ばしたことを1行出す） |
//! | | **それ以外** | **赤**（打ち間違いを黙って見ないままにしない）。判定は [`user_name_to_check`] |
//! | [`EXTRA_SPELLINGS_ENV`] | 無い・空 | 追加の綴りの検査を飛ばし、そのことを1行出す |
//! | | ファイルのパス | そのファイルの語と突き合わせる（下の規則） |
//!
//! * **旗は、利用者が自分の環境に置く**（シェルの環境変数か、cargo の**利用者レベルの**設定
//!   ——例 `~/.cargo/config.toml` の `[env]`）。**リポジトリの `.cargo/config.toml` には足さない**（公開側の全員に効く）。
//! * 2つの旗を分けてあるのは、名前だけを見たい人に、中身のあるファイルを要求しないため。
//! * **[`EXTRA_SPELLINGS_ENV`] が Git の作業ツリーの中を指していたら赤にする**（追加の綴りを木に置いたら、
//!   それ自体が実名の公開になる）。中かどうかは2つで見る ——
//!   この作業ツリーの根の下か、**ファイルの親で `git rev-parse --is-inside-work-tree` が成功するか**
//!   （ワークツリーから回したときの本体のチェックアウトなど、**別の木の中**も赤にする）。
//!   「外」と扱うのは、`git` が「Git のリポジトリではない」と答えたときだけで、
//!   **`git` を起こせない・別の理由で失敗したときは赤にする**（黙って「外」と扱わない）。
//! * 指すファイルが読めないとき、**空か空の行だけのとき**も赤にする（綴りを足したつもりで見ていない、を避ける）。
//! * ファイルの形は「平文で1行1語」（前後の空白は落とし、空の行は読まない）。
//! * `user.name` の語のうち、ASCII で3文字未満の語（イニシャル等）と、ASCII 以外で1文字の語は使わない
//!   （ありふれた語に当たって、毎回赤になる）。追加の綴りのファイルの語は、長さで落とさない。
//!
//! # ハッシュで持つことの限界（**秘密にはなっていない**）
//!
//! * **短い語のハッシュは辞書で戻せる。** 製品名のような英字の語は、辞書や総当たりで特定できる。
//!   **表は綴りを秘密にしていない。** 目的は2つだけ —— **木に平文で出さないこと**と、
//!   **名前で検索した人に、このファイルが引っかからないこと**。
//! * **窓の種類（カタカナ・漢字）の行は、文字数を平文で持つ**（窓の長さに使う）。`AsciiWord` の行は長さを持たない。
//! * **表の値が、本当にその綴りのハッシュであることは、このテストは確かめられない**（確かめるには平文が要る）。
//!   植えた対照（[`a_planted_spelling_is_found_by_the_scan`] ほか）は、**同じ走査の本体に、実行時に作った表と綴りを通して**
//!   切り出しと突き合わせが効くことを見る。本物の表の値は、O-11 で**置き換える前の木に当てて赤になること**を測った
//!   （変異の記録は `<measurements>` の `o11-…` の下）。
//!
//! # 何を見ていないか（先に書く）
//!
//! * **語の一部は見ていない。** `AsciiWord` の綴りは「語の全体」か「境目で割った断片」が一致したときだけ当たる。
//!   前後に英字が続く形（複数形・接頭辞つき・全部小文字で繋げた合成語）や、途中に記号を挟んだ綴りは当たらない。
//! * **同じ綴りの、無関係な語にも当たる**（ふつうの英単語として使われる綴りなら、その語にも鳴る）。
//!   当たったら、言い換えるか、理由つきで [`ALLOWED`] に載せる。
//! * **半角カタカナ・ひらがな・ローマ字の別の綴り（長音の書き方の違い等）は見ていない。** 表に無い綴りは当たらない。
//! * **漢字の実名は、木には登録しない。** 見るときは、姓と名を1行ずつ書いたファイルをどの Git の作業ツリーにも入らない場所に置き、
//!   [`EXTRA_SPELLINGS_ENV`] でそのパスを渡す。
//! * **UTF-16 などで書かれたファイルは見ていない**（ASCII の文字の間に別のバイトが挟まり、語として切り出せない）。
//! * **追跡されていないファイル・`.git/` の中（コミットのメッセージ・作者欄）は見ていない。**
//!   作者欄の実名は ADR-0048 §2 の手順（公開側は noreply のアドレスで1コミット）が扱う。
//! * **このファイルも走査の対象**である。植えた対照の綴りは作り物の語で、本物の綴りはこのファイルのどこにも無い。
//!
//! # 表に行を足すとき
//!
//! 綴りを畳んで（ASCII なら小文字に）、UTF-8 のバイト列の SHA-256 を取る（`printf '%s' '<綴り>' | sha256sum` など）。
//! **綴りを、コミットのメッセージ・コメント・Issue・測定の記録に書かないこと**（書けば、ここで隠した意味が無い）。
//! **実名は表に足さない**（上の「持ち方」）。
//!
//! 許可リスト [`ALLOWED`] は空である。足すときは「なぜここだけ残すのか」を理由に書くこと
//! （理由が空ならテストが落ちる）。

#[path = "common/sha256.rs"]
mod sha256;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha256::sha256_hex;

/// 許可する（ファイル, 理由）。**いまは1つも無い。**
const ALLOWED: &[(&str, &str)] = &[];

/// 追加の綴り（漢字の実名など）のファイルを指す環境変数。ファイルはどの Git の作業ツリーにも入れない。
const EXTRA_SPELLINGS_ENV: &str = "PERSONACORE_PRIVATE_SPELLINGS";

/// `git config user.name` の語を見るかの旗。**`1` のときだけ見る**（無い・空なら飛ばす、それ以外は赤）。
/// 利用者が自分の環境に置く。リポジトリの `.cargo/config.toml` には足さない。
const CHECK_USER_NAME_ENV: &str = "PERSONACORE_CHECK_GIT_USER_NAME";

/// `git config user.name` の語に付ける「何の綴りか」。
const USER_NAME_NOTE: &str = "git の user.name の語";

/// 追加の綴りのファイルの語に付ける「何の綴りか」。
const EXTRA_NOTE: &str = "追加の綴りのファイルの語";

/// 何の綴りか。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Kind {
    /// 第三者の製品の名前（ADR-0048 の追記の2。一般名は「鍵で守られた中継」）。
    ThirdPartyProduct,
    /// 利用者の実名（実行時にだけ受け取る）。
    RealName,
}

/// 語の切り出し方（綴りの文字の種類）。
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
enum Form {
    /// ASCII の英数字の語（と、境目で割った断片）。小文字に畳む。長さで絞らない。
    AsciiWord,
    /// 全角カタカナの連なりの中の、`chars` 文字の窓。
    Katakana { chars: usize },
    /// 漢字の連なりの中の、`chars` 文字の窓。
    Kanji { chars: usize },
}

/// 表の1行。
#[derive(Debug, Clone, Copy)]
struct Hashed {
    kind: Kind,
    form: Form,
    /// 畳んだ綴りの UTF-8 のバイト列の SHA-256（16 進の小文字）。
    sha256: &'static str,
    /// 報告に載せる「何の綴りか」（綴りそのものは書かない）。
    note: &'static str,
}

/// 探す綴りの表。**綴りは平文で書かない**。**実名は持たない**（モジュールの doc）。
const HASHED: &[Hashed] = &[
    Hashed {
        kind: Kind::ThirdPartyProduct,
        form: Form::AsciiWord,
        sha256: "82ba7e1f6f4171d3648bb4f785ca31992efe32df6e024acb5bccd297feb2f263",
        note: "鍵で守られた中継の製品名（ローマ字）",
    },
    Hashed {
        kind: Kind::ThirdPartyProduct,
        form: Form::Katakana { chars: 4 },
        sha256: "22ccbc139f7df8ba144d5339ece5e5c9b6a20c260d03a7099c2a4dc73845f5e9",
        note: "鍵で守られた中継の製品名（カタカナ）",
    },
];

/// 実行時に受け取る平文の綴り（畳んだ形）。**木には書かない。**
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plain {
    kind: Kind,
    folded: String,
    note: &'static str,
}

/// 全角英数字（U+FF10–FF19 / FF21–FF3A / FF41–FF5A）を ASCII に畳む。ほかの文字はそのまま。
fn fold_fullwidth(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\u{FF10}'..='\u{FF19}' | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' => {
                char::from_u32(u32::from(ch) - 0xFEE0).unwrap_or(ch)
            }
            _ => ch,
        })
        .collect()
}

/// 語を畳む（全角英数字を ASCII に、ASCII の大文字を小文字に）。
fn fold_word(word: &str) -> String {
    fold_fullwidth(word).to_ascii_lowercase()
}

/// 全角カタカナ（`ァ`〜`ヺ`・長音符 `ー`・`ヽ` `ヾ` `ヿ`）か。中黒 `・` は含めない（語の区切りとして読む）。
fn is_katakana(ch: char) -> bool {
    matches!(ch, '\u{30A1}'..='\u{30FA}' | '\u{30FC}'..='\u{30FF}')
}

/// 漢字（CJK 統合漢字・拡張 A・互換漢字・`々`）か。
fn is_kanji(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}' | '\u{3005}'
    )
}

/// ASCII の英数字の連なりを、境目で割る。境目は3種類:
///
/// * 小文字→大文字（`keyRelay` → `key` / `Relay`）
/// * 英字↔数字（`relay2B` → `relay` / `2` / `B`）
/// * 大文字の連なりの最後の1文字の前で、その後ろに小文字が続くとき（`HTTPClient` → `HTTP` / `Client`）
///
/// 全部が大文字の連なり（`ABC`）は割らない。
fn ascii_pieces(run: &str) -> Vec<&str> {
    let bytes = run.as_bytes();
    let mut pieces = Vec::new();
    let mut start = 0;
    for i in 1..bytes.len() {
        let (prev, cur) = (bytes[i - 1], bytes[i]);
        let next = bytes.get(i + 1).copied();
        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_uppercase()
                && cur.is_ascii_uppercase()
                && next.is_some_and(|n| n.is_ascii_lowercase()))
            || (prev.is_ascii_alphabetic() && cur.is_ascii_digit())
            || (prev.is_ascii_digit() && cur.is_ascii_alphabetic());
        if boundary {
            pieces.push(&run[start..i]);
            start = i;
        }
    }
    pieces.push(&run[start..]);
    pieces
}

/// 畳んだ行から、`AsciiWord` の語を（小文字に畳んで）集める。
fn ascii_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in line
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|run| !run.is_empty())
    {
        out.push(run.to_ascii_lowercase());
        let pieces = ascii_pieces(run);
        if pieces.len() > 1 {
            out.extend(pieces.into_iter().map(str::to_ascii_lowercase));
        }
    }
    out
}

/// 行から、`belongs` の文字の連なりの中の `n` 文字の窓を全部集める。
fn windows(line: &str, belongs: fn(char) -> bool, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !belongs(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && belongs(chars[i]) {
            i += 1;
        }
        let run = &chars[start..i];
        if n <= run.len() {
            for window in run.windows(n) {
                out.push(window.iter().collect());
            }
        }
    }
    out
}

/// 当たり（表の行か、平文の綴りか）。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum Hit {
    Hashed(usize),
    Plain(usize),
}

/// 走査器。一度ハッシュした語の結果を覚えておく。
struct Scanner<'a> {
    hashed: &'a [Hashed],
    plain: &'a [Plain],
    /// 表に `AsciiWord` の行があるか。
    hashes_ascii: bool,
    /// 表にある窓の種類（重複なし）。
    window_forms: Vec<Form>,
    /// ASCII だけの平文の綴り → その添字。
    plain_ascii: HashMap<String, Vec<usize>>,
    /// それ以外の平文の綴りの添字（部分として探す）。
    plain_other: Vec<usize>,
    memo: HashMap<(Form, String), Vec<usize>>,
}

impl<'a> Scanner<'a> {
    fn new(hashed: &'a [Hashed], plain: &'a [Plain]) -> Self {
        let mut window_forms = Vec::new();
        for entry in hashed {
            if entry.form != Form::AsciiWord && !window_forms.contains(&entry.form) {
                window_forms.push(entry.form);
            }
        }
        let mut plain_ascii: HashMap<String, Vec<usize>> = HashMap::new();
        let mut plain_other = Vec::new();
        for (index, spelling) in plain.iter().enumerate() {
            if spelling.folded.is_empty() {
                continue;
            }
            if spelling.folded.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                plain_ascii
                    .entry(spelling.folded.clone())
                    .or_default()
                    .push(index);
            } else {
                plain_other.push(index);
            }
        }
        Self {
            hashed,
            plain,
            hashes_ascii: hashed.iter().any(|entry| entry.form == Form::AsciiWord),
            window_forms,
            plain_ascii,
            plain_other,
            memo: HashMap::new(),
        }
    }

    /// `form` の語 `word` に当たる表の行の添字（覚えてあればそれを返す）。
    fn hashed_hits(&mut self, form: Form, word: String) -> Vec<usize> {
        let key = (form, word);
        if let Some(matched) = self.memo.get(&key) {
            return matched.clone();
        }
        let digest = sha256_hex(key.1.as_bytes());
        let matched: Vec<usize> = self
            .hashed
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.form == form && entry.sha256 == digest)
            .map(|(index, _)| index)
            .collect();
        self.memo.insert(key, matched.clone());
        matched
    }

    /// 1行から当たりを返す（同じ行で同じ綴りは1回だけ）。
    fn hits_in_line(&mut self, line: &str) -> Vec<Hit> {
        let folded = fold_fullwidth(line);
        let mut hits = Vec::new();
        if self.hashes_ascii || !self.plain_ascii.is_empty() {
            for word in ascii_words(&folded) {
                if let Some(indices) = self.plain_ascii.get(&word) {
                    hits.extend(indices.iter().map(|&index| Hit::Plain(index)));
                }
                if self.hashes_ascii {
                    hits.extend(
                        self.hashed_hits(Form::AsciiWord, word)
                            .into_iter()
                            .map(Hit::Hashed),
                    );
                }
            }
        }
        for form in self.window_forms.clone() {
            let (belongs, n): (fn(char) -> bool, usize) = match form {
                Form::Katakana { chars } => (is_katakana, chars),
                Form::Kanji { chars } => (is_kanji, chars),
                Form::AsciiWord => continue,
            };
            for window in windows(&folded, belongs, n) {
                hits.extend(self.hashed_hits(form, window).into_iter().map(Hit::Hashed));
            }
        }
        if !self.plain_other.is_empty() {
            let lower = folded.to_ascii_lowercase();
            for &index in &self.plain_other {
                if lower.contains(self.plain[index].folded.as_str()) {
                    hits.push(Hit::Plain(index));
                }
            }
        }
        hits.sort_unstable();
        hits.dedup();
        hits
    }

    /// 当たりの（種類, 何の綴りか）。
    fn label(&self, hit: Hit) -> (Kind, &'static str) {
        match hit {
            Hit::Hashed(index) => (self.hashed[index].kind, self.hashed[index].note),
            Hit::Plain(index) => (self.plain[index].kind, self.plain[index].note),
        }
    }
}

/// `root` の下の `paths`（`root` からの相対）を読み、許可されていない当たりを `ファイル:行:種類:何の綴りか` で返す。
/// UTF-8 として不正なバイトは置き換え文字にして読む（ASCII の部分は見える）。
fn scan(
    root: &Path,
    paths: &[String],
    hashed: &[Hashed],
    plain: &[Plain],
    allowed: &[(&str, &str)],
) -> Vec<String> {
    let mut scanner = Scanner::new(hashed, plain);
    let mut found = Vec::new();
    for path in paths {
        if allowed.iter().any(|(name, _)| name == path) {
            continue;
        }
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            for hit in scanner.hits_in_line(line) {
                let (kind, note) = scanner.label(hit);
                found.push(format!("{path}:{}:{kind:?}:{note}", index + 1));
            }
        }
    }
    found
}

/// `git config user.name` の値から、突き合わせに使う語を切り出す（畳んだ形）。
///
/// 英数字（ASCII 以外を含む）の連なりで切る。ASCII で3文字未満の語と、ASCII 以外で1文字の語は使わない。
fn user_name_words(raw: &str) -> Vec<String> {
    fold_fullwidth(raw)
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| {
            if word.is_ascii() {
                word.len() >= 3
            } else {
                word.chars().count() >= 2
            }
        })
        .map(fold_word)
        .collect()
}

/// 実行時の平文の綴りを組む。`user_name` は `git config user.name` の値（無ければ `None`）、
/// `extra` は追加の綴りのファイルの行。**本体も植えた対照も、ここを通す。**
fn private_spellings(user_name: Option<&str>, extra: &[String]) -> Vec<Plain> {
    let mut plain = Vec::new();
    if let Some(raw) = user_name {
        for word in user_name_words(raw) {
            plain.push(Plain {
                kind: Kind::RealName,
                folded: word,
                note: USER_NAME_NOTE,
            });
        }
    }
    for line in extra {
        let word = fold_word(line.trim());
        if !word.is_empty() {
            plain.push(Plain {
                kind: Kind::RealName,
                folded: word,
                note: EXTRA_NOTE,
            });
        }
    }
    plain
}

/// `git config user.name` を読む。無い・空なら `None`。
fn git_user_name(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// 旗 [`CHECK_USER_NAME_ENV`] を見て決めた、`user.name` の語の扱い。
#[derive(Debug, PartialEq, Eq)]
enum UserNameToCheck {
    /// 旗が無い・空（見ない）。
    FlagOff,
    /// 旗は `1` だが、`user.name` が無い・空（見られない）。
    NoUserName,
    /// 旗は `1` で、この `user.name` を見る。
    Check(String),
}

/// 旗の値が `1` でも、無い・空でもなかった（打ち間違い）。**値は持たない**（文面に出さないため）。
#[derive(Debug, PartialEq, Eq)]
struct InvalidUserNameFlag;

/// **旗を見て、`user.name` を本体へ渡すか決める**（純粋関数）。
///
/// `flag` は [`CHECK_USER_NAME_ENV`] の値。`read_user_name` は `git config user.name` を読む関数で、
/// **旗が `1` のときだけ呼ぶ**（旗が無ければ、`git` の設定にも触れない）。
fn user_name_to_check(
    flag: Option<&OsStr>,
    read_user_name: impl FnOnce() -> Option<String>,
) -> Result<UserNameToCheck, InvalidUserNameFlag> {
    match flag {
        None => Ok(UserNameToCheck::FlagOff),
        Some(value) if value.is_empty() => Ok(UserNameToCheck::FlagOff),
        Some(value) if value == "1" => Ok(match read_user_name() {
            Some(name) => UserNameToCheck::Check(name),
            None => UserNameToCheck::NoUserName,
        }),
        Some(_) => Err(InvalidUserNameFlag),
    }
}

/// 追加の綴りのファイルの読み取りの結果。
#[derive(Debug, PartialEq, Eq)]
enum ExtraSpellings {
    /// 環境変数が無い・空（検査を飛ばす）。
    Unset,
    /// ファイルの行（空の行は落とした後。1つ以上ある）。
    Words(Vec<String>),
}

/// 追加の綴りのファイルを使えなかった理由。**パスも中身も持たない**（文面に出さないため）。
#[derive(Debug, PartialEq, Eq)]
enum ExtraSpellingsError {
    /// Git の作業ツリーの中を指している（この作業ツリーでも、別の木でも）。
    InsideWorkTree,
    /// 作業ツリーの中かどうかを確かめられない（`git` を起こせない）。
    CannotTellWorkTree,
    /// 読めない（無い・権限が無い等）。
    Unreadable,
    /// 読めたが、語が1つも無い（空か、空の行だけ）。
    NoWords,
}

/// `directory` が Git の作業ツリー（か、その `.git` の中）にあるか。
///
/// * 成功したら（`true` でも、`.git` の中の `false` でも）木の中。
/// * 失敗して、`git` が「Git のリポジトリではない」と言ったときだけ木の外。
/// * それ以外（`git` を起こせない・別の理由で失敗した —— パスを読めない等）は `Err`。**黙って「外」と扱わない。**
///
/// `git` の文面で見分けるので、翻訳されないよう `LC_ALL=C` で起こす。
fn is_inside_some_git_tree(directory: &Path) -> Result<bool, ExtraSpellingsError> {
    let output = Command::new("git")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .arg("-C")
        .arg(directory)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|_| ExtraSpellingsError::CannotTellWorkTree)?;
    if output.status.success() {
        return Ok(true);
    }
    if String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
        return Ok(false);
    }
    Err(ExtraSpellingsError::CannotTellWorkTree)
}

/// `setting`（環境変数の値）が指す追加の綴りのファイルを読む。`root` はこの作業ツリーの根。
fn read_extra_spellings(
    root: &Path,
    setting: Option<&OsStr>,
) -> Result<ExtraSpellings, ExtraSpellingsError> {
    let Some(raw) = setting.filter(|value| !value.is_empty()) else {
        return Ok(ExtraSpellings::Unset);
    };
    let resolved =
        std::fs::canonicalize(PathBuf::from(raw)).map_err(|_| ExtraSpellingsError::Unreadable)?;
    let tree = std::fs::canonicalize(root).map_err(|_| ExtraSpellingsError::Unreadable)?;
    if resolved.starts_with(&tree) {
        return Err(ExtraSpellingsError::InsideWorkTree);
    }
    let parent = resolved.parent().ok_or(ExtraSpellingsError::Unreadable)?;
    if is_inside_some_git_tree(parent)? {
        return Err(ExtraSpellingsError::InsideWorkTree);
    }
    let bytes = std::fs::read(&resolved).map_err(|_| ExtraSpellingsError::Unreadable)?;
    let words: Vec<String> = String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    if words.is_empty() {
        return Err(ExtraSpellingsError::NoWords);
    }
    Ok(ExtraSpellings::Words(words))
}

/// 検査を飛ばしたことを標準エラーへ1行出す。
///
/// `eprintln!` はテストの実行器に捕まえられ、緑の回には見えない。飛ばしたことは緑の回にこそ見えてほしいので、
/// 標準エラーへ直接書く（書けなくてもテストの判定には関わらないので、失敗は捨てる）。
fn notice(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{message}");
}

fn tracked_files(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git が起こせること");
    assert!(output.status.success(), "git ls-files が成功すること");
    String::from_utf8(output.stdout)
        .expect("ファイル名が UTF-8 であること")
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn both_newlines(source: &str) -> [String; 2] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

/// 植えた綴りの表の行を、実行時に作る（綴りは作り物の語。本物の表とは無関係）。
fn planted_entry(kind: Kind, form: Form, spelling: &str, note: &'static str) -> Hashed {
    let folded = if form == Form::AsciiWord {
        spelling.to_ascii_lowercase()
    } else {
        spelling.to_owned()
    };
    let digest: &'static str = Box::leak(sha256_hex(folded.as_bytes()).into_boxed_str());
    Hashed {
        kind,
        form,
        sha256: digest,
        note,
    }
}

/// 本体: 追跡されたファイルに、表の綴りと実名が残っていない。
#[test]
fn no_tracked_text_file_carries_a_hashed_spelling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = tracked_files(root);
    // このファイル自身も必ず走査する（まだ `git add` されていない間も見張れるように）。
    let own = "tests/hashed_spellings.rs".to_owned();
    if !files.contains(&own) {
        files.push(own);
    }
    // 走査が空振りしていない（git の出力が空・読めない、で緑にならない）こと。
    assert!(
        files.len() > 100,
        "追跡ファイルが少なすぎる: {}",
        files.len()
    );
    assert!(files.iter().any(|f| f == "CLAUDE.md"));

    let user_name = match user_name_to_check(
        std::env::var_os(CHECK_USER_NAME_ENV).as_deref(),
        || git_user_name(root),
    ) {
        Ok(UserNameToCheck::FlagOff) => {
            notice(&format!(
                "hashed_spellings: {CHECK_USER_NAME_ENV} が無いので、git の user.name の語の検査を飛ばした"
            ));
            None
        }
        Ok(UserNameToCheck::NoUserName) => {
            notice(
                "hashed_spellings: git の user.name が無い・空なので、その語の検査を飛ばした（実名はこの機では見ていない）",
            );
            None
        }
        Ok(UserNameToCheck::Check(name)) => Some(name),
        Err(InvalidUserNameFlag) => panic!(
            "{CHECK_USER_NAME_ENV} の値は `1`（見る）だけを受け付ける。見ないなら、外すか空にすること"
        ),
    };
    let extra = match read_extra_spellings(root, std::env::var_os(EXTRA_SPELLINGS_ENV).as_deref()) {
        Ok(ExtraSpellings::Unset) => {
            notice(&format!(
                "hashed_spellings: {EXTRA_SPELLINGS_ENV} が無いので、追加の綴りの検査を飛ばした"
            ));
            Vec::new()
        }
        Ok(ExtraSpellings::Words(words)) => words,
        Err(ExtraSpellingsError::InsideWorkTree) => panic!(
            "{EXTRA_SPELLINGS_ENV} が Git の作業ツリーの中を指している。追加の綴りのファイルは、どの木にも入らない場所に置くこと"
        ),
        Err(ExtraSpellingsError::CannotTellWorkTree) => panic!(
            "{EXTRA_SPELLINGS_ENV} が指すファイルが Git の作業ツリーの中か確かめられない（git を起こせない）"
        ),
        Err(ExtraSpellingsError::Unreadable) => {
            panic!("{EXTRA_SPELLINGS_ENV} が指すファイルを読めない")
        }
        Err(ExtraSpellingsError::NoWords) => {
            panic!("{EXTRA_SPELLINGS_ENV} が指すファイルに語が1つも無い（空か、空の行だけ）")
        }
    };
    let plain = private_spellings(user_name.as_deref(), &extra);

    let found = scan(root, &files, HASHED, &plain, ALLOWED);
    assert!(
        found.is_empty(),
        "公開側に残さない綴りが残っている（第三者の中継製品の名前は「鍵で守られた中継」/ \
         \"a key-protected relay\" に、実名は書かずに言い換えること）:\n{}",
        found.join("\n")
    );
}

/// 対照: 植えた綴りは、走査本体を通すと種類つきで見つかる。製品名は実行時に作った表で、
/// 実名は作り物の `user.name` を [`private_spellings`] に通して渡す。
/// 大文字小文字の混在・全角英数字・語の境界・境目で割った断片・LF と CRLF の両方。
#[test]
fn a_planted_spelling_is_found_by_the_scan() {
    let table = [planted_entry(
        Kind::ThirdPartyProduct,
        Form::AsciiWord,
        "quokka",
        "植えた製品名",
    )];
    let plain = private_spellings(Some("WALLAROO, Numbat"), &[]);
    let product = "ThirdPartyProduct:植えた製品名";
    let name = "RealName:git の user.name の語";
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("quokka", vec![product]),
        ("QUOKKA", vec![product]),
        ("qUoKkA の中継", vec![product]),
        // 語の境界: 記号・日本語・空白・`_`・`-`・`.`・`:` で切れる
        ("（Quokka）", vec![product]),
        ("`quokka_mcp:false`", vec![product]),
        ("quokka-main", vec![product]),
        ("http://quokka.local:1", vec![product]),
        ("中継はquokkaで", vec![product]),
        // 境目で割った断片（小文字→大文字・英字↔数字・大文字の連なりの後）
        ("const keyQuokka = 1", vec![product]),
        ("quokka2", vec![product]),
        ("QUOKKAClient", vec![product]),
        ("HTTPQuokka", vec![product]),
        // 全角英数字
        ("ＱＵＯＫＫＡ", vec![product]),
        ("ｑｕｏｋｋａの中継", vec![product]),
        ("ｋｅｙＱｕｏｋｋａ２", vec![product]),
        // 実名（作り物の user.name の語）
        ("wallaroo", vec![name]),
        ("Numbat さん", vec![name]),
        ("Ｗａｌｌａｒｏｏ", vec![name]),
        ("WallarooQuokka", vec![product, name]),
    ];
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    for (planted, labels) in &cases {
        // 当たりは行の末尾にも置く（CR が綴りの直後に来る形）。
        let body = format!("前の行\n{planted}\n次の行 {planted}");
        let mut expected: Vec<String> = Vec::new();
        for line in [2, 3] {
            for label in labels {
                expected.push(format!("planted.md:{line}:{label}"));
            }
        }
        expected.sort();
        for (arm, text) in both_newlines(&body).iter().enumerate() {
            std::fs::write(dir.path().join("planted.md"), text).expect("書ける");
            let mut found = scan(dir.path(), &["planted.md".to_owned()], &table, &plain, &[]);
            found.sort();
            assert_eq!(found, expected, "腕 {arm} / 植えた綴り {planted:?}");
        }
    }
}

/// 対照の裏: 語の一部・似た綴り・表に無い語は鳴らない（製品名も実名も）。
#[test]
fn a_word_that_only_contains_the_spelling_is_not_reported() {
    let table = [planted_entry(
        Kind::ThirdPartyProduct,
        Form::AsciiWord,
        "quokka",
        "植えた製品名",
    )];
    let plain = private_spellings(Some("Wallaroo"), &[]);
    let mut scanner = Scanner::new(&table, &plain);
    for line in [
        "quokkas",
        "xquokka",
        "quokkaland",
        "quo kka",
        "quok-ka",
        "wallaroos",
        "wallar oo",
        "鍵で守られた中継",
        "a key-protected relay",
        "",
    ] {
        assert_eq!(
            scanner.hits_in_line(line),
            Vec::<Hit>::new(),
            "誤爆: {line:?}"
        );
    }
}

/// `user.name` の語の切り出し: 区切り・全角・短い語の扱い。
#[test]
fn the_user_name_is_split_into_folded_words() {
    assert_eq!(
        user_name_words("WALLAROO, Numbat-Jo X"),
        vec!["wallaroo".to_owned(), "numbat".to_owned()]
    );
    assert_eq!(
        user_name_words("Ｗａｌｌａｒｏｏ　山田 太 郎"),
        vec!["wallaroo".to_owned(), "山田".to_owned()]
    );
    assert!(user_name_words("").is_empty());
    assert!(private_spellings(None, &[]).is_empty());
    assert_eq!(
        private_spellings(Some("Wallaroo"), &["  山田 ".to_owned(), String::new()]),
        vec![
            Plain {
                kind: Kind::RealName,
                folded: "wallaroo".to_owned(),
                note: USER_NAME_NOTE,
            },
            Plain {
                kind: Kind::RealName,
                folded: "山田".to_owned(),
                note: EXTRA_NOTE,
            },
        ]
    );
}

/// 対照: 追加の綴り（漢字など）は、連なりの中の部分にも当たる。ASCII だけの追加の綴りは語として当たる。
#[test]
fn a_planted_extra_spelling_is_found_by_the_scan() {
    let plain = private_spellings(None, &["山田".to_owned(), "Numbat".to_owned()]);
    let mut scanner = Scanner::new(&[], &plain);
    let cases: Vec<(&str, Vec<Hit>)> = vec![
        ("山田太郎", vec![Hit::Plain(0)]),
        ("小山田さん", vec![Hit::Plain(0)]),
        ("ＮＵＭＢＡＴ と山田", vec![Hit::Plain(0), Hit::Plain(1)]),
        ("numbats", vec![]),
        ("山の田", vec![]),
    ];
    for (line, expected) in cases {
        assert_eq!(scanner.hits_in_line(line), expected, "{line:?}");
    }
}

/// 対照: カタカナと漢字の窓。連なりの中の部分にも当たり、ひらがな・中黒・ASCII で切れる。
#[test]
fn a_planted_katakana_or_kanji_spelling_is_found_inside_a_run() {
    let table = [
        planted_entry(
            Kind::ThirdPartyProduct,
            Form::Katakana { chars: 4 },
            "クオッカ",
            "植えた製品名（カタカナ）",
        ),
        planted_entry(
            Kind::ThirdPartyProduct,
            Form::Kanji { chars: 2 },
            "袋熊",
            "植えた製品名（漢字）",
        ),
    ];
    let mut scanner = Scanner::new(&table, &[]);
    let cases: Vec<(&str, Vec<Hit>)> = vec![
        ("クオッカ", vec![Hit::Hashed(0)]),
        ("クオッカー・サーバ", vec![Hit::Hashed(0)]),
        ("ザクオッカの中継", vec![Hit::Hashed(0)]),
        ("袋熊科", vec![Hit::Hashed(1)]),
        ("大袋熊さん", vec![Hit::Hashed(1)]),
        ("クオッカと袋熊", vec![Hit::Hashed(0), Hit::Hashed(1)]),
        // 切れる・別の文字
        ("クオ・ッカ", vec![]),
        ("くおっか", vec![]),
        ("ｸｵｯｶ", vec![]),
        ("袋の熊", vec![]),
        ("袋 熊", vec![]),
    ];
    for (line, expected) in cases {
        assert_eq!(scanner.hits_in_line(line), expected, "{line:?}");
    }
}

/// 対照: 走査本体はファイルを読み、許可したファイルだけを飛ばす。UTF-8 として不正なファイルも ASCII の部分を見る。
#[test]
fn an_allowed_file_is_skipped_and_an_invalid_utf8_file_is_still_read() {
    let table = [planted_entry(
        Kind::ThirdPartyProduct,
        Form::AsciiWord,
        "quokka",
        "植えた製品名",
    )];
    let plain = private_spellings(None, &["山田".to_owned()]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    std::fs::write(dir.path().join("kept.md"), "quokka").expect("書ける");
    std::fs::write(dir.path().join("other.md"), "x\r\n山田さんの Quokka").expect("書ける");
    std::fs::write(
        dir.path().join("broken.bin"),
        [b"\xff\xfe ok\n".as_slice(), b"\x80QUOKKA\xc3\n".as_slice()].concat(),
    )
    .expect("書ける");
    let paths = vec![
        "kept.md".to_owned(),
        "other.md".to_owned(),
        "broken.bin".to_owned(),
    ];
    assert_eq!(
        scan(
            dir.path(),
            &paths,
            &table,
            &plain,
            &[("kept.md", "テスト用")]
        ),
        vec![
            "other.md:2:ThirdPartyProduct:植えた製品名".to_owned(),
            "other.md:2:RealName:追加の綴りのファイルの語".to_owned(),
            "broken.bin:2:ThirdPartyProduct:植えた製品名".to_owned(),
        ]
    );
    assert_eq!(scan(dir.path(), &paths, &table, &plain, &[]).len(), 4);
}

/// 追加の綴りのファイル: 作業ツリーの中を指したら赤（`InsideWorkTree`）、外なら読む、無い・空なら飛ばす、読めなければ赤。
#[test]
fn the_extra_spellings_file_must_live_outside_the_work_tree() {
    let tree = tempfile::tempdir().expect("作業ツリーの代わり");
    let outside = tempfile::tempdir().expect("外");
    std::fs::create_dir(tree.path().join("docs")).expect("作れる");
    let inside_file = tree.path().join("docs").join("names.txt");
    std::fs::write(&inside_file, "zzqq\n").expect("書ける");
    let outside_file = outside.path().join("names.txt");
    std::fs::write(&outside_file, "  山田 \n\nNumbat\n").expect("書ける");

    assert_eq!(
        read_extra_spellings(tree.path(), Some(inside_file.as_os_str())),
        Err(ExtraSpellingsError::InsideWorkTree)
    );
    // `..` を挟んでも、中は中
    let roundabout = tree
        .path()
        .join("docs")
        .join("..")
        .join("docs")
        .join("names.txt");
    assert_eq!(
        read_extra_spellings(tree.path(), Some(roundabout.as_os_str())),
        Err(ExtraSpellingsError::InsideWorkTree)
    );
    assert_eq!(
        read_extra_spellings(tree.path(), Some(outside_file.as_os_str())),
        Ok(ExtraSpellings::Words(vec![
            "山田".to_owned(),
            "Numbat".to_owned()
        ]))
    );
    assert_eq!(
        read_extra_spellings(tree.path(), None),
        Ok(ExtraSpellings::Unset)
    );
    assert_eq!(
        read_extra_spellings(tree.path(), Some(OsStr::new(""))),
        Ok(ExtraSpellings::Unset)
    );
    assert_eq!(
        read_extra_spellings(
            tree.path(),
            Some(outside.path().join("missing.txt").as_os_str())
        ),
        Err(ExtraSpellingsError::Unreadable)
    );
}

/// 追加の綴りのファイル: **別の Git の作業ツリーの中**（ワークツリーから回したときの本体のチェックアウト等）も赤。
/// この作業ツリーの根とは無関係な一時ディレクトリに `git init` した木へ置く。
///
/// 前提: 一時ディレクトリ（`%TEMP%` 等）そのものは、どの Git の木にも入っていないこと
/// （入っている機では、外の腕まで `InsideWorkTree` になって赤になる —— 鳴る向きなので見逃しはしない）。
#[test]
fn an_extra_spellings_file_inside_another_git_tree_is_refused() {
    let tree = tempfile::tempdir().expect("この作業ツリーの代わり");
    let other = tempfile::tempdir().expect("別の木");
    let status = Command::new("git")
        .arg("-C")
        .arg(other.path())
        .args(["init", "-q"])
        .status()
        .expect("git が起こせること");
    assert!(status.success(), "git init が成功すること");
    std::fs::create_dir(other.path().join("notes")).expect("作れる");
    let in_other_tree = other.path().join("notes").join("names.txt");
    std::fs::write(&in_other_tree, "zzqq\n").expect("書ける");
    assert_eq!(
        read_extra_spellings(tree.path(), Some(in_other_tree.as_os_str())),
        Err(ExtraSpellingsError::InsideWorkTree)
    );
    // `git` が「リポジトリではない」以外の理由で失敗したら、外とも中とも言わない（無いディレクトリ）。
    assert_eq!(
        is_inside_some_git_tree(&other.path().join("missing-directory")),
        Err(ExtraSpellingsError::CannotTellWorkTree)
    );
    // 対照: 同じ中身を木の外に置けば読める。
    let outside = tempfile::tempdir().expect("外");
    let outside_file = outside.path().join("names.txt");
    std::fs::write(&outside_file, "zzqq\n").expect("書ける");
    assert_eq!(
        read_extra_spellings(tree.path(), Some(outside_file.as_os_str())),
        Ok(ExtraSpellings::Words(vec!["zzqq".to_owned()]))
    );
}

/// 追加の綴りのファイル: 空・空の行だけなら赤（`NoWords`）。綴りを足したつもりで見ていない、を避ける。
#[test]
fn an_extra_spellings_file_without_words_is_refused() {
    let tree = tempfile::tempdir().expect("この作業ツリーの代わり");
    let outside = tempfile::tempdir().expect("外");
    for (name, body) in [("empty.txt", ""), ("blank.txt", "\n  \r\n\t\n")] {
        let file = outside.path().join(name);
        std::fs::write(&file, body).expect("書ける");
        assert_eq!(
            read_extra_spellings(tree.path(), Some(file.as_os_str())),
            Err(ExtraSpellingsError::NoWords),
            "{name}"
        );
    }
}

/// 旗 [`CHECK_USER_NAME_ENV`]: 無い・空なら渡さない（`user.name` を読みもしない）、`1` なら渡す、それ以外は赤。
#[test]
fn the_user_name_is_checked_only_when_the_flag_says_one() {
    let never = || -> Option<String> { panic!("旗が無いのに user.name を読んだ") };
    assert_eq!(
        user_name_to_check(None, never),
        Ok(UserNameToCheck::FlagOff)
    );
    assert_eq!(
        user_name_to_check(Some(OsStr::new("")), never),
        Ok(UserNameToCheck::FlagOff)
    );
    assert_eq!(
        user_name_to_check(Some(OsStr::new("1")), || Some("Wallaroo".to_owned())),
        Ok(UserNameToCheck::Check("Wallaroo".to_owned()))
    );
    assert_eq!(
        user_name_to_check(Some(OsStr::new("1")), || None),
        Ok(UserNameToCheck::NoUserName)
    );
    for typo in ["0", "true", "yes", " 1", "1 ", "11", "l"] {
        assert_eq!(
            user_name_to_check(Some(OsStr::new(typo)), never),
            Err(InvalidUserNameFlag),
            "{typo:?}"
        );
    }
}

/// 表の行の形: 値は SHA-256 の 16 進の小文字 64 桁で、重複が無く、何の綴りかが書いてある。
/// **製品名の行が無くなったら赤。実名の行があったら赤**（実名は表に持たない）。
#[test]
fn every_hashed_entry_is_well_formed() {
    assert!(
        HASHED
            .iter()
            .any(|entry| entry.kind == Kind::ThirdPartyProduct)
    );
    assert!(
        HASHED.iter().all(|entry| entry.kind != Kind::RealName),
        "実名は表に持たない（実行時に git config から読む）"
    );
    for (index, entry) in HASHED.iter().enumerate() {
        assert_eq!(entry.sha256.len(), 64, "{index}: {}", entry.note);
        assert!(
            entry
                .sha256
                .chars()
                .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch)),
            "{index}: {}",
            entry.note
        );
        if let Form::Katakana { chars } | Form::Kanji { chars } = entry.form {
            assert!(chars > 0, "{index}: {}", entry.note);
        }
        assert!(!entry.note.trim().is_empty(), "{index}");
        assert!(
            HASHED[..index]
                .iter()
                .all(|other| other.sha256 != entry.sha256),
            "{index}: 重複 {}",
            entry.note
        );
    }
}

/// 許可リストの全項目に理由がある。
#[test]
fn every_allowed_entry_states_its_reason() {
    for (path, reason) in ALLOWED {
        assert!(!reason.trim().is_empty(), "{path} の理由が空");
    }
}
