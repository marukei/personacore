//! ドメイン型が共有する文字列の正規化・検証。
//!
//! ここに置くのは**判定だけ**で、値の構築はしない。各 newtype の `parse` からのみ使う。

/// 前後から除去する空白文字。
///
/// # 写しの台帳（**作業単位 0004b-2 で数え直した**）
///
/// **Rust 側の定義はここ1箇所。SQL 側に 9 箇所の写しがある**（作業単位 12a で 2 増えた）。
/// **⚠ SQL の外にも1つある**——`web/src/lib/emphasis.ts`（作業単位 B-46 / 下の表の 7）。
/// **綴りが違うので下の `grep` には出ない**（`char(12288)` ではなく `\u3000`）。
/// `grep -rn "char(12288)" migrations src` で 10 行出る
/// （この doc のこの行と、`migrations/` の 9 行。**`src/` にはもう写しが無い**）。
///
/// **SQL の写しは「いま効いているもの」と「役目を終えたもの」に分かれる。**
/// マイグレーションは適用済みなので編集できず、古い定義もファイルには残り続ける。
/// **同期を取るべき相手は「いま効いているもの」だけ**である。
///
/// ## いま効いている写し（**広げるときはここを全部更新する**）
///
/// | # | 場所 | 何を守っているか |
/// |---|---|---|
/// | 1 | ここ（`src/domain/text.rs` の `TRIMMABLE`） | 定義。`trim_blank` / `is_invisible` が引く |
/// | 2 | `migrations/0001_initial.sql` — `stat_alloc.axis` の CHECK | 空白だけの軸名を入れさせない |
/// | 3 | `migrations/0005_*.sql` — `character.display_name` の CHECK | 空白だけの表示名を入れさせない |
/// | 4 | `migrations/0005_*.sql` — `character.handle` の trim CHECK | **冗長**（文字集合の CHECK に含まれる） |
/// | 5 | `migrations/0009_*.sql` — `character_attribute.key` の CHECK | 空白だけのキーを入れさせない |
/// | 6 | `migrations/0009_*.sql` — `character_attribute.value` の CHECK | 空白だけの値を入れさせない |
/// | 7 | `web/src/lib/emphasis.ts` の `TRIMMABLE`（**FE の手写し** / B-46） | 強調の印が**宙に浮いているか**の判定。画面から印を落とす規則の一部。**`web/src/lib/thinking.ts` も `import` して空行の判定に使う**（B-61。写しは増えていない） |
///
/// ## 役目を終えた写し（**同期の対象ではない。編集もできない**）
///
/// | 場所 | いつ効かなくなったか |
/// |---|---|
/// | `migrations/0001_initial.sql` — `character.handle` の CHECK | `0005` が `character` を作り直した |
/// | `migrations/0004_*.sql` — `display_name` の CHECK | 同上（3 が引き継いだ） |
/// | `migrations/0002_*.sql` — `character_name_key_idx` | **`0006` が索引を落とした**（0004b-2） |
/// | `migrations/0005_*.sql` — 索引の作り直し | 同上 |
///
/// ## 0004b-2 で消えた写し
///
/// `src/store/character.rs` の 3 つ——`SELECT_BY_NORMALIZED_KEY` /
/// `UPDATE_CHARACTER` / `DELETE_CHARACTER` の `trim(name, ...)`——は**もう無い。**
/// 引き当てはすべて完全一致になった（ADR-0017 /
/// `migrations/0006_drop_character_name_key_idx.sql`）。
/// **`TRIMMABLE` が識別子の照合に関わることは、もう一切ない。**
///
/// > **台帳は「8箇所」と書いてあったが、実際は 11 箇所あった。**
/// > `0004b-1` が `0005` で `character` を作り直したときに 3 つ増えたのに、
/// > 台帳が更新されなかった（「5箇所」の時期に 6 / 7 が足されたのと同じ形で**2度目**）。
/// > **数を書くだけでは守れない**ので、上のように「効いている / 終えた」で分けてある。
///
/// # ずれるとどうなるか
///
/// * **1 と 3 は広い方向（CHECK 側だけが広い）がより痛い。** `DisplayName::parse` を
///   通った表示名を DB が `CheckViolation` で弾き、`server` はそれを分類できないので
///   **理由の分からない 500** になる。
/// * **1 と 4** は識別子の側だが、4 は文字集合の CHECK（`'*[^a-z0-9_-]*'`）に
///   完全に含まれるので**単独では受理集合を変えない。** 触る理由も無い。
/// * **2 は `TRIMMABLE` 由来のテストが無い**（`stat_alloc_axis_cannot_be_blank` は
///   固定リスト）ので、**目視で合わせること。**
/// * **1 と 7 がずれると、保存した本文と画面に出る本文が食い違う。**
///   どちらの向きも同じ重さで、**画面だけが印を落とす／画面だけが印を残す**という形で出る。
///   **JavaScript の `trim()` や正規表現の空白類で代用すると必ずずれる**（あちらは U+2009 / U+FEFF も空白と認める）。
///
/// なお **Unicode の空白文字すべてを網羅しない**（U+2000 系 / U+200B / U+FEFF などは
/// trim の対象にしない）。**これは意図的で、広げてはいけない**（写しがあるため）。
/// 「見えない文字だけの値」を拒否するのは trim ではなく `is_effectively_blank` の仕事。
///
/// # 見張り番
///
/// この定数を公開しているのは、テストから各写しとの一致を機械的に確認するため。
///
/// - **1 と 3** — `tests/store_schema.rs::trimmable_set_matches_the_schema_check`
///   の順方向ループ（`handle` を正当な値で埋め、`display_name` に1文字ずつ入れて
///   CHECK に弾かせる）。`character_display_name_cannot_be_blank` は固定リストなので
///   **この検出には入らない**。逆方向（スキーマ側だけ広げる）は同テストの
///   **3文字スポットチェック**が見る。網羅ではない。
/// - **1 と 4** — 同テストの最初のループ（`TRIMMABLE` の1文字だけの `handle` を
///   INSERT して CHECK に弾かせる。**文字集合の CHECK でも落ちるので、
///   これは 4 単独の見張り番ではない**）。
/// - **1 と 7** — `web/tests/emphasis-corpus.test.ts` と `tests/emphasis_corpus.rs`。
///   **同じ1つの手書きの表**（`tests/fixtures/emphasis_corpus.tsv`）を両側が読み、
///   **`TRIMMABLE` のうち行区切りでない4点**（半角空白 / TAB / 全角空白 / NBSP）と、
///   **JavaScript だけが空白と認める2点**（U+2009 / U+FEFF）を1行ずつ持っている。
///   `**<空白>あ` の `**` は、空白と認めれば宙に浮いた印として落ち、認めなければ残る——
///   **出力が変わるので表から観測できる。**
///   **実測**（B-46 の是正 / 2026-09-17）——FE 側の `TRIMMABLE` の判定を
///   JavaScript の空白類へ寄せる変異で **2 赤**（`pnpm test`）。
/// - **2** — 無い（上記）。
/// - **5 と 6** — `tests/store_schema.rs::attribute_blank_set_matches_the_schema_check`。
///   **順方向（`TRIMMABLE` の全文字を INSERT して CHECK に弾かせる）も、
///   逆方向（3 文字のスポットチェック）も、`key` と `value` を別々の行で見る。**
///   5 と 6 は同じ CHECK 式の写しだが、**片方が捕まればもう片方も捕まる、
///   ということはない**——12a のレビューで「`value` 側だけを狭める変異」と
///   「`key` 側だけを広げる変異」を実測したところ、
///   **前者はこのテストを素通りし、後者はスイート全体を素通りした。**
///   （実測値はテスト側の doc にある。ここには写さない。）
///   固定リストの側（`attribute_key_and_value_cannot_be_blank`）が使う
///   `BLANK_STRINGS` は、**`TRIMMABLE` から導出していない手書きの別配列**。
///   導出させないままにしてあるのは、`TRIMMABLE` 自体を書き換える変異に対する
///   **独立した第二の意見**になるため。その代わり
///   **`TRIMMABLE` に文字を足しても `BLANK_STRINGS` は追随しない**ので、
///   写しの一致を見る役目は上のループが負う。
///
/// **この台帳自体は機械的には守られない**（実測: 件数だけを直してテストを足さなくても
/// 何も赤にならない）。写しを増やしたら、**ここへ書き足すのと同時に見張り番も足すこと。**
///
/// > **スポットチェックの文字は作業単位 0009a で入れ替えた。** 以前は
/// > U+2000 / U+200B / U+FEFF を使っていたが、`is_effectively_blank` を強めた結果
/// > **これらだけの値は `parse` が拒否する**ようになり、「アプリが通す値を DB が弾く」
/// > という検出したい状況を作れなくなった（この3文字については、CHECK 側が広くても
/// > アプリ側の方が更に広く弾くので実害が消えた）。
/// > 代わりに **`is_effectively_blank` が見えない側に含めない空白めいた文字**を使う。
/// > **文字は作業単位 20b でもう一度入れ替えた** —— U+3164 / U+2800 / U+115F は
/// > `MEASURED_INVISIBLE_SPELLINGS` に入ったので同じ理由で使えなくなり、
/// > いまは U+FFA0 / U+034F / U+FE00。詳細は `is_effectively_blank` の
/// > 「覆えないもの」を参照。
/// >
/// > **この入れ替えは今後も起きうる。** カテゴリの外にある綴りを測って足すたびに、
/// > このスポットチェックは1組ぶん材料を失う。**材料が尽きたら、
/// > 逆方向の検出はスポットチェックでは維持できない**と正直に書くこと。
pub const TRIMMABLE: [char; 8] = [
    ' ',        // U+0020 半角スペース
    '\u{0009}', // TAB
    '\u{000A}', // LF
    '\u{000B}', // VT
    '\u{000C}', // FF
    '\u{000D}', // CR
    '\u{3000}', // 全角空白
    '\u{00A0}', // NBSP
];

/// 行区切りとして扱う文字。
///
/// **`char::is_control` だけでは足りない。** U+2028（LINE SEPARATOR）と
/// U+2029（PARAGRAPH SEPARATOR）は Zl / Zp カテゴリで `is_control` が偽になるが、
/// 表示・トークン化の過程で改行として解釈されうる。
///
/// **`TRIMMABLE` とは別物なので統合しないこと。** あちらは U+0085 / U+2028 / U+2029 を
/// 含まず（DB の CHECK 制約と揃える必要がある）、こちらは TAB / 全角空白 / NBSP を含まない。
///
/// **この集合は「行の数え方」そのものなので、写しを作らず参照すること。**
/// 現在の利用箇所と、それを固定しているテストは以下のとおり。
///
/// 1. ここ（`src/domain/text.rs` の `LINE_BREAKS`）
/// 2. `src/prompt/safe.rs` — `SafeText::sanitize` が**改行 `\n` に正規化**する。
///    行の境界が確定していないと、`prompt::writer` の引用接頭辞が1行分しか付かず
///    偽の役割行を仕込める。
///    固定: `prompt::safe::tests::unicode_line_separators_become_newlines`（全文字ループ）
/// 3. `src/parse/mod.rs` — 台詞の中では**半角スペースへ畳み**、地の文では**行を分ける**。
///    固定: `parse::tests::every_line_separator_is_folded_inside_a_dialogue`（全文字ループ）と
///    `parse::tests::every_line_separator_splits_the_narration`（全文字ループ）
///    **同じファイルの `strip_body_emphasis` も引く**（作業単位 B-46）——
///    **強調の印を落とす前に行へ割る**ため。対は行をまたがないので、ここが痩せると
///    `**朝` と `**夜` が対になり、**行をまたいで本文が書き換わる。**
///    固定: `tests/emphasis_corpus.rs`（手書きの表。**行区切り7点と CRLF を1行ずつ持つ**）
///    **実測**（B-46 の是正 / 2026-09-17）——`strip_body_emphasis` の行割りを
///    LF 1点に痩せさせる変異で **2 赤**（`cargo test --no-fail-fast` / exit 101）。
/// 4. ここ（`breaks_the_line`）— ドメイン型の `parse` が**入口で拒否する**（作業単位 0009）。
///    1行のラベル型（`Handle` / `DisplayName` / `StatAxis` / `MemoryCategory`）は
///    含むだけで拒否する。**理由は「1行のラベルは1行である」という衛生条件**で、
///    注入防御ではない（`contains_control_or_line_break` の doc を参照）。
///    自由文の型（`Persona` / `OutputFormat` / `MemoryContent`）は複数行が正当なので
///    この検査を掛けず、**全型に `is_effectively_blank` を掛ける**（0009a）。
///    固定: 各型の `*_rejects_every_line_separator` /
///    `*_rejects_values_that_sanitize_to_nothing`（いずれも全文字ループ）と、
///    横断の見張り番 `prompt::tests::every_parsed_value_survives_sanitizing`
///
/// 5. `web/src/lib/emphasis.ts` — **FE の手写し**（作業単位 B-46）。画面から強調の印を
///    落とすときに、**Rust と同じ行の割り方**をしなければ対の作り方がずれる。
///    **⚠ ここだけは「写しを作らず参照すること」を破るしかない**——
///    別言語・別プロセスなので、この定数を引く手段が無い。
///    固定: `web/tests/emphasis-corpus.test.ts`（3 と**同じ1つの表**を読む）。
///    **実測**（B-46 の是正 / 2026-09-17）——FE 側の `LINE_BREAKS` を LF 1点に痩せさせる
///    変異で **2 赤**（`pnpm test`）。
///    **`web/src/lib/thinking.ts` もこの写しを `import` して使う**（作業単位 B-61。写しは増やしていない）。
/// 6. `src/domain/thinking.rs` — 漏れた思考を剥がした**境目の空行**を決める（作業単位 B-61 / ADR-0053）。
///    `TRIMMABLE` のうち行区切りでない4点と合わせて「空行」を定義する。
///    固定: `tests/thinking_corpus.rs` と `web/tests/thinking-corpus.test.ts`
///    （**同じ1つの手書きの表**。行区切り7点＋CRLF・空白4点・JavaScript だけが空白と認める2点を1行ずつ持つ）。
///
/// **2 と 4 がずれると**、`parse` を通った値がプロンプトで空になる
/// （0009 が狙った穴。ただし**塞ぎ切れていたのは行区切りだけ**で、
/// 見えない文字全般は 0009a の `is_effectively_blank` が受け持つ）。
///
/// **2 と 3 がずれると**、`parse` が1行として保存した台詞が `prompt` では2行に見える
/// （またはその逆）。すなわち「`dialogue` の1行 = 台詞1つ」の不変条件が、
/// **Rust から見て1行・表示上は2行**という形で静かに破れる。
/// 定義をここ1箇所に置いているので、**新しい行区切りを足すときもここだけを触ること。**
/// **⚠ 5 だけは例外で、一緒に直すしかない。** 機械的には守られない——
/// **歯止めは、両側が同じ1つの手書きの表**（`tests/fixtures/emphasis_corpus.tsv`）**を読み、
/// 食い違えばどちらかが赤くなること**だけである。
pub const LINE_BREAKS: [char; 7] = [
    '\u{000A}', // LF
    '\u{000D}', // CR
    '\u{000B}', // VT
    '\u{000C}', // FF
    '\u{0085}', // NEL
    '\u{2028}', // LINE SEPARATOR
    '\u{2029}', // PARAGRAPH SEPARATOR
];

/// **不可視の書式文字**（Unicode の `Cf` = Format カテゴリ）。**Unicode 16.0 時点の写し。**
///
/// # なぜ表を持つのか
///
/// `char::is_whitespace`（White_Space）と `char::is_control`（`Cc`）は std にあるが、
/// **`Cf` を判定する手段が std に無い。** 依存クレートを増やさない判断のため、
/// ここに範囲表を置いている。
///
/// これが覆うのは U+00AD（SOFT HYPHEN）/ U+180E（MONGOLIAN VOWEL SEPARATOR）/
/// U+200B（ZERO WIDTH SPACE）/ U+200C..U+200F（ZWNJ / ZWJ / LRM / RLM）/
/// U+202A..U+202E（双方向の埋め込み・上書き）/ U+2060（WORD JOINER）/
/// U+FEFF（BOM）/ U+E0020..U+E007F（タグ文字）など。
///
/// # 限界
///
/// * **Unicode の版に自動追随しない。** 新しい `Cf` が増えても表は古いままになる。
///   追随が必要になったら `unicode-properties` 等の導入を検討すること。
///
/// # 形の固定（作業単位 0009b）
///
/// **表そのものは 170 点あり、点ごとの見張り番は持てない。** 代わりに
/// `tests::the_invisible_format_table_keeps_its_shape` が**総符号位置数（170）と
/// 昇順・重複無し・逆転無し**を固定する。端が1点ずれれば必ず総数が変わるので落ちる。
///
/// **表は Unicode 16.0 の `Cf` と過不足なく一致する**（レビュアーが全スカラ値を
/// 列挙して確認済み。未割当の符号位置は1点も含まない。U+13430..U+1343F も
/// Unicode 16.0 では全点が `Cf` として割り当て済み）。**緩めてよいという意味ではない。**
/// 広げれば `½` や `®` を含む正当な表示名が「表示名が空です」で拒否され、
/// **利用者に回避手段が無い**（`PATCH` も同じ `parse` を通るので、弾いているのは入口そのもの）。
/// 狭めればその符号位置だけ `parse` を通り、`> - : 5` の形が復活する。
/// それぞれ `tests::visible_characters_next_to_the_table_are_not_format` と
/// `tests::every_range_of_the_invisible_format_table_is_probed` が受け持つ。
const INVISIBLE_FORMAT: [(u32, u32); 21] = [
    (0x00AD, 0x00AD),   // SOFT HYPHEN
    (0x0600, 0x0605),   // ARABIC NUMBER SIGN 〜 ARABIC NUMBER MARK ABOVE
    (0x061C, 0x061C),   // ARABIC LETTER MARK
    (0x06DD, 0x06DD),   // ARABIC END OF AYAH
    (0x070F, 0x070F),   // SYRIAC ABBREVIATION MARK
    (0x0890, 0x0891),   // ARABIC POUND / PIASTRE MARK ABOVE
    (0x08E2, 0x08E2),   // ARABIC DISPUTED END OF AYAH
    (0x180E, 0x180E),   // MONGOLIAN VOWEL SEPARATOR
    (0x200B, 0x200F),   // ZWSP / ZWNJ / ZWJ / LRM / RLM
    (0x202A, 0x202E),   // LRE / RLE / PDF / LRO / RLO
    (0x2060, 0x2064),   // WORD JOINER 〜 INVISIBLE PLUS
    (0x2066, 0x206F),   // LRI 〜 NOMINAL DIGIT SHAPES
    (0xFEFF, 0xFEFF),   // ZERO WIDTH NO-BREAK SPACE（BOM）
    (0xFFF9, 0xFFFB),   // INTERLINEAR ANNOTATION ANCHOR 〜 TERMINATOR
    (0x110BD, 0x110BD), // KAITHI NUMBER SIGN
    (0x110CD, 0x110CD), // KAITHI NUMBER SIGN ABOVE
    (0x13430, 0x1343F), // EGYPTIAN HIEROGLYPH の書式制御
    (0x1BCA0, 0x1BCA3), // SHORTHAND FORMAT の書式制御
    (0x1D173, 0x1D17A), // MUSICAL SYMBOL BEGIN BEAM 〜 END PHRASE
    (0xE0001, 0xE0001), // LANGUAGE TAG
    (0xE0020, 0xE007F), // TAG SPACE 〜 CANCEL TAG
];

/// **実測で `parse` を通っていた「見えないが文字カテゴリ」の綴り**（作業単位 20b）。
///
/// # これは防御の完成ではなく、測った穴の埋め戻し
///
/// **立場は `prompt::safe::MEASURED_CONTROL_TOKEN_SPELLINGS` と同じ。**
/// 「測ったものは塞ぐ。完全性は主張しない。」
/// カテゴリ（White_Space ∪ `Cc` ∪ `Cf`）で線を引く限り、
/// **列挙を足しても「表に無い綴り」は必ず残る**（`is_effectively_blank` の
/// 「覆えないもの」の節と `tests::the_predicate_does_not_cover_every_invisible_character`）。
///
/// # なぜ `Cf` の表（`INVISIBLE_FORMAT`）に混ぜないのか
///
/// あちらは **Unicode 16.0 の `Cf` と過不足なく一致する**という不変条件を持ち、
/// `the_invisible_format_table_keeps_its_shape` が総数 170 と符号位置の総和で固定している。
/// ここに `Lo` / `So` を1点足せば、その不変条件が壊れる。
/// **「カテゴリの写し」と「測って足した例外」は別の入れ物に置くこと。**
///
/// # 重さの見立て（**危険度は「二度と直せない」系より低い**）
///
/// * **到達できるのは運用者だけ。** 入口は `POST /admin/characters` と
///   `PUT /admin/characters/{handle}/attributes` で、**外部入力ではない。**
/// * **同じ入口で直せる**（0012a 以降。`PATCH /admin/characters/{handle}` と `PUT`）。
/// * 実害は **「モデルが `一人称: ` という欠落を演技する」**（ADR-0009）。
///   `> - : 5`（名前の見えない軸に数値だけ）と同じ形で、
///   直前の「記載のない軸は考慮しません。」と矛盾した依頼が出る。
///
/// 見張り番は `tests::every_measured_invisible_spelling_is_rejected`（直書き）と、
/// `INVISIBLE_SAMPLES` を回す各ドメイン型の `*_rejects_invisible_characters`。
const MEASURED_INVISIBLE_SPELLINGS: [char; 4] = [
    '\u{3164}', // HANGUL FILLER（Lo）— `StatAxis::parse` が `Ok` を返していた
    '\u{115F}', // HANGUL CHOSEONG FILLER（Lo）
    '\u{1160}', // HANGUL JUNGSEONG FILLER（Lo）
    '\u{2800}', // BRAILLE PATTERN BLANK（So。点が1つも無い点字）
];

/// 前後の空白を除去する。**中間の空白は保持する**（`"葵 碧"` は正当な名前）。
pub(super) fn trim_blank(raw: &str) -> &str {
    raw.trim_matches(|c| TRIMMABLE.contains(&c))
}

/// `Cf`（Format）カテゴリの文字か。`INVISIBLE_FORMAT` の表を引くだけ。
fn is_format(character: char) -> bool {
    let code = character as u32;
    INVISIBLE_FORMAT
        .iter()
        .any(|(first, last)| (*first..=*last).contains(&code))
}

/// **画面に何も現れない文字か**
/// （White_Space ∪ `Cc` ∪ `Cf` ∪ `LINE_BREAKS` ∪ `TRIMMABLE` ∪ `MEASURED_INVISIBLE_SPELLINGS`）。
///
/// `LINE_BREAKS` と `TRIMMABLE` は前3つに含まれる（現在の集合ではすべて White_Space か `Cc`）が、
/// **写しにせず参照している**のは片方向の保証を構造的に保つため。
/// どちらかの集合に「空白でも制御文字でもない文字」を足されても、判定側が自動的に追随する。
///
/// 最後の1つだけは**カテゴリではなく測った綴りの列挙**で、性質が違う
/// （`MEASURED_INVISIBLE_SPELLINGS` の doc）。**完全性は主張しない。**
fn is_invisible(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || is_format(character)
        || LINE_BREAKS.contains(&character)
        || TRIMMABLE.contains(&character)
        || MEASURED_INVISIBLE_SPELLINGS.contains(&character)
}

/// 1行を2行に割りうる文字か（制御文字 ∪ `LINE_BREAKS`）。
///
/// **`char::is_control` だけでは足りない。** U+2028 / U+2029 は Zl / Zp カテゴリで
/// `is_control` が偽だが、`prompt::SafeText::sanitize` は改行として畳む
/// （`LINE_BREAKS` の doc）。**判定を2つに分けないこと。**
fn breaks_the_line(character: char) -> bool {
    character.is_control() || LINE_BREAKS.contains(&character)
}

/// 制御文字または行区切りを含むか。**1行のラベル型が使う。**
///
/// 改行入りの値が `model` フィールド経由でログに乗ると読みにくくなるため、
/// 中間に残った制御文字は拒否する。
///
/// # この検査が守っているのは「**1行のラベルは1行である**」という衛生条件
///
/// **注入防御ではない。** `prompt::writer::quote_line` は `Seg::Text` を
/// `SafeText::to_inline()` で1行に畳んでから引用接頭辞を付けるので、
/// `"葵\u{2028}碧"` が偽の役割行になることは無い（`> 葵 碧` になるだけ）。
///
/// **この理由を消さないこと。** 書いていないと、後から
/// 「`to_inline` があるのだから要らない」として外される
/// （`0006` で `Mutex` を「効果を測れない」として外したのと同じ形）。
/// 守っているのは、**1行として保存し1行として読むと決めた型に、
/// 表示上は2行になる値を入れさせない**ことである。
/// 自由文（`Persona` / `OutputFormat` / `MemoryContent`）は複数行が正当なので、
/// そちらにはこの検査を掛けず、`is_effectively_blank` だけを掛ける。
///
/// # 「見えなくなる」方は別の検査（作業単位 0009 / 0009a）
///
/// `"\u{2028}"` **だけ**の表示名・軸名が通ってしまう欠陥は、
/// この検査ではなく `is_effectively_blank` が塞ぐ。こちらは行区切りしか見ないので、
/// U+2000（EN QUAD）や U+200B（ZWSP）だけの値は素通りする（実測で残っていた欠陥）。
pub(super) fn contains_control_or_line_break(value: &str) -> bool {
    value.chars().any(breaks_the_line)
}

/// **見えない文字しか入っていない値か**（全文字が `is_invisible`）。
///
/// # 何を保証するための述語か（作業単位 0009a で強めた）
///
/// > **`parse` を通った値は、無害化後に「見える文字」が1つ以上残る。**
///
/// **全型が使う。** 自由文（`Persona` / `OutputFormat` / `MemoryContent`）だけでなく、
/// 1行のラベル（`Handle` / `DisplayName` / `StatAxis` / `MemoryCategory`）も通す。
/// ラベル側は `contains_control_or_line_break` も併せて掛かるが、
/// **あちらは行区切りしか見ない**ので、これが無いと U+2000 だけの表示名が通る。
///
/// # 何が起きていたか
///
/// 0009 は「無害化すると空になる」を条件にしたので、
/// **空にはならないが見えない**文字が素通りしていた（レビュアーの実測）。
///
/// ```text
/// DisplayName::parse("\u{2000}") → Ok   // EN QUAD
/// StatAxis::parse("\u{3164}")    → Ok   // HANGUL FILLER
/// → プロンプト: "> \u{2000}" と "> - ㅤ: 5"
/// ```
///
/// > **どちらも今は拒否する。** 上段は 0009a（カテゴリを広げた）、
/// > 下段は 20b（`MEASURED_INVISIBLE_SPELLINGS` に足した）。
/// > **`StatAxis` / `AttributeValue` / `DisplayName` の3つとも**この行で塞がる。
///
/// `> - : 5` は 0009 が「塞いだ」と名乗っていた形そのもので、
/// **不変条件を満たしたまま欠陥だけが残っていた。** 条件を「空にならない」から
/// 「見える文字が残る」へ強めたのはこのため。
///
/// # `sanitize` との関係は**片方向**（同値ではない）
///
/// > **`prompt::SafeText::sanitize` が空にする値は、必ず `parse` が拒否する。**
/// > 逆は成り立たない（`parse` の方が厳しい）。
///
/// `sanitize` は制御文字を半角スペースに、`LINE_BREAKS` を改行に置き換えてから
/// 各行を `TRIMMABLE` で trim して空行を落とすので、それが空にする値の文字は
/// すべて `TRIMMABLE` ∪ `LINE_BREAKS` ∪ `Cc` に入る。**これは `is_invisible` の部分集合**
/// なので片方向は構造的に成り立つ。逆向きは成り立たない
/// （`sanitize("\u{200B}")` は `"\u{200B}"` のままだが、こちらは拒否する）。
///
/// 見張り番は `prompt::tests` の
/// `every_parsed_single_character_still_shows_something`（全スカラ値を**単独で**回す）と
/// `every_parsed_value_survives_sanitizing`。
///
/// # **覆えないもの**（「塞いだ」とは書けない）
///
/// **「見えない文字」を完全に列挙することはできない。** カテゴリで覆えているのは
/// **White_Space ∪ `Cc` ∪ `Cf`** までで、「見えないが文字カテゴリ」は
/// **原理的に素通りする。**
///
/// > **20b で4綴りを足した。** かつてここに並べていた
/// > `U+3164` HANGUL FILLER / `U+115F` HANGUL CHOSEONG FILLER /
/// > `U+1160` HANGUL JUNGSEONG FILLER（いずれも `Lo`）/
/// > `U+2800` BRAILLE PATTERN BLANK（`So`）は、
/// > **測った綴りとして `MEASURED_INVISIBLE_SPELLINGS` が拒否する。**
/// > **これは「塞いだ」ではない。** カテゴリの線は動いていない。
///
/// いま素通りするものの例（Default_Ignorable_Code_Point のうち
/// White_Space でも `Cc` でも `Cf` でもないもの）:
///
/// * `U+FFA0` HALFWIDTH HANGUL FILLER（`Lo`）
/// * `U+034F` COMBINING GRAPHEME JOINER（`Mn`）
/// * `U+FE00`..`U+FE0F` 異体字選択子 / `U+E0100`..`U+E01EF` 同（`Mn`）
///
/// これらは `char::is_whitespace` でも `char::is_control` でも `Cf` の表でも捕まらない。
/// カテゴリで線を引く以上ここが限界で、**列挙を足しても「表に無い綴り」は必ず残る**
/// （`prompt::safe::MEASURED_CONTROL_TOKEN_SPELLINGS` と同じ構図）。
/// この事実は `tests::the_predicate_does_not_cover_every_invisible_character` が
/// 実行可能な形で固定している。**測った側は
/// `tests::every_measured_invisible_spelling_is_rejected` が固定する。両方消さないこと。**
///
/// **空文字にも真を返す。** 呼び出し側は先に空を弾いているので、
/// この述語が拾うのは「空ではないのに何も見えない」値だけになる。
pub(super) fn is_effectively_blank(value: &str) -> bool {
    value.chars().all(is_invisible)
}

/// **見えない文字の見本。** 各ドメイン型のテストが共有して回す。
///
/// **`TRIMMABLE` に入っていない文字だけを並べてある**（`trim_blank` では落ちず、
/// `is_effectively_blank` に到達するもの）。ここに写しを作らず、
/// `*_rejects_invisible_characters` はすべてこれを参照すること。
///
/// U+2000 と U+200B は**レビュアーが実測で素通りを確認した文字**なので必ず残すこと。
///
/// **末尾の4綴りは `MEASURED_INVISIBLE_SPELLINGS` から導出せず直書きしてある**
/// （作業単位 20b）。定数から回すと、**表から1つ落とす変異が
/// この見本も道連れにして全型のテストが緑のまま**になる。
#[cfg(test)]
pub(super) const INVISIBLE_SAMPLES: [char; 12] = [
    '\u{2000}', // EN QUAD（White_Space）— 実測で `DisplayName::parse` を通っていた
    '\u{2003}', // EM SPACE（White_Space）
    '\u{205F}', // MEDIUM MATHEMATICAL SPACE（White_Space）
    '\u{1680}', // OGHAM SPACE MARK（White_Space）
    '\u{200B}', // ZERO WIDTH SPACE（Cf）
    '\u{FEFF}', // ZERO WIDTH NO-BREAK SPACE / BOM（Cf）
    '\u{00AD}', // SOFT HYPHEN（Cf）
    '\u{2060}', // WORD JOINER（Cf）
    // 以下は**カテゴリでは捕まらない**。測って足した綴り（20b）。
    '\u{3164}', // HANGUL FILLER（Lo）— 実測で `StatAxis::parse` を通っていた
    '\u{115F}', // HANGUL CHOSEONG FILLER（Lo）
    '\u{1160}', // HANGUL JUNGSEONG FILLER（Lo）
    '\u{2800}', // BRAILLE PATTERN BLANK（So）
];

#[cfg(test)]
mod tests {
    use super::*;

    /// **`TRIMMABLE` と `LINE_BREAKS` の全文字は「見えない」側に入る。**
    ///
    /// ここが破れると「`sanitize` が空にする値は `parse` が拒否する」の片方向が崩れる。
    /// 集合に文字を足した人が気付けるよう、固定リストではなく全文字を回す。
    #[test]
    fn every_shared_blank_and_line_break_is_invisible() {
        for character in TRIMMABLE.into_iter().chain(LINE_BREAKS) {
            assert!(
                is_invisible(character),
                "U+{:04X} が見える側に落ちている",
                character as u32
            );
            assert!(is_effectively_blank(&character.to_string()));
        }
    }

    /// **空白めいて見える文字は、`TRIMMABLE` に入っていなくても拒否側に落ちる。**
    ///
    /// 0009 の欠陥（`DisplayName::parse("\u{2000}") → Ok`）はここが空だったこと。
    /// **`TRIMMABLE` を広げずに判定側だけを広げた**ので、写し5箇所は無傷のまま。
    #[test]
    fn invisible_characters_outside_the_trimmable_set_are_blank() {
        for character in [
            '\u{2000}',  // EN QUAD（White_Space）
            '\u{2003}',  // EM SPACE
            '\u{2007}',  // FIGURE SPACE
            '\u{200A}',  // HAIR SPACE
            '\u{202F}',  // NARROW NO-BREAK SPACE
            '\u{205F}',  // MEDIUM MATHEMATICAL SPACE
            '\u{1680}',  // OGHAM SPACE MARK
            '\u{200B}',  // ZERO WIDTH SPACE（Cf）
            '\u{200D}',  // ZERO WIDTH JOINER（Cf）
            '\u{FEFF}',  // BOM（Cf）
            '\u{00AD}',  // SOFT HYPHEN（Cf）
            '\u{2060}',  // WORD JOINER（Cf）
            '\u{202E}',  // RIGHT-TO-LEFT OVERRIDE（Cf）
            '\u{180E}',  // MONGOLIAN VOWEL SEPARATOR（Cf）
            '\u{E0020}', // TAG SPACE（Cf）
        ] {
            assert!(
                is_invisible(character),
                "U+{:04X} を見える側に残している",
                character as u32
            );
            assert!(
                is_effectively_blank(&character.to_string()),
                "U+{:04X} だけの値を通している",
                character as u32
            );
        }
    }

    /// **`INVISIBLE_FORMAT` の「形」を固定する**（作業単位 0009b）。
    ///
    /// # なぜ表そのものを写さないのか
    ///
    /// 170 点を並べた写しを作れば「一致」は保証できるが、**写しがもう1つ増える**だけで
    /// 誤りは同時に両方へ入る。ここで固定するのは**表の形**（総数と並び）で、
    /// **端の1点ずれは必ず総数を変える**ので、狭める向きにも広げる向きにも落ちる。
    ///
    /// レビュアーの実測では、着手前は**表を9点に縮めても、レンジの端を
    /// `(0x00AD, 0x00BD)` に広げても 724 全緑**だった（170 点のうち 161 点が無防備）。
    ///
    /// # 昇順・重複無しまで見る理由
    ///
    /// `is_format` は `any` で引くので**並びは結果を変えない**（順序の入れ替えは
    /// 振る舞いとしては等価変異）。それでも固定するのは、
    /// **総数と端の検査が「表を目で追える」ことに依存している**ため。
    /// 並びが崩れた表では、重複や取りこぼしを人が読んで見つけられなくなる。
    ///
    /// # 総数だけでは足りない — **レンジの「ずれ」は総数を変えない**
    ///
    /// 総数が捕まえるのは**広げる／狭める**方向だけで、**丸ごと横へ1点ずらす**
    /// 誤り（`(0x1D173, 0x1D17A)` → `(0x1D172, 0x1D179)`）は総数も並びも変えない。
    /// 実測でこの変異は **731 全緑**を通り抜けた。Unicode のデータファイルから
    /// レンジを書き写すときに最も起きやすい形なので、
    /// **覆う符号位置の総和**（指紋）も併せて固定する。
    /// 1つのレンジを `d` だけずらせば総和は `d × そのレンジの長さ` だけ動くので、
    /// 「総数 + 総和」で**単一レンジのずれは必ず落ちる。**
    ///
    /// 総和は目で検算する値ではない。**落ちたら「表の値が動いた」と読み**、
    /// 差分を `git diff` で確かめること。
    #[test]
    fn the_invisible_format_table_keeps_its_shape() {
        // Unicode 16.0 の `Cf` は 170 点。表はこれと過不足なく一致する。
        const TOTAL_CODE_POINTS: u32 = 170;
        // 覆う符号位置の総和（`Σ code`）。**位置の指紋**で、総数と対にして使う。
        const CODE_POINT_SUM: u64 = 92_312_063;

        let mut total = 0_u32;
        let mut sum = 0_u64;
        let mut previous_last: Option<u32> = None;
        for (first, last) in INVISIBLE_FORMAT {
            assert!(
                first <= last,
                "レンジが逆転している: U+{first:04X}..U+{last:04X}"
            );
            if let Some(previous) = previous_last {
                // **隣接も禁止する**（`previous + 1 == first` なら1つに畳めるはず）。
                // 昇順であることと、同じ符号位置を2度書いていないことを同時に見る。
                assert!(
                    previous + 1 < first,
                    "レンジが昇順に並んでいないか、隣接・重複している: \
                     U+{previous:04X} の次が U+{first:04X}"
                );
            }
            previous_last = Some(last);
            total += last - first + 1;
            sum += (first..=last).map(u64::from).sum::<u64>();
        }

        assert_eq!(
            total, TOTAL_CODE_POINTS,
            "表が覆う符号位置の総数が変わった。端を広げた／狭めたならここが落ちる。\
             意図した変更なら Unicode の版と併せて doc を更新すること"
        );
        assert_eq!(
            sum, CODE_POINT_SUM,
            "表が覆う符号位置の位置が変わった（総数は同じなので、どこかのレンジが横へずれている）。\
             意図した変更なら `git diff` で動いたレンジを確かめ、この指紋を更新すること"
        );
    }

    /// **表の各レンジに代表を1点ずつ置く**（レンジを丸ごと落とす変異を捕まえる）。
    ///
    /// 着手前は**21 レンジのうち 7 つにしか代表がいなかった**ので、
    /// 残りは丸ごと消しても緑を通った。ここは固定リストで構わないが、
    /// **「どのレンジにも必ず1つ」であること**を機械的に確かめる
    /// （レンジを足した人は代表も足さないとコンパイル後に赤くなる）。
    #[test]
    fn every_range_of_the_invisible_format_table_is_probed() {
        /// 各レンジから1点ずつ。**並びは `INVISIBLE_FORMAT` と対応させること。**
        const PROBES: [char; 21] = [
            '\u{00AD}',  // SOFT HYPHEN
            '\u{0603}',  // ARABIC SIGN SAFHA
            '\u{061C}',  // ARABIC LETTER MARK
            '\u{06DD}',  // ARABIC END OF AYAH
            '\u{070F}',  // SYRIAC ABBREVIATION MARK
            '\u{0891}',  // ARABIC PIASTRE MARK ABOVE
            '\u{08E2}',  // ARABIC DISPUTED END OF AYAH
            '\u{180E}',  // MONGOLIAN VOWEL SEPARATOR
            '\u{200D}',  // ZERO WIDTH JOINER
            '\u{202E}',  // RIGHT-TO-LEFT OVERRIDE
            '\u{2064}',  // INVISIBLE PLUS
            '\u{2069}',  // POP DIRECTIONAL ISOLATE
            '\u{FEFF}',  // BOM
            '\u{FFFA}',  // INTERLINEAR ANNOTATION SEPARATOR
            '\u{110BD}', // KAITHI NUMBER SIGN
            '\u{110CD}', // KAITHI NUMBER SIGN ABOVE
            '\u{13438}', // EGYPTIAN HIEROGLYPH BEGIN SEGMENT
            '\u{1BCA2}', // SHORTHAND FORMAT UP STEP
            '\u{1D175}', // MUSICAL SYMBOL END TIE
            '\u{E0001}', // LANGUAGE TAG
            '\u{E0041}', // TAG LATIN CAPITAL LETTER A
        ];

        assert_eq!(
            PROBES.len(),
            INVISIBLE_FORMAT.len(),
            "レンジを増減させたら代表も増減させること"
        );

        for (first, last) in INVISIBLE_FORMAT {
            let inside = PROBES
                .iter()
                .filter(|probe| (first..=last).contains(&(**probe as u32)))
                .count();
            assert_eq!(
                inside, 1,
                "U+{first:04X}..U+{last:04X} の代表が {inside} 点ある（1点にすること）"
            );
        }

        for probe in PROBES {
            assert!(
                is_format(probe),
                "U+{:04X} を表から落とした（`parse` を通って画面に何も残らない値が復活する）",
                probe as u32
            );
            assert!(
                is_effectively_blank(&probe.to_string()),
                "U+{:04X} だけの値を通している",
                probe as u32
            );
        }
    }

    /// **表の隣にいる「見える文字」を、書式文字にしないこと**（広げる向きの誤り）。
    ///
    /// 表を広げると `®` や `½` を含む正当な表示名が「表示名が空です」で拒否される。
    /// **`PATCH` も同じ `parse` を通るので、利用者に回避手段が無い。**
    ///
    /// 文字は**各レンジの直前・直後**から採る（直後が未割当や空白文字の場合は、
    /// そのレンジに最も近い可視文字を採る）。端を1点広げる変異は
    /// `the_invisible_format_table_keeps_its_shape` の総数でも落ちるが、
    /// **こちらは「落ちたときに何が壊れるのか」を読める形で残す**ためにある。
    ///
    /// **結合記号（`Mn`）も混ぜてある。** 単独では基底文字の点線円に付いて描かれるので
    /// 「見えない」側ではなく、`Cf` に含めてはいけない。隣接する符号位置が
    /// 未割当でも空白でもない場合は必ず採ってあるので、**レンジがそちら側へ
    /// 1点ずれる誤りをここで捕まえられる。**
    #[test]
    fn visible_characters_next_to_the_table_are_not_format() {
        for character in [
            '\u{06DC}',  // ٜ ARABIC SMALL HIGH SEEN（U+06DD の直前）
            '\u{08E1}',  // ࣡ ARABIC SMALL HIGH SIGN SAFHA（U+08E2 の直前）
            '\u{08E3}',  // ࣣ ARABIC TURNED DAMMA BELOW（U+08E2 の直後）
            '\u{180D}',  // MONGOLIAN FREE VARIATION SELECTOR THREE（U+180E の直前）
            '\u{180F}',  // MONGOLIAN FREE VARIATION SELECTOR FOUR（U+180E の直後）
            '\u{13440}', // 𓑀 EGYPTIAN HIEROGLYPH MIRROR VERTICAL（U+1343F の直後）
            '\u{1D172}', // 𝅲 MUSICAL SYMBOL COMBINING FLAG-5（U+1D173 の直前）
            '\u{1D17B}', // 𝅻 MUSICAL SYMBOL COMBINING ACCENT（U+1D17A の直後）
            '\u{00AC}',  // ¬ NOT SIGN（U+00AD の直前）
            '\u{00AE}',  // ® REGISTERED SIGN（U+00AD の直後）
            '\u{00AF}',  // ¯ MACRON
            '\u{00BD}',  // ½ VULGAR FRACTION ONE HALF
            '\u{05F4}',  // ״ HEBREW PUNCTUATION GERSHAYIM（U+0600 の手前）
            '\u{0606}',  // ؆ ARABIC-INDIC CUBE ROOT（U+0605 の直後）
            '\u{061B}',  // ؛ ARABIC SEMICOLON（U+061C の直前）
            '\u{061D}',  // ؝ ARABIC END OF TEXT MARK（U+061C の直後）
            '\u{06DE}',  // ۞ ARABIC START OF RUB EL HIZB（U+06DD の直後）
            '\u{0710}',  // ܐ SYRIAC LETTER ALAPH（U+070F の直後）
            '\u{088E}',  // ࢎ ARABIC VERTICAL TAIL（U+0890 の手前）
            '\u{08A0}',  // ࢠ ARABIC LETTER BEH WITH SMALL V BELOW（U+0891 の後）
            '\u{2010}',  // ‐ HYPHEN（U+200F の直後）
            '\u{2030}',  // ‰ PER MILLE SIGN（U+202A の近傍。直前直後は空白文字）
            '\u{2032}',  // ′ PRIME
            '\u{2070}',  // ⁰ SUPERSCRIPT ZERO（U+206F の直後）
            '\u{2103}',  // ℃ DEGREE CELSIUS
            '\u{2116}',  // № NUMERO SIGN
            '\u{FEFC}',  // ﻼ ARABIC LIGATURE LAM WITH ALEF（U+FEFF の手前）
            '\u{FF01}',  // ！ FULLWIDTH EXCLAMATION MARK（U+FEFF の後）
            '\u{FFFC}',  // ￼ OBJECT REPLACEMENT CHARACTER（U+FFFB の直後）
            '\u{FFFD}',  // � REPLACEMENT CHARACTER
            '\u{110BC}', // 𑂼 KAITHI ENUMERATION SIGN（U+110BD の直前）
            '\u{110BE}', // 𑂾 KAITHI SECTION MARK（U+110BD の直後）
            '\u{110C2}', // 𑃂 KAITHI VOWEL SIGN VOCALIC R（U+110CD の手前）
            '\u{1342F}', // 𓐯 EGYPTIAN HIEROGLYPH V011D（U+13430 の直前）
            '\u{1BC9F}', // 𛲟 DUPLOYAN PUNCTUATION CHINOOK FULL STOP（U+1BCA0 の直前）
            '\u{1D164}', // 𝅘𝅥𝅱 MUSICAL SYMBOL SIXTY-FOURTH NOTE（U+1D173 の手前）
            '\u{1D183}', // 𝆃 MUSICAL SYMBOL ARPEGGIATO UP（U+1D17A の後）
        ] {
            assert!(
                !is_format(character),
                "U+{:04X} を書式文字に含めた（見える文字が拒否される）",
                character as u32
            );
            assert!(
                !is_invisible(character),
                "U+{:04X} を見えない側に落とした",
                character as u32
            );
            assert!(
                !is_effectively_blank(&character.to_string()),
                "U+{:04X} だけの値を「空です」で拒否している",
                character as u32
            );
        }
    }

    /// **測って `parse` を通ることを確かめた4綴りは、覆えている**（作業単位 20b）。
    ///
    /// # ここは `MEASURED_INVISIBLE_SPELLINGS` から導出しない
    ///
    /// 定数から回すと、**表から1つ落とす変異が期待値を道連れにして緑になる。**
    /// 4綴りは直書きし、`INVISIBLE_SAMPLES` も直書きしてある
    /// （`prompt::safe` の `MEASURED_CONTROL_TOKEN_SPELLINGS` の見張り番と同じ作法）。
    ///
    /// **`the_predicate_does_not_cover_every_invisible_character` と対にして読むこと。**
    /// あちらが固定しているのは「まだ覆えないものが在る」で、
    /// **こちらが固定しているのは「測ったものは覆った」。** どちらも消さない。
    #[test]
    fn every_measured_invisible_spelling_is_rejected() {
        for character in [
            '\u{3164}', // HANGUL FILLER（Lo）
            '\u{115F}', // HANGUL CHOSEONG FILLER（Lo）
            '\u{1160}', // HANGUL JUNGSEONG FILLER（Lo）
            '\u{2800}', // BRAILLE PATTERN BLANK（So。点が1つも無い点字）
        ] {
            assert!(
                is_invisible(character),
                "U+{:04X} を見える側に残している（`> - : 5` の形が復活する）",
                character as u32
            );
            assert!(
                is_effectively_blank(&character.to_string()),
                "U+{:04X} だけの値を通している",
                character as u32
            );
            // **見本の側にも必ず居ること。** 各ドメイン型の
            // `*_rejects_invisible_characters` は `INVISIBLE_SAMPLES` しか回さないので、
            // ここが抜けると**述語は正しいのに型ごとの検査だけが黙って手薄になる。**
            assert!(
                INVISIBLE_SAMPLES.contains(&character),
                "U+{:04X} が `INVISIBLE_SAMPLES` から落ちている（型ごとの検査が回らない）",
                character as u32
            );
        }
    }

    /// **覆えていないものを、実行可能な形で書き留める。**
    ///
    /// 「塞いだ」と書けないことの根拠。ここが赤になったら
    /// **覆う範囲が広がった**ということなので、`is_effectively_blank` の
    /// 「覆えないもの」の節を書き換えること（消すのではなく更新する）。
    ///
    /// > **20b で中身を入れ替えた。** 以前はここに U+3164 / U+115F / U+1160 / U+2800 が
    /// > 並んでいたが、**測ったものは塞いだ**ので
    /// > `every_measured_invisible_spelling_is_rejected` へ移った。
    /// > **主張そのものは今も正しい** —— 列挙を足しても「表に無い綴り」は必ず残る。
    /// > いま並べてあるのは Default_Ignorable_Code_Point のうち、
    /// > White_Space でも `Cc` でも `Cf` でもないもの。
    #[test]
    fn the_predicate_does_not_cover_every_invisible_character() {
        for character in [
            '\u{FFA0}',  // HALFWIDTH HANGUL FILLER（Lo）
            '\u{034F}',  // COMBINING GRAPHEME JOINER（Mn）
            '\u{FE00}',  // VARIATION SELECTOR-1（Mn）
            '\u{E0100}', // VARIATION SELECTOR-17（Mn）
        ] {
            assert!(
                !is_invisible(character),
                "U+{:04X} を覆えるようになった。doc の「覆えないもの」を更新すること",
                character as u32
            );
        }
    }

    /// 見える文字が1つでもあれば「空」ではない（弾きすぎていないことの確認）。
    #[test]
    fn a_single_visible_character_is_enough() {
        for value in [
            "葵",
            "a",
            "「",
            "🐈",
            "\u{2000}葵\u{2000}",
            "\u{200B}a",
            "#",
        ] {
            assert!(
                !is_effectively_blank(value),
                "見える文字があるのに空扱いした: {value:?}"
            );
        }
    }

    /// 空文字にも真を返す（呼び出し側は先に空を弾いている）。
    #[test]
    fn the_empty_string_counts_as_blank() {
        assert!(is_effectively_blank(""));
    }
}
