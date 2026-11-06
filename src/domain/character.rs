//! キャラクターのドメイン型。

use std::fmt;

use chrono::{DateTime, Utc};

use super::axis_candidate::AxisDiscoverySlots;
use super::drift::StatDriftMode;
use super::stat::StatScale;
use super::text::{contains_control_or_line_break, is_effectively_blank, trim_blank};

/// `character` テーブルの1行。
///
/// **読み書きは `store`、置き場所はここ**（理由は `super` の doc）。
/// 中身は検証済みのドメイン型と `String` と日時だけで、sqlx にも serde にも依存しない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    /// 主キー。`CharacterId` を得る数少ない経路のひとつ。
    pub id: CharacterId,
    /// 検証済みの識別子。`model` フィールドで指す値（ADR-0004）。
    ///
    /// **プロンプトには載らない。** 載る呼び名は `display_name`（ADR-0011）。
    pub handle: Handle,
    /// プロンプトに載る呼び名。**一意である必要はない**（ADR-0011）。
    ///
    /// `handle` と分けてあるので、`aoi` と `aoi-v2` が**どちらもプロンプト上は「葵」**
    /// として共存できる。比較実験で名前が変数にならないことがこの列の存在理由。
    pub display_name: DisplayName,
    /// ペルソナ本文。プロンプトの土台。
    pub base_persona: Persona,
    /// 台詞 / 地の文の形式指示。未設定なら節ごと出さない（`prompt`）。
    pub output_format_instruction: Option<OutputFormat>,
    /// 性格ステータスの尺度（ADR-0015）。**キャラクターごとに違ってよい。**
    ///
    /// **プロンプトには引用行1本として載る**（作業単位 0011b / `prompt::stat_scale_line`）。
    /// 「事実の記述」の側に置くので定数行にはしない。
    /// **軸ごとには載せない**（軸の件数に上限が無く、費用が青天井になるため）。
    pub stat_scale: StatScale,
    /// **変容の走らせ方**（[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md)）。
    ///
    /// **`None` は「未設定」＝起動時の既定（`PERSONACORE_STAT_DRIFT`）に従う。**
    /// 既定で埋めない——埋めると「既定に従う」と「既定と同じ値を選んだ」が区別できず、
    /// 起動時の既定を変えたときに、どのキャラクターが従うのかが消える。
    /// 効く走らせ方は `ResolvedDriftMode::resolve` が決める。
    pub stat_drift_mode: Option<StatDriftMode>,
    /// **見いだす枠の数 `m`**（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定1）。
    ///
    /// **既定 0（見いださない）。** 0 のキャラクターには軸の候補が1件も作れない。
    /// **プロンプトには載らない**（候補を出すかどうかを決めるだけ）。
    pub axis_discovery_slots: AxisDiscoverySlots,
    /// 作成日時（UTC）。
    pub created_at: DateTime<Utc>,
    /// **しまった時刻**（[ADR-0049](../../docs/adr/0049-deleting-characters-and-conversations.md) §決定1）。
    /// **`None` は「しまっていない」。**
    ///
    /// しまったキャラクターは `GET /v1/models` と画面の一覧に出ず、chat は断られる
    /// （`server::models` / `server::chat`）。**中身は1行も変わらない**ので、戻せば元どおり。
    /// **プロンプトには載らない。**
    pub archived_at: Option<DateTime<Utc>>,
}

/// 一覧の1件。**読めた行と、読めなかった行の両方**を表す（ADR-0016 §3 / 作業単位 0012b）。
///
/// # なぜ `Vec<Character>` ではないのか
///
/// `character` の列は `parse` を通してからしかドメイン型にできないので、
/// **保存済みの行が後から不正になる**ことがある（検証を厳しくした場合。0009 / 0009a で
/// 実際に起きた）。`Vec<Character>` を返す形では、そうした行の扱いが
/// 「一覧を丸ごと失敗させる」か「黙って落とす」の二択しかない。
///
/// * 丸ごと失敗 → **1件壊れただけで正常なキャラクターの一覧すら見られない。**
/// * 黙って落とす → **存在するのに見えない。** 直す相手が分からず、
///   しかも正規化キーだけは塞がれ続けるので作り直すこともできない（ADR-0004 の袋小路）。
///
/// どちらも修復の入口を塞ぐので、**行ごとに「読めた / 読めなかった」を返す。**
///
/// # **型では守れない。守っているのはテスト**
///
/// 列挙にすると1件ごとに `match` で書き分けることにはなるが、
/// **「壊れた行を応答に出す」ことを型が強制するわけではない。**
/// server の一覧を `filter_map` で `Unreadable` を落とし `unreadable: Vec::new()` にする
/// 実装は、**そのままコンパイルが通る**（0012b で実測）。
///
/// **`ValidTag`（ADR-0006）と同じ強さだと読まないこと。** あちらは辞書引き以外の
/// 構築手段が無く「表現できない」が、こちらは**表現できてしまう。**
/// 「型が守るから見張り番は要らない」と読まれた瞬間に、下のテストは外される。
///
/// 黙って落とす実装を赤にしているのは、この5本（と `/v1/models` 側の分岐）:
///
/// * `tests/store_character.rs::a_broken_row_does_not_take_the_healthy_ones_with_it`
/// * `tests/store_character.rs::every_kind_of_broken_column_is_pointed_at`
/// * `tests/server_admin.rs::a_broken_row_is_pointed_at_without_breaking_the_listings`
/// * `tests/server_admin.rs::a_row_whose_handle_cannot_be_parsed_is_pointed_at_by_id_alone`
///   （**`handle: null` の行だけを落とす実装**はこの1本しか赤にできない）
/// * `tests/server_admin.rs::a_ghost_row_no_longer_blocks_creating_the_same_handle`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterEntry {
    /// 全列が `parse` を通った行。
    Readable(Character),
    /// **1列以上が `parse` を通らなかった行。** 修復の対象（ADR-0016 §3）。
    Unreadable(UnreadableCharacter),
}

/// 読めなかった行の**手掛かり**。修復に必要な最小限だけを持つ。
///
/// # **格納値そのものは持たない**
///
/// 上限超過の値は数千文字ありうるうえ、何が入っているか分からない
/// （`StoreError::InvalidStoredValue` の `Display` に格納値を載せないのと同じ判断）。
/// 修復に要るのは「どの行か」と「どの列か」であって、壊れた値そのものではない。
/// 中身を確かめたいときは `id` で DB を直接見る。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableCharacter {
    /// 主キー。**この行を指す唯一の確実な手掛かり。**
    pub id: CharacterId,
    /// 修復 API（`Store::update_character` / `Store::delete_character`）に渡せる識別子。
    ///
    /// **格納値そのもの**（`Handle::parse` は正規化しない。ADR-0017）。
    /// 更新・削除の引き当ては**完全一致**なので、ここに値があれば必ず届く
    /// （見張り番は `tests/store_character.rs::
    /// a_row_that_parses_is_exactly_a_row_the_repair_api_can_reach`）。
    ///
    /// **`None` は「識別子そのものが `parse` を通らない行」。** その行は
    /// `Handle` を作れない以上どの API からも指せず、`id` だけが手掛かりになる。
    ///
    /// > **`None` の行に修復手段は無い**（0004b-2 で確定した。ADR-0016 §1 の帰結）。
    /// > 識別子は不変なので `update_character` では読めるようにならず、
    /// > 掃除（`delete_character`）も綴りを渡せない以上届かない。
    /// > 0012a には正規化キーで引き当てる抜け道があったが、
    /// > **その照合は `character_name_key_idx` に寄りかかっており、索引を落とすと
    /// > 本物のキャラクターまで巻き添えにする**ので一緒に捨てた
    /// > （`migrations/0006_drop_character_name_key_idx.sql`）。
    /// > **この行を直す手段は `sqlite3` だけ。**
    pub handle: Option<Handle>,
    /// 読めなかった列。**壊れ方が複数なら全部並べる**（1つ直しても読めないことが分かる）。
    ///
    /// 空にはならない（空なら読めた行として返している）。
    pub invalid_columns: Vec<CharacterColumn>,
}

/// `character` の列のうち、読み出しで `parse` を通すもの。
///
/// 文字列表現は**DB の列名と同じ**にしてある（手掛かりから DB を直接見に行けるように）。
/// 綴りが実在の列名からずれると、応答を見た運用者が
/// `sqlite3` で `SELECT <その名前> FROM character` を打って `no such column` で止まる
/// ——**指し示す先が存在しない**という、最も避けたい壊れ方になる。
/// 見張り番は `tests/store_schema.rs::every_pointed_at_column_is_a_real_character_column`
/// （`PRAGMA table_info(character)` と突き合わせるので、列名を変えるマイグレーションにも効く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CharacterColumn {
    /// 識別子。**`parse` を通るかどうかだけで決まる**（0004b-2）。
    ///
    /// `Handle::parse` は正規化しない（ADR-0017）ので、`parse` を通った値は
    /// **格納値そのもの**であり、完全一致の照合に必ず当たる。
    /// 逆に `" aoi "` や `"Aoi"` の行は `parse` を通らず、`model` からも
    /// 修復 API からも**決して引けない**（手掛かりは id だけ）。
    Handle,
    /// プロンプトに載る呼び名。
    DisplayName,
    /// ペルソナ本文。
    BasePersona,
    /// 台詞 / 地の文の形式指示。**NULL は「未設定」であって不正ではない。**
    OutputFormatInstruction,
    /// 性格ステータスの尺度（ADR-0015）。
    ///
    /// **NOT NULL かつスキーマ側にも CHECK があるので、普通は壊れない。**
    /// それでも指し示せるようにしてあるのは、CHECK が
    /// `PRAGMA ignore_check_constraints` で外せる（`tests/common` の抜け穴と同じ）ため。
    /// 範囲外の値が入った行を「読めない行」として一覧に出せないと、
    /// **その行だけ 500 になって修復の入口が消える。**
    StatScale,
    /// **変容の走らせ方**（ADR-0045）。**NULL は「未設定」であって不正ではない。**
    ///
    /// スキーマに CHECK（`off` / `shadow` / `on`）があるので普通は壊れないが、
    /// `StatScale` と同じ理由（CHECK は外せる）で指し示せるようにしてある。
    StatDriftMode,
    /// **見いだす枠の数**（ADR-0046 §決定1）。
    ///
    /// NOT NULL かつスキーマに CHECK（整数・0〜4）があるので普通は壊れないが、
    /// `StatScale` と同じ理由（CHECK は外せる）で指し示せるようにしてある。
    AxisDiscoverySlots,
    /// **しまった時刻**（ADR-0049 §決定1）。**NULL は「しまっていない」であって不正ではない。**
    ///
    /// スキーマの CHECK は型（TEXT）しか縛らないので、日時として読めない値は入りうる。
    /// **読めない値を「しまっていない」に落とさない** —— 落とすと、しまったはずのキャラクターが
    /// 黙って `GET /v1/models` に戻ってくる。読めない行として指し示す（直し方は `PATCH` で戻す）。
    ArchivedAt,
}

impl CharacterColumn {
    /// **全 variant。** 列名の突き合わせ（上の見張り番）が「全部」を回るために要る。
    ///
    /// 数え漏らしは `every_variant_is_listed_in_all` が止める
    /// （variant を足すとあのテストの `match` がコンパイルできなくなる）。
    pub const ALL: [Self; 8] = [
        Self::Handle,
        Self::DisplayName,
        Self::BasePersona,
        Self::OutputFormatInstruction,
        Self::StatScale,
        Self::StatDriftMode,
        Self::AxisDiscoverySlots,
        Self::ArchivedAt,
    ];

    /// DB の列名。ログと API 応答に出す。
    ///
    /// **`Display` は生やさない。** 同じ文字列を返す経路が2本あると、片方だけが
    /// 綴りを変えられても気付けない（`Display` には使い手が無かった / 0012b で削除）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Handle => "handle",
            Self::DisplayName => "display_name",
            Self::BasePersona => "base_persona",
            Self::OutputFormatInstruction => "output_format_instruction",
            Self::StatScale => "stat_scale",
            Self::StatDriftMode => "stat_drift_mode",
            Self::AxisDiscoverySlots => "axis_discovery_slots",
            Self::ArchivedAt => "archived_at",
        }
    }
}

/// 更新できる列だけを並べた差分（ADR-0016 / 作業単位 0012a）。
///
/// # **識別子（`handle`）が無いのは仕様**
///
/// `character.handle` は **不変**（ADR-0016 §1）。`model` フィールドで指す値なので、
/// 変えると **FE 側に保存された `aoi@42` が指さなくなる。**
/// 「渡さない約束」ではなく**渡せない形**にしてあるので、`store` の更新 API に
/// 新しい識別子を届ける経路は型として存在しない。
/// 呼び名を変えたいだけなら `display_name` で足りる（ADR-0011）。
///
/// # 省略した列は変わらない
///
/// `Default` は「1列も変えない」。`..CharacterUpdate::default()` で埋めた列は
/// **既定値で上書きされるのではなく、触られない。**
///
/// ```
/// use personacore::domain::{CharacterUpdate, DisplayName, FieldUpdate};
///
/// let only_the_display_name = CharacterUpdate {
///     display_name: Some(DisplayName::parse("葵").expect("正当な呼び名")),
///     ..CharacterUpdate::default()
/// };
/// assert_eq!(
///     only_the_display_name.output_format_instruction,
///     FieldUpdate::Keep,
///     "触れていない列は「変えない」"
/// );
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterUpdate {
    /// プロンプトに載る呼び名。`None` なら変えない。
    pub display_name: Option<DisplayName>,
    /// ペルソナ本文。`None` なら変えない。
    pub base_persona: Option<Persona>,
    /// 出力形式指示。**NULL になりうる列なので3値**（`FieldUpdate` の doc を参照）。
    pub output_format_instruction: FieldUpdate<OutputFormat>,
    /// 性格ステータスの尺度（ADR-0015）。`None` なら変えない。
    ///
    /// **`FieldUpdate` にしない。** 列が NOT NULL なので「消す」に対応する状態が無く、
    /// 3値にすると `Clear` が SQL に落とせない値になる（`display_name` / `base_persona`
    /// と同じ理由）。
    pub stat_scale: Option<StatScale>,
    /// 変容の走らせ方（ADR-0045 §決定3）。**NULL になりうる列なので3値**
    /// （`Clear` は「未設定＝起動時の既定に従う」へ戻す）。
    pub stat_drift_mode: FieldUpdate<StatDriftMode>,
    /// 見いだす枠の数（ADR-0046 §決定1）。`None` なら変えない。
    ///
    /// **`FieldUpdate` にしない**（`stat_scale` と同じ理由。列が NOT NULL なので「消す」が無い）。
    pub axis_discovery_slots: Option<AxisDiscoverySlots>,
    /// **しまう／戻す**（ADR-0049 §決定1）。`None` なら変えない。
    ///
    /// | 値 | 意味 |
    /// |---|---|
    /// | `None` | 変えない |
    /// | `Some(true)` | しまう。**すでにしまってあれば、最初の時刻のまま**（しまい直しで「いつ」を失わない） |
    /// | `Some(false)` | 戻す（しまった時刻を消す） |
    ///
    /// **時刻そのものは渡さない。** 書くのは `store` の SQL（`strftime`）で、
    /// 呼び出し側が任意の時刻を書ける口は作らない。
    pub archived: Option<bool>,
}

/// **NULL になりうる列の部分更新**を表す3値。
///
/// # なぜ `Option<Option<T>>` にしないか
///
/// 「変えない」「設定する」「消す」の3つは**別の意味**だが、`Option<T>` の1段では
/// 2つしか表せない。無理に載せると **`None` が「触れない」と「消す」の両方**を指し、
/// 表示名だけを変える更新のたびに**形式指示が黙って消える。**
/// `Option<Option<T>>` は3値を表せるが、`Some(None)` と `None` は**見た目が近すぎて
/// 呼び出し側が取り違える**（しかも取り違えても型検査を通る）。名前で区別する。
///
/// 消えた形式指示は `prompt` が節ごと落とすだけなので、
/// **エラーにも警告にもならず「なんとなく前と違う」としか観測できない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldUpdate<T> {
    /// 現在の値をそのまま残す（既定）。
    Keep,
    /// 指定の値に置き換える。
    Set(T),
    /// 未設定（NULL）にする。
    Clear,
}

// **`derive(Default)` を使わない**（clippy::derivable_impls の提案には従えない）。
// 派生は型引数に `T: Default` を要求するが、`OutputFormat` は検証を通らないと作れないので
// `Default` を持たない（持たせない）。派生にすると `CharacterUpdate::default()` が
// 「`OutputFormat: Default` を満たさない」として通らなくなる。
#[expect(
    clippy::derivable_impls,
    reason = "派生は T: Default を要求する。OutputFormat は Default を持たない"
)]
impl<T> Default for FieldUpdate<T> {
    fn default() -> Self {
        Self::Keep
    }
}

/// `character` テーブルの主キー。
///
/// **`i64` の裸のままにしない。** 近い将来 `session.id` / `message.id` / `memory.id` が
/// 加わると同型の識別子が並び、`store.stat_allocs(session_id)` のような取り違えが
/// 型検査を素通りする。外部キーは「存在しない id」しか弾けず、
/// `session.id = 3` と `character.id = 3` が**両方存在する状況（ほぼ常に成立）では
/// 静かに別キャラのデータを返す**。
///
/// `Handle` と同じく **`From<i64>` も `pub fn new` も生やさない**。
/// 得る手段は `Store::create_character` の戻り値と `Character::id` だけで、
/// DB からの復元経路は `pub(crate)` に閉じてある。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharacterId(i64);

impl CharacterId {
    /// DB から復元する内部経路。**crate 外からは呼べない。**
    pub(crate) const fn from_row(raw: i64) -> Self {
        Self(raw)
    }

    /// 生の主キー値。SQL へのバインド値はこれを使う。
    ///
    /// **取り出しは危険ではない**（危険なのは検証されていない値から作れること）ため公開する。
    pub const fn get(self) -> i64 {
        self.0
    }

    /// **テスト専用の抜け穴。** 「存在しない id」を作る用途にのみ使う。
    ///
    /// `Store::pool` と同じ位置づけで、`testing` フィーチャ（と crate 内テスト）でしか
    /// 存在しない。本番ビルドには含まれないので、未検証の id が `store` に届く経路は増えない。
    #[cfg(any(test, feature = "testing"))]
    pub const fn from_raw_for_tests(raw: i64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for CharacterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 検証を通ったキャラクター**識別子**（`character.handle` の列そのもの）。
///
/// **`Handle::parse` が唯一のコンストラクタ**で、`From<String>` も `new` も生やさない
/// （ADR-0006 の `ValidTag` と同じ発想）。これにより「検証を通っていない文字列が
/// DB や `model` の照合に届く」経路が型レベルで存在しなくなる。
///
/// # 使えるのは `ALLOWED_CHARACTERS` の 38 文字だけ（ADR-0017 / 作業単位 0004b-1）
///
/// **正規化はしない。保持するのは入力そのまま。** 前後の空白は trim せず、
/// 大文字も畳み込まず、どちらも**拒否する。**
///
/// > **識別子に残った仕事は「安定した鍵であること」だけ。**
/// > 呼び名は `display_name` が持ち（ADR-0011）、識別子は不変（ADR-0016）。
/// > 意味を載せる必要が無くなったので、打てる範囲まで文字集合を絞ってある。
///
/// 空白を**表現できなくした**ので、`"aoi"` と `" aoi "` が別キャラとして併存する事故
/// （ADR-0004 のゴースト行）は原理的に起きない。同じ理由で `"aoi"` と `"aoi\u{200B}"` の
/// 併存（0009 の残余）も消える。**「防ぐ」のではなく「表現できなくする」側に寄せてある。**
///
/// > 照合（`model` からの引き当て）は**この値で完全一致**させる。
///
/// # 長すぎる識別子を拒否する理由
///
/// 識別子も**削れない枠**（docs/03-data-model.md「上限を持つ型にする」）に載る自由文で、
/// 上限が無ければペルソナとまったく同じ壊れ方をする。加えて識別子には固有の事情がある。
///
/// * **`model` フィールドに載る**（ADR-0004）。`GET /v1/models` の一覧はクライアントの
///   モデル選択 UI にそのまま並ぶので、極端に長い識別子は**利用者側の UI を壊す。**
/// * `/admin/characters/{handle}/stats` のパスにも載る。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(String);

impl Handle {
    /// キャラクター識別子の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 50 文字か
    ///
    /// 実在の識別子は `aoi` や `aoi-v2` のように十数文字に収まる
    /// （日本語は書けない。使えるのは `ALLOWED_CHARACTERS` の 38 文字だけ / ADR-0017）。
    /// 50 文字はその3倍以上あり**正当な識別子を弾かない**一方、
    /// 貼り付け事故（本文を識別子欄に入れる）は確実に止まる。
    ///
    /// 削れない枠に載る分は最悪 50 バイト。**識別子は ASCII しか含めない**ので
    /// （`ALLOWED_CHARACTERS` / ADR-0017）1文字1バイトで、
    /// `DisplayName` のように「すべてアストラル面の文字で4倍」になる場合が**存在しない**。
    /// 既定の予算 7,168 に対して十分に小さい。全項目を上限いっぱいにしても
    /// 収まることは `prompt` 側の
    /// `every_field_at_its_limit_still_fits_the_default_budget` が固定している。
    ///
    /// **文字数で数える理由は、この型だけ他と違う。**
    /// 他の型（`DisplayName` / `MemoryContent` 等）は「UTF-8 では日本語1文字が3バイトで、
    /// 日本語だけが 1/3 しか書けなくなる」から文字数にしている。
    /// **識別子は ASCII しか受け付けないので、文字数とバイト数は常に一致する**（ADR-0017）。
    /// **他と揃えてあるだけで、ここでは選択の余地が無い。**
    /// 見張り番は `a_handle_has_the_same_length_in_characters_and_bytes`。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 書き込みの検証だけでなく、**読み出し**（`store` の `character_entry`）でも
    /// 同じ `parse` を通す。下げると保存済みの長い識別子の行は**読めない行**に落ち、
    /// しかも `Handle` を作れないので `find_character_by_handle` で引くことも
    /// 更新・削除も作り直しもできなくなる（一覧が id で指し示すだけで、
    /// 復旧手段は DB の直接編集だけ）。上げる方向は安全。
    pub const MAX_CHARS: usize = 50;

    /// **識別子に使える文字の全体**（ADR-0017 / 作業単位 0004b-1）。
    ///
    /// ASCII の小文字英字・数字・`-`・`_` だけ。**大文字も含めない。**
    /// 畳み込み（`Aoi` → `aoi`）は正規化であり、正規化がある限り
    /// 「畳み込む前の行」が存在しうるため（ADR-0017「却下: 大文字を小文字へ畳み込む」）。
    ///
    /// # 写しは3箇所。**増やさないこと**
    ///
    /// 1. ここ（`Handle::ALLOWED_CHARACTERS`）
    /// 2. `migrations/0005_character_handle_alphabet.sql` — `character.handle` の CHECK
    ///    （GLOB の否定クラス `'*[^a-z0-9_-]*'`）と、移行前の検査
    /// 3. `web/src/lib/CharacterForm.svelte` — 識別子の欄の説明文（作業単位 D-14）。
    ///    **判定には使わない、読ませるだけの写し。** 描かれた説明から文字の集合を読んで
    ///    ここと突き合わせる見張り番は `web/tests/character-form-examples.test.ts`。
    ///
    /// **2 は範囲記法（`a-z0-9_-`）で、1 は列挙**。綴りをわざと変えてあるので、
    /// 片方の範囲指定を書き損なえばもう片方と食い違う。
    /// ずれの検出は `tests/store_schema.rs::handle_alphabet_matches_the_schema_check`
    /// が**両方向**（アプリが通す値を DB も通す / アプリが弾く値を DB も弾く）で見ている。
    ///
    /// `TRIMMABLE` が8箇所に増えた経緯があるので、**4箇所目を作らないこと。**
    /// エラーメッセージにも列挙しない（`HandleError::Charset` の doc）。
    pub const ALLOWED_CHARACTERS: &'static str = "abcdefghijklmnopqrstuvwxyz0123456789-_";

    /// 識別子に使える文字か。`ALLOWED_CHARACTERS` を引くだけ。
    ///
    /// **判定を別の式で書き直さないこと**（`character.is_ascii_lowercase() || …` は
    /// `ALLOWED_CHARACTERS` の4箇所目の写しになる）。
    #[must_use]
    pub fn is_allowed_char(character: char) -> bool {
        Self::ALLOWED_CHARACTERS.contains(character)
    }

    /// 唯一のコンストラクタ。**正規化しない**（ADR-0017 / 作業単位 0004b-1）。
    ///
    /// 1. 空なら拒否する。
    /// 2. **見える文字が1つも無いなら拒否する**（作業単位 0009a）。
    /// 3. `@` を含むなら拒否する（`model` の予約語 `aoi@new` と衝突するため）。
    /// 4. 制御文字・行区切りを含むなら拒否する。
    /// 5. `MAX_CHARS` を超えるなら拒否する。
    /// 6. `ALLOWED_CHARACTERS` の外の文字を含むなら拒否する。
    ///
    /// # **`trim` しない**（0009 以前と真逆）
    ///
    /// `" aoi "` は `"aoi"` に直さず**拒否する。** 空白は識別子として表現できないので、
    /// 「前後に空白を付けた別の行」＝ゴースト行が**原理的に作れなくなる**
    /// （ADR-0004 が「最も危険」とした、照合では一致するのに完全一致では引けない行）。
    /// **直す代わりに拒否するのが要点。** 直す実装（正規化）が残っている限り、
    /// 「直す前に入った行」の問題は消えない。
    ///
    /// # 6 を最後に置く理由
    ///
    /// 2〜5 はすべて 6 の部分集合なので、順序を変えても**受理する集合は変わらない。**
    /// 変わるのは**返る理由**だけで、貼り付け事故（ペルソナ本文を識別子欄に入れる）に
    /// 「使えない文字が含まれています」と返すより「長すぎます」と返す方が送り直しに直結する。
    /// 同じ理由で、改行入りには `Control`、`@` には `AtSign` を返す。
    pub fn parse(raw: &str) -> Result<Self, HandleError> {
        if raw.is_empty() {
            return Err(HandleError::Empty);
        }
        // **見えない文字だけの識別子は「空」として拒否する**（作業単位 0009a）。
        // 文字集合の検査でも弾けるが、`"\u{200B}"` に「使えない文字」と返すより
        // 「空です」と返す方が打ち手には分かりやすい。
        if is_effectively_blank(raw) {
            return Err(HandleError::Empty);
        }
        if raw.contains('@') {
            return Err(HandleError::AtSign);
        }
        if contains_control_or_line_break(raw) {
            return Err(HandleError::Control);
        }
        if raw.chars().count() > Self::MAX_CHARS {
            return Err(HandleError::TooLong);
        }
        if !raw.chars().all(Self::is_allowed_char) {
            return Err(HandleError::Charset);
        }

        Ok(Self(raw.to_owned()))
    }

    /// 検証済みの文字列。SQL へのバインド値はこれを使う。
    ///
    /// **入力そのまま**（正規化していないので、`parse` に渡した文字列と必ず一致する）。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 検証・正規化を通った**表示名**（プロンプトに載る呼び名 / ADR-0011）。
///
/// **`DisplayName::parse` が唯一のコンストラクタ**で、`From<String>` も `new` も生やさない
/// （`Handle` と同じ発想）。
///
/// # なぜ `Handle` と別の型なのか
///
/// 役割が違う。`Handle` は**識別子**（一意・`model` で指す・`@` 禁止）で、
/// こちらは**呼び名**（一意でなくてよい・`model` に現れない）。
/// 同じ `String` の newtype にすると、`create_character(name, display_name, …)` の
/// 引数を取り違えても型検査を素通りし、**プロンプトに識別子が載る**という
/// この列を足した意味そのものを打ち消す壊れ方をする（`StatAxis` と `Handle` を
/// 分けたのと同じ理由）。
///
/// # `@` を禁止しない
///
/// `Handle` が `@` を禁じているのは `model` の予約語（`aoi@new`）と
/// 衝突するからで（ADR-0004）、表示名は `model` に現れない。**禁じる理由が無い。**
/// 逆に制御文字は拒否する。`Persona` と違い、これは1行の短いラベルだからで、
/// 改行が正当な自由文ではない。
///
/// # 長すぎる呼び名を拒否する理由
///
/// 表示名は**削れない枠**（docs/03-data-model.md「上限を持つ型にする」）に載る。
/// 上限が無ければ、巨大な表示名を1件入れただけでそのキャラクターの以後の全チャットが
/// `PromptError::BudgetExhausted` になる（`Persona` / `OutputFormat` とまったく同じ穴）。
///
/// **それでも入口で止める理由は `Persona` の doc の 0012a の注記が権威**
/// （このファイルで写しを増やさないこと。0012a より前は3箇所で食い違っていた）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DisplayName(String);

impl DisplayName {
    /// 表示名の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ `Handle::MAX_CHARS` と同じ 50 文字か
    ///
    /// 呼び名も識別子も「実在の名前」であり、単位が同じ（「葵」「葵 碧」のように
    /// 日本語で数文字、英語でも十数文字）。50 文字はその3倍以上ある。
    ///
    /// **識別子より短くしてはいけない。** `display_name` を省略した作成要求は
    /// `handle` の値で埋める（ADR-0011 / 0004a）ので、こちらが短いと
    /// **識別子としては正当なのに作成が失敗する**名前が生まれる。
    /// この関係は `every_valid_handle_is_also_a_valid_display_name` が固定している。
    ///
    /// 削れない枠に載る分は最悪 200 バイト（すべてアストラル面の文字）で、
    /// 全項目を上限いっぱいにしても既定の予算に収まることは `prompt` 側の
    /// `every_field_at_its_limit_still_fits_the_default_budget` が固定している。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// `Persona::MAX_CHARS` と同じ。**書き込みの検証**と**読み出しの検証**
    /// （`StoreError::InvalidStoredValue`）の両方に使われるので、下げると
    /// 保存済みの長い表示名を持つキャラクターが即座に引けなくなる。
    ///
    /// > **0012a / 0012b より前は「DB の直接編集以外に復旧手段が無い」と書いてあった。**
    /// > いまは `GET /admin/characters` の `unreadable` が壊れた行を指し示し、
    /// > **`PATCH` は読み出しを通らずに直せる**
    /// > （`tests/store_character.rs::an_update_repairs_a_row_that_cannot_be_read`）。
    /// > **下げてよいという意味ではない** —— 直すのは人で、しかも1件ずつ。
    pub const MAX_CHARS: usize = 50;

    /// 唯一のコンストラクタ。**正規化してから検証する**（順序を入れ替えないこと）。
    ///
    /// 1. 前後の空白（`text::TRIMMABLE`）を除去する。中間の空白は保持する。
    /// 2. 空になったら拒否する（空だと `# 名前` の下が空の見出しになる）。
    /// 3. **見える文字が1つも残らないなら拒否する**（作業単位 0009a）。
    /// 4. 制御文字・行区切りを含むなら拒否する。
    /// 5. `MAX_CHARS` を超えるなら拒否する。
    ///
    /// **`@` は拒否しない**（型の doc を参照）。
    pub fn parse(raw: &str) -> Result<Self, DisplayNameError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(DisplayNameError::Empty);
        }
        // **見えない文字だけの呼び名も「空」として拒否する**（作業単位 0009a）。
        // 0009 は行区切りしか見ておらず、`DisplayName::parse("\u{2000}")` が
        // 通って `# 名前` の下に**見えない呼び名の引用行**が載っていた（実測）。
        if is_effectively_blank(normalized) {
            return Err(DisplayNameError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(DisplayNameError::Control);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 上限ちょうどの表示名が弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(DisplayNameError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 表示名が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
///
/// **`HandleError` と文言を揃えない。** 揃えると、応答を見た人が
/// 「識別子と表示名のどちらが悪いのか」を判断できない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DisplayNameError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("表示名が空です")]
    Empty,

    /// 制御文字を含む。
    #[error("表示名に改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `DisplayName::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("表示名が長すぎます")]
    TooLong,
}

/// 検証・正規化を通ったペルソナ本文。
///
/// **`parse` が唯一のコンストラクタ**（`Handle` / `MemoryContent` と同じ発想）。
/// `String` のままにしておくと、**検証がどこにも無い列**になる。
///
/// # 空文字を拒否する理由
///
/// `prompt` はペルソナが空なら**節ごと落とす**ので、空のまま作れると
/// 「ペルソナの無いキャラクター」が黙って成立する。
/// `base_persona` の綴りを間違えた投入スクリプトは、**気付かれないまま
/// ペルソナの無いキャラクターを増やす**（`/admin/` から見ても空欄と区別が付かない）。
/// 作成時に弾けば、送り直せばよいだけの失敗で済む。
///
/// > **0012a より前は「作り直しの手段が無い」と書いてあった。**
/// > いまは `PATCH /admin/characters/{handle}` と
/// > `DELETE /admin/characters/{handle}` があるので**直せる。**
/// > 拒否する理由は「復旧できないから」ではなく「黙って成立するから」に変わった。
///
/// 前後の空白は除去し、**中間の空白・改行は保持する**（ペルソナは複数段落の散文）。
/// **制御文字は拒否しない**（`Handle` との非対称は意図したもの。
/// 識別子は照合キー・短いラベルだが、こちらは自由文で改行が正当）。
/// プロンプトへの整形は `prompt::SafeText` の責務。
///
/// # 長すぎる本文を拒否する理由
///
/// **`MemoryContent` とまったく同じ構造の穴が、`character` 側には空いていた。**
/// ペルソナは**削れない枠**（docs/02-architecture.md のコンテキスト予算）に載るので、
/// 1MB のペルソナを持つキャラクターは以後のすべてのチャットが
/// `PromptError::BudgetExhausted` になる。記憶と違って削られもしない。
///
/// > **0012a より前は「アプリ経由では二度と直せない」と書いてあった。**
/// > いまは `PATCH /admin/characters/{handle}` で直せる（`MAX_CHARS` の doc も参照。
/// > **同じファイルの離れた2箇所で食い違っていた**ので、まとめて訂正した）。
/// > それでも入口で止める——**気付くのは「そのキャラクターと話せなくなってから」**で、
/// > `BudgetExhausted` からペルソナの長さへ辿り着ける人は限られる。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Persona(String);

impl Persona {
    /// ペルソナ本文の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 600 文字か（**1000 から下げた。ADR-0009 / 作業単位 12b**）
    ///
    /// **ペルソナ単独では既定の予算を使い切れない**ように選んである。
    /// 見積もりは UTF-8 の 1 バイト 1 トークン（`prompt::estimate_tokens`）なので、
    /// 最悪ケース（すべてアストラル面の文字 = 4 バイト）でも 2,400 トークン。
    /// 既定のプロンプト取り分は 8,192 − 1,024 = 7,168 トークンなので、
    /// **上限いっぱいのペルソナでも他の枠が残る**（日本語なら約 1,800 トークン）。
    /// この関係は `prompt` 側の
    /// `a_persona_at_the_limit_still_fits_the_default_budget` が固定している。
    ///
    /// **下げた理由は「場所が無いから」ではなく「役割が縮んだから」。**
    /// 不変属性（性別・年齢・種族・一人称）が `character_attribute` へ移り、
    /// `base_persona` は本来の用途（口調・振る舞いの散文）に戻る。
    /// **数字の出どころは
    /// [ADR-0009](../../docs/adr/0009-character-attributes-and-facts.md) の
    /// 「上限の数字と、その出どころ」節。ここには写さない。**
    ///
    /// 記憶（`MemoryContent::MAX_CHARS` = 500）より長いのは、**単位が違う**から。
    /// 記憶は「猫が好き」のような事実1件で、ペルソナはキャラクター定義の全文。
    /// ペルソナが予算を食えば、その分だけ記憶と履歴が削られる
    /// （壊れはしないが「覚えていない」形で効く）。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値を下げると保存済みの行が読めなくなる**
    ///
    /// `MemoryContent::MAX_CHARS` と同じ。この定数は**書き込みの検証**と
    /// **読み出しの検証**（`StoreError::InvalidStoredValue`）の両方に使われる。
    /// 1000 → 600 に下げたので、**保存済みの 601 文字以上のペルソナを持つ行は
    /// `CharacterEntry::Unreadable` になり**、1件を指す読み出しは止まる。
    ///
    /// **`PATCH /admin/characters/{handle}` で直せる**（0012a 以降。それ以前は
    /// 直せなかった）。更新は既存の行を読まないので届く。見張り番は
    /// `tests/server_admin.rs::a_persona_over_the_lowered_limit_is_unreadable_until_patched`。
    ///
    /// **着手時点の影響はゼロ**（実測）: `scripts/characters.sample.json` は
    /// 79 / 55 文字、作業 DB のキャラクターは 0 件。
    /// 上げる方向は安全。
    pub const MAX_CHARS: usize = 600;

    /// 唯一のコンストラクタ。**正規化してから検証する**（順序を入れ替えないこと）。
    pub fn parse(raw: &str) -> Result<Self, PersonaError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(PersonaError::Empty);
        }
        // **「広い意味で空白だけ」も空として拒否する**（作業単位 0009）。
        // 行区切りを丸ごと拒否はできない（複数段落の散文は正当）が、
        // 全部が空白相当なら `prompt::SafeText::sanitize` の後に何も残らず、
        // **ペルソナの無いキャラクターが黙って成立する。**
        if is_effectively_blank(normalized) {
            return Err(PersonaError::Empty);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 上限ちょうどのペルソナが弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(PersonaError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Persona {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// ペルソナ本文が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PersonaError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("ペルソナ本文が空です")]
    Empty,

    /// 正規化後の文字数が `Persona::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("ペルソナ本文が長すぎます")]
    TooLong,
}

/// 検証・正規化を通った出力形式指示。
///
/// **`parse` が唯一のコンストラクタ**（`Persona` / `MemoryContent` と同じ発想）。
/// `Option<String>` のままにしておくと、**検証がどこにも無い列**になる。
///
/// # なぜ `String` では駄目だったか
///
/// `Persona` → `MemoryContent` → ここ、と**同じ構造の穴が3回**出た。
/// 「削れない枠に載る自由文は必ず上限を持つ型にする」という規則
/// （docs/03-data-model.md）の3つ目の実例で、個別対応をやめるために型を置いている。
///
/// # 長すぎる指示を拒否する理由
///
/// 形式指示は**削れない枠**に載る。1MB の形式指示を持つキャラクターは
/// 以後のすべてのチャットが `PromptError::BudgetExhausted` になる（`Persona` と同じ）。
///
/// > **0012a より前は「アプリ経由では二度と直せない」と書いてあった。**
/// > いまは `PATCH /admin/characters/{handle}` で直せる（`MAX_CHARS` の doc も参照）。
/// > 入口で止める理由は「復旧できないから」ではなく、
/// > **気付くのが「そのキャラクターと話せなくなってから」だから。**
///
/// **加えてここには固有の危険がある。** 形式指示は、プロンプト上で
/// 「指示として従え」と宣言される**唯一の節**（`prompt::FORMAT_NOTE` / ADR-0007）。
/// 上限が無いと、そこは**面積無制限の指示チャネル**になる。
/// 現在この列に書けるのは運用者だけだが、docs/03-data-model.md が警告しているとおり
/// **書き込み経路が増えた瞬間に注入経路になる。** 面積は最初から絞っておく。
///
/// 前後の空白は除去し、**中間の空白・改行は保持する**（複数行の指示は正当）。
/// **制御文字は拒否しない**（`Persona` と同じ扱い。無害化は `prompt::SafeText` の責務）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputFormat(String);

impl OutputFormat {
    /// 形式指示の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 150 文字か
    ///
    /// 形式指示は「台詞は「」で囲み、それ以外は地の文として書いてください。」のような
    /// **1〜2文**（`scripts/characters.sample.json` の実物は 27 文字）。
    /// 150 文字はその5倍で、複数行に分けた指示も収まる。
    ///
    /// **ペルソナ（`Persona::MAX_CHARS` = 600 文字）と同じ桁にしない。** ADR-0007 は
    /// 「形式指示のトークンコストがほぼゼロ」であることを採用の決め手のひとつに
    /// 挙げており、桁を揃えるとその前提そのものが崩れる。
    /// 加えて、ここは「指示として従え」と宣言される唯一の節なので、
    /// **書ける面積を広げること自体が設計上の代償**になる（型の doc 参照）。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// `Persona::MAX_CHARS` と同じ。**書き込みの検証**と**読み出しの検証**
    /// （`StoreError::InvalidStoredValue`）の両方に使われるので、下げると
    /// 保存済みの長い形式指示を持つキャラクターが即座に引けなくなる。
    ///
    /// **復旧手段はある**——`PATCH /admin/characters/{handle}`（0012a）は
    /// 既存の行を読まないので届く（`Persona::MAX_CHARS` の doc と同じ事情。
    /// 見張り番は `tests/server_admin.rs::
    /// a_persona_over_the_lowered_limit_is_unreadable_until_patched`）。
    /// **下げてよい理由にはならない**（気付くのは「読めなくなってから」）。
    /// 上げる方向は安全（ただし `prompt` の予算テストを必ず確認すること）。
    pub const MAX_CHARS: usize = 150;

    /// 唯一のコンストラクタ。**正規化してから検証する**（順序を入れ替えないこと）。
    ///
    /// **空を拒否する。** 空の形式指示は `prompt` が節ごと落とすので、作れてしまうと
    /// 「設定したのに効かない」が黙って成立する。未設定は `Option::None` で表す
    /// （`Some("")` と `None` の2通りの「未設定」を作らない）。
    pub fn parse(raw: &str) -> Result<Self, OutputFormatError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(OutputFormatError::Empty);
        }
        // **「広い意味で空白だけ」も空として拒否する**（作業単位 0009）。
        // 全部が空白相当なら無害化の後に何も残らず、`prompt` が節ごと落とすので
        // 「設定したのに効かない」が黙って成立する（`Empty` を拒否するのと同じ理由）。
        if is_effectively_blank(normalized) {
            return Err(OutputFormatError::Empty);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 上限ちょうどの指示が弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(OutputFormatError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 出力形式指示が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputFormatError {
    /// 正規化すると空になる（空文字・空白のみ）。未設定は `None` で表すこと。
    #[error("出力形式指示が空です")]
    Empty,

    /// 正規化後の文字数が `OutputFormat::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("出力形式指示が長すぎます")]
    TooLong,
}

/// キャラクター**識別子**が受け付けられない理由。
///
/// `Display` は**定数文字列**にする（エラーレスポンスに載りうるため。CLAUDE.md 絶対制約）。
/// 入力そのものを埋め込まないので、値の反射による情報漏れも起きない。
///
/// # 文言は「名前」ではなく「識別子」で書く（作業単位 0004b-3）
///
/// 列も型も `handle` に揃えたのに文言だけ「キャラクター名」のままだと、
/// **受け取った側は `handle` と `display_name` のどちらを直せばよいのか読めない**
/// （作成要求には両方が載る）。`DisplayNameError` は「表示名」で書くこと。
/// 見張り番は `tests/server_admin.rs::invalid_handles_are_rejected_with_distinguishable_reasons`
/// （応答本文を完全一致で固定している）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HandleError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("キャラクター識別子が空です")]
    Empty,

    /// `@` を含む。`model` フィールドの予約語と衝突する（ADR-0004）。
    #[error("キャラクター識別子に @ は使えません")]
    AtSign,

    /// 制御文字を含む。
    #[error("キャラクター識別子に改行や制御文字は使えません")]
    Control,

    /// 文字数が `Handle::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("キャラクター識別子が長すぎます")]
    TooLong,

    /// **`Handle::ALLOWED_CHARACTERS` の外の文字を含む**（ADR-0017 / 作業単位 0004b-1）。
    ///
    /// 大文字・空白・記号・日本語・不可視文字はすべてここに落ちる。
    ///
    /// **`Control` / `AtSign` と統合しないこと。** どちらもこの規則の部分集合になったが、
    /// 「改行が混ざっている」「`@` を使った」は打ち手が原因を掴みやすい特別な失敗で、
    /// 混ぜると「使えない文字が含まれています」としか言えなくなる。
    ///
    /// **使える文字をメッセージに列挙しない。** `Display` は定数文字列に保つ規約はあるが、
    /// ここは実行時の値を埋めなければ列挙してよい。それでも書かないのは、
    /// 列挙が `ALLOWED_CHARACTERS` の**9つ目の写し**になるため
    /// （`TRIMMABLE` が8箇所に増えた経緯と同じ轍を踏まない）。
    #[error("キャラクター識別子に使えない文字が含まれています")]
    Charset,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    /// 正規化すると空になる文字列。`migrations/0001_initial.sql` の CHECK が弾く集合と対応する。
    const BLANKS: [&str; 12] = [
        "",
        " ",
        "   ",
        "\t",                                  // U+0009 TAB
        "\n",                                  // U+000A LF
        "\u{000B}",                            // VT
        "\u{000C}",                            // FF
        "\r",                                  // U+000D CR
        "\u{3000}",                            // 全角空白
        "\u{00A0}",                            // NBSP
        " \u{3000}\t\r\n",                     // 混在
        "\u{000B}\u{000C}\u{00A0}\u{3000} \t", // 混在（順序違い）
    ];

    fn parsed(raw: &str) -> Handle {
        Handle::parse(raw).expect("正当なキャラクター名として解釈できること")
    }

    /// `CharacterId` は生の値を往復させる（`store` のバインド値として使うため）。
    #[test]
    fn character_id_round_trips_the_raw_key() {
        for raw in [1_i64, 42, i64::MAX] {
            assert_eq!(CharacterId::from_row(raw).get(), raw);
        }
        assert_eq!(CharacterId::from_row(42).to_string(), "42");
    }

    /// 値が違えば別の id。同じなら等しい（HashMap のキーに使えること）。
    #[test]
    fn character_id_compares_by_value() {
        assert_eq!(CharacterId::from_row(3), CharacterId::from_row(3));
        assert_ne!(CharacterId::from_row(3), CharacterId::from_row(4));
        assert!(CharacterId::from_row(3) < CharacterId::from_row(4));
    }

    /// テスト専用の抜け穴も同じ表現を作る（`store` に渡せる id であること）。
    ///
    /// **本番ビルドにはこの関数も `Store::pool` も存在しない**（`testing` フィーチャで
    /// ゲートしてある）。裸の `i64` から `CharacterId` を作れる経路は本番には無い。
    #[test]
    fn test_only_constructor_matches_the_internal_one() {
        assert_eq!(
            CharacterId::from_raw_for_tests(9_999),
            CharacterId::from_row(9_999)
        );
    }

    /// **識別子は正規化されない。** 前後の空白は trim されず**拒否される**
    /// （ADR-0017 / 作業単位 0004b-1。**0009 以前と真逆の挙動**）。
    ///
    /// # なぜ「直す」ではなく「拒否する」なのか
    ///
    /// trim して受理する実装（0009 以前）は、`" aoi "` を `"aoi"` に直してくれる代わりに
    /// **「直す前に入った行」の問題を残す。** DB を直接編集された `" aoi "` の行は
    /// 完全一致では引けず、正規化キーだけを塞ぎ続ける（ADR-0004 のゴースト行）。
    /// **空白を表現できなくすれば、その行そのものが作れない。**
    ///
    /// ここが緑のまま `trim` が復活すると、ADR-0017 が消したはずの欠陥クラスが丸ごと戻る。
    #[test]
    fn the_handle_is_never_normalized() {
        // **trim 後なら正当**になる値を並べる（trim が復活したら `Ok("aoi")` になる）。
        for raw in [
            " aoi ",
            "   aoi",
            "aoi   ",
            "\taoi\n",
            "\u{00A0}aoi\u{00A0}",
            "\u{000B}\u{000C}aoi\r",
            " \u{3000}\t\r\naoi\u{00A0} ",
        ] {
            let parsed = Handle::parse(raw);
            assert!(
                parsed.is_err(),
                "前後に空白のある識別子を受理した（trim が復活している）: raw = {raw:?}"
            );
            assert_ne!(
                parsed.as_ref().map(Handle::as_str),
                Ok("aoi"),
                "trim して `aoi` に直してはいけない: raw = {raw:?}"
            );
        }

        // **大文字も畳み込まない**（畳み込みも正規化）。
        for raw in ["Aoi", "AOI", "aoI"] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Charset),
                "大文字を受理・畳み込みしないこと: raw = {raw:?}"
            );
        }

        // 受理した値は**入力そのまま**（何ひとつ書き換えない）。
        for raw in ["aoi", "a-b_c9", "0", "_"] {
            assert_eq!(parsed(raw).as_str(), raw, "raw = {raw:?}");
        }
    }

    /// **`ALLOWED_CHARACTERS` の外の文字は1つ残らず拒否される**（ADR-0017）。
    ///
    /// # 固定リストにしないこと
    ///
    /// 代表を並べたリストでは「大文字だけ通す」「`.` だけ通す」といった漏れを検出できない。
    /// **全スカラー値を回す**（`prompt::tests::every_parsed_single_character_still_shows_something`
    /// と同じ手口）。
    ///
    /// # 期待値を `is_allowed_char` から作らないこと
    ///
    /// `parse` と同じ関数から期待値を作ると、**その関数を変異させても両辺が一緒に動いて緑**になる。
    /// ここでは許す集合をテスト側で**別の綴り**（範囲の列挙）で書き下ろす。
    /// `ALLOWED_CHARACTERS` を1文字でも増減させれば必ず落ちる。
    #[test]
    fn every_character_outside_the_alphabet_is_rejected() {
        // **`ALLOWED_CHARACTERS` の写しではない別の綴り。** 片方だけを触れば食い違う。
        let allowed = |character: char| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        };

        let mut accepted = 0_u32;
        for code in 0..=0x0010_FFFF_u32 {
            let Some(character) = char::from_u32(code) else {
                continue;
            };

            // 単独の名前。
            let alone = Handle::parse(&character.to_string());
            assert_eq!(
                alone.is_ok(),
                allowed(character),
                "U+{code:04X} 単独: 期待 = {}, 実際 = {alone:?}",
                allowed(character)
            );

            // **正当な文字に挟んだ形も見る。** 単独だけだと「1文字目しか見ない」実装を通す。
            let embedded = Handle::parse(&format!("a{character}b"));
            assert_eq!(
                embedded.is_ok(),
                allowed(character),
                "U+{code:04X} を中間に含む名前: 期待 = {}, 実際 = {embedded:?}",
                allowed(character)
            );

            if allowed(character) {
                accepted += 1;
            }
        }

        assert_eq!(
            accepted, 38,
            "受理する文字はちょうど 38 種（小文字 26 + 数字 10 + `-` + `_`）であること"
        );
        assert_eq!(
            Handle::ALLOWED_CHARACTERS.chars().count(),
            38,
            "定数の側も 38 文字であること（重複を書いていないこと）"
        );
    }

    /// 使える文字は**すべて**単独でも連結でも通る（弾きすぎていないことの確認）。
    #[test]
    fn every_character_of_the_alphabet_is_accepted() {
        for character in Handle::ALLOWED_CHARACTERS.chars() {
            assert_eq!(
                parsed(&character.to_string()).as_str(),
                character.to_string(),
                "U+{:04X} 単独が通らない",
                character as u32
            );
        }
        let all_of_them: String = Handle::ALLOWED_CHARACTERS.chars().collect();
        assert_eq!(
            parsed(&all_of_them).as_str(),
            all_of_them,
            "全文字を並べた識別子も通ること"
        );
        // 実運用中の識別子（ADR-0017「結果」）。
        for raw in ["aoi", "s10", "s100", "nar", "nost", "aoi-v2", "aoi_2"] {
            assert_eq!(parsed(raw).as_str(), raw, "raw = {raw:?}");
        }
    }

    /// 値が違えば別の識別子。同じなら等しい。
    #[test]
    fn distinct_spellings_are_distinct_handles() {
        assert_eq!(parsed("aoi"), parsed("aoi"));
        assert_ne!(parsed("aoi"), parsed("rin"));
        assert_ne!(parsed("aoi-v2"), parsed("aoiv2"));
        assert_ne!(parsed("aoi-v2"), parsed("aoi_v2"));
    }

    /// 正規化後が空なら拒否する。
    #[test]
    fn rejects_blank() {
        for raw in BLANKS {
            assert_eq!(Handle::parse(raw), Err(HandleError::Empty), "raw = {raw:?}");
        }
    }

    /// `@` は `model` フィールドの予約語（`aoi@new`）と衝突するため拒否する（ADR-0004）。
    #[test]
    fn rejects_at_sign() {
        for raw in ["a@b", "@", "@aoi", "aoi@", "aoi@new", "a@o@i", "葵@42"] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::AtSign),
                "raw = {raw:?}"
            );
        }
    }

    /// 制御文字を含むなら拒否する。前後の空白として除去されなかった分が対象。
    #[test]
    fn rejects_control_characters() {
        for raw in [
            "a\u{0007}b",   // BEL
            "葵\u{0000}碧", // NUL
            "a\nb",         // 中間の LF
            "a\tb",         // 中間の TAB
            "a\u{0085}b",   // NEL（trim 対象外の制御文字）
            "aoi\u{007F}",  // DEL
            " a\rb ",       // 前後を trim しても中間に残る
        ] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Control),
                "raw = {raw:?}"
            );
        }
    }

    /// **行区切りを含むなら拒否する**（作業単位 0009）。
    ///
    /// `char::is_control` は U+2028 / U+2029 を**偽**と判定する（Zl / Zp カテゴリ）ので、
    /// 制御文字の検査だけでは素通りする。**`LINE_BREAKS` の全文字を回す**
    /// （固定リストにすると、集合に文字を足した人が気付けない）。
    ///
    /// # 守っているのは「**1行のラベルは1行である**」という衛生条件
    ///
    /// **注入防御ではない。** `prompt::writer::quote_line` は `Seg::Text` を
    /// `to_inline()` で1行に畳むので、`"葵\u{2028}碧"` が偽の役割行になることは無く
    /// `> 葵 碧` になるだけ。この理由を消すと、後から
    /// 「`to_inline` があるのだから要らない」として外される
    /// （`0006` の `Mutex` と同じ形）。
    ///
    /// 「見えなくなる」方は `handle_rejects_invisible_characters` の担当。
    #[test]
    fn handle_rejects_every_line_separator() {
        for separator in LINE_BREAKS {
            assert_eq!(
                Handle::parse(&format!("a{separator}b")),
                Err(HandleError::Control),
                "separator = {separator:?}"
            );
            assert!(
                Handle::parse(&separator.to_string()).is_err(),
                "行区切りだけの名前も作れないこと: separator = {separator:?}"
            );
        }
    }

    /// **何も見えない値は `Control` ではなく `Empty` として報告する**（作業単位 0009b）。
    ///
    /// # 何を固定しているのか
    ///
    /// 0009a で `is_effectively_blank` を `contains_control_or_line_break` の**前**に
    /// 置いた結果、入力とエラーの対応が変わった。**コードの集合は不変**だが、
    /// どの入力がどのコードになるかは動いている。
    ///
    /// ```text
    /// "\u{0007}"    制御文字だけ        Control → Empty
    /// "\u{2028}"    行区切りだけ        Control → Empty
    /// "\u{0007}a"   見える文字と混在    Control のまま
    /// ```
    ///
    /// **意図した対応。** 何も見えない値に「改行や制御文字は使えません」と返すより、
    /// 「空です」と返す方が、送り直しの判断に直結する。
    ///
    /// # なぜ専用の見張り番が要るのか
    ///
    /// `handle_rejects_every_line_separator` は単独の場合を `is_err()` としか見ておらず、
    /// **2つの検査の順序を戻す変異が緑を通る**（レビュアーの実測）。
    /// ここが順序を固定する唯一の見張り番。
    ///
    /// なお `TRIMMABLE` に入っている行区切り（LF / CR / VT / FF）は
    /// `trim_blank` の時点で消えて先頭の `is_empty` に落ちるので、順序を区別するのは
    /// **U+0085 / U+2028 / U+2029 と、trim されない制御文字**の側。
    #[test]
    fn handle_reports_empty_rather_than_control_when_nothing_is_visible() {
        for raw in ["\u{0007}", "\u{0000}", "\u{007F}", "\u{0007}\u{0000}"] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Empty),
                "何も見えない値は「空」として報告すること: raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                Handle::parse(&separator.to_string()),
                Err(HandleError::Empty),
                "行区切りだけの値も「空」として報告すること: separator = {separator:?}"
            );
        }
        // **見える文字が混じったら `Control` のまま。** 逆向きの固定。
        for raw in ["\u{0007}a", "a\u{0007}", "葵\u{2028}碧"] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Control),
                "見える文字があるなら制御文字として報告すること: raw = {raw:?}"
            );
        }
    }

    /// **見えない文字だけの名前は作れない**（作業単位 0009a）。
    ///
    /// 0009 は「無害化すると空になる」を条件にしたので、**空にはならないが見えない**
    /// 文字（U+2000 / U+200B など）が素通りしていた。
    ///
    /// **`text::INVISIBLE_SAMPLES` を参照する**（写しを作らない）。
    ///
    /// 0004b-1 以降は文字集合の検査でも弾けるが、**返す理由は `Empty` のまま**にしてある
    /// （`"\u{200B}"` に「使えない文字」と返すより「空です」の方が送り直しに直結する）。
    /// 併せて **`"aoi\u{200B}"` が `"aoi"` と併存できた 0009 の残余**が消えたことも見る。
    #[test]
    fn handle_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                Handle::parse(&character.to_string()),
                Err(HandleError::Empty),
                "見えない文字だけの名前が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                Handle::parse(&format!(" {character}\u{3000}{character}\t")),
                Err(HandleError::Empty),
                "空白と混ぜても空: U+{:04X}",
                character as u32
            );
            // **0009 の残余。** 見える文字が混じっていれば通っていたので、
            // `aoi` と `aoi\u{200B}` が別キャラとして併存できた（ADR-0017「背景」）。
            assert_eq!(
                Handle::parse(&format!("aoi{character}")),
                Err(HandleError::Charset),
                "不可視文字を混ぜた識別子が通った: U+{:04X}",
                character as u32
            );
        }
    }

    /// **検証の順序を固定する**（どの理由で拒否するかが変わるので明示的に見る）。
    ///
    /// 受理する集合はどの順でも同じ（`Charset` が他をすべて含む）。ここが守るのは
    /// **返る理由**で、打ち手の次の一手が変わる。
    #[test]
    fn the_reason_for_rejection_is_the_most_specific_one() {
        // `@` は文字集合の外でもあるが、`AtSign` として報告する（ADR-0004 の予約語）。
        assert_eq!(Handle::parse("@"), Err(HandleError::AtSign));
        assert_eq!(Handle::parse("aoi@new"), Err(HandleError::AtSign));
        // 制御文字・行区切りも文字集合の外だが、`Control` として報告する。
        assert_eq!(Handle::parse("a\nb"), Err(HandleError::Control));
        assert_eq!(Handle::parse("aoi\u{2028}"), Err(HandleError::Control));
        // 見えない文字しか無ければ `Empty`。
        assert_eq!(Handle::parse("\t"), Err(HandleError::Empty));
        assert_eq!(Handle::parse("\r\n"), Err(HandleError::Empty));
        assert_eq!(Handle::parse("\u{200B}"), Err(HandleError::Empty));
        // 上限超過は、使えない文字で埋まっていても `TooLong`
        // （貼り付け事故に「使えない文字」と返しても送り直しの役に立たない）。
        assert_eq!(
            Handle::parse(&"あ".repeat(Handle::MAX_CHARS + 1)),
            Err(HandleError::TooLong)
        );
        // どれにも当たらなければ `Charset`。
        assert_eq!(Handle::parse("葵"), Err(HandleError::Charset));
        assert_eq!(Handle::parse(" aoi "), Err(HandleError::Charset));
        assert_eq!(Handle::parse("aoi.v2"), Err(HandleError::Charset));
    }

    /// エラーは理由を区別できる（空 / `@` / 制御文字 / 長すぎる / 使えない文字）。
    #[test]
    fn error_variants_are_distinguishable() {
        let variants = [
            HandleError::Empty,
            HandleError::AtSign,
            HandleError::Control,
            HandleError::TooLong,
            HandleError::Charset,
        ];

        for (index, left) in variants.iter().enumerate() {
            assert!(
                !left.to_string().is_empty(),
                "理由を説明するメッセージがあること"
            );
            for right in &variants[index + 1..] {
                assert_ne!(left, right);
                assert_ne!(
                    left.to_string(),
                    right.to_string(),
                    "文言でも区別できること"
                );
            }
        }
    }

    /// **識別子のエラーは「識別子」と名乗り、表示名のエラーは「表示名」と名乗る**
    /// （作業単位 0004b-3）。
    ///
    /// # 区別できるだけでは足りない
    ///
    /// 上の `error_variants_are_distinguishable` は**互いに違うこと**しか見ないので、
    /// 5つまとめて「キャラクター名…」に戻す変異は**緑を通る**（0004b-3 で実測）。
    /// 列も型も `handle` に揃えた後にその文言が返ると、応答を受け取った側は
    /// **`handle` と `display_name` のどちらを直せばよいのか読めない**
    /// （作成要求には両方が載る）。
    ///
    /// **両方向を見る。** 片側だけだと、2つとも同じ語に寄せる変更が通ってしまう。
    ///
    /// 応答に載る側は別の写し（`server::error::ApiError::invalid_character_handle`）で、
    /// そちらは `tests/server_admin.rs::invalid_handles_are_rejected_with_distinguishable_reasons`
    /// が本文の完全一致で固定している。**ここが見ているのは `Display` の側。**
    #[test]
    fn the_two_kinds_of_name_never_share_a_word() {
        for reason in [
            HandleError::Empty,
            HandleError::AtSign,
            HandleError::Control,
            HandleError::TooLong,
            HandleError::Charset,
        ] {
            let message = reason.to_string();
            assert!(
                message.starts_with("キャラクター識別子"),
                "識別子のエラーだと名乗ること: {reason:?} => {message}"
            );
            assert!(
                !message.contains("表示名"),
                "表示名の語を混ぜないこと: {reason:?} => {message}"
            );
        }

        for reason in [
            DisplayNameError::Empty,
            DisplayNameError::Control,
            DisplayNameError::TooLong,
        ] {
            let message = reason.to_string();
            assert!(
                message.starts_with("表示名"),
                "表示名のエラーだと名乗ること: {reason:?} => {message}"
            );
            assert!(
                !message.contains("識別子"),
                "識別子の語を混ぜないこと: {reason:?} => {message}"
            );
        }
    }

    /// `Display` は検証済みの値をそのまま返す（SQL へのバインド値と一致する）。
    #[test]
    fn display_matches_as_str() {
        let name = parsed("aoi-v2");
        assert_eq!(name.to_string(), "aoi-v2");
        assert_eq!(name.to_string(), name.as_str());
    }

    /// **貼り付け事故は名前でも入口で止まる。**
    ///
    /// 名前は `model` フィールドに載る（ADR-0004）ので、極端に長い名前は
    /// `GET /v1/models` を通じて**クライアントのモデル選択 UI を壊す。**
    #[test]
    fn handle_rejects_a_pasted_document() {
        assert_eq!(
            Handle::parse(&"あ".repeat(1_000)),
            Err(HandleError::TooLong)
        );
        let persona_text = "静かな図書館の司書。物腰は落ち着いていて、".repeat(10);
        assert_eq!(
            Handle::parse(&persona_text),
            Err(HandleError::TooLong),
            "ペルソナ本文を名前欄に貼っても止まること"
        );
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    ///
    /// **文字集合の中の文字で数えること。** 規則外の文字（`"あ"`）で試すと
    /// `Charset` でも `TooLong` でも赤にならず、上限そのものを見張れない。
    ///
    /// # **数え方も1行だけ見ておく**（0004b-1 のレビュー指摘）
    ///
    /// `raw.chars().count()` を `raw.len()`（バイト数）に変えても、
    /// **このスイートは全部緑だった。** 受理する集合が同じ（ASCII だけなので
    /// 文字数とバイト数が一致する）ためで、**等価変異ではあるが返る理由は変わる。**
    /// 多バイト文字を並べた入力に `Charset` ではなく `TooLong` が返るようになり、
    /// 「使えない文字が含まれています」と言うべき場面で「長すぎます」と答える。
    /// **順序（6 を最後に置く理由）を守っていることを1行で見張る。**
    #[test]
    fn handle_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "a".repeat(Handle::MAX_CHARS);
        assert_eq!(
            Handle::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(Handle::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        assert_eq!(
            Handle::parse(&"a".repeat(Handle::MAX_CHARS + 1)),
            Err(HandleError::TooLong),
            "上限を1文字超えたら拒否すること"
        );

        // **文字数で数えること。** バイト数で数えると、20 文字（60 バイト）の
        // この入力が上限判定に先に引っ掛かり、`Charset` ではなく `TooLong` が返る。
        assert_eq!(
            Handle::parse(&"あ".repeat(20)),
            Err(HandleError::Charset),
            "上限はバイト数ではなく文字数で数えること（返る理由が入れ替わらないこと）"
        );
    }

    /// **文字数とバイト数は識別子では常に一致する**（文字集合が ASCII だけなので / ADR-0017）。
    ///
    /// 0004b-1 より前は「バイト数で数えると日本語だけ 1/3 しか書けない」ことを
    /// `name_length_is_counted_in_characters_not_bytes` が見張っていたが、
    /// **その状況はもう作れない**（多バイト文字が1つでもあれば `Charset` で落ちる）。
    /// **見張り番を黙って消さず、成り立たなくなった事実の方を書き留める。**
    ///
    /// 数え方の見張り番が要るのは `DisplayName` / `Persona` / `MemoryContent` の側で、
    /// そちらは今も日本語を受ける（それぞれ `*_length_is_counted_in_characters_not_bytes`）。
    ///
    /// # **`"a".repeat(50)` で確かめない**（0004b-1 のレビュー指摘）
    ///
    /// 以前はそう書いていたが、`a` は1バイト文字なので**この主張は無条件に真**で、
    /// 何を守っているのか分からない状態だった。
    /// 守りたいのは「`ALLOWED_CHARACTERS` に多バイト文字が紛れ込んでいないこと」なので、
    /// **文字集合そのものを回す。** これなら例えば全角の `０` や `–`（en dash）を
    /// 混ぜた瞬間に赤くなる（`is_allowed_char` は `contains` なので**通ってしまう**）。
    #[test]
    fn a_handle_has_the_same_length_in_characters_and_bytes() {
        // **文字集合を1文字ずつ回す。** 前提として、集合が空だと何も確かめられない。
        let alphabet = Handle::ALLOWED_CHARACTERS;
        assert!(
            alphabet.chars().count() >= 30,
            "前提: 文字集合が空でないこと: {alphabet:?}"
        );

        for character in alphabet.chars() {
            assert_eq!(
                character.len_utf8(),
                1,
                "識別子に使える文字は1バイトであること: {character:?}"
            );
        }

        // 文字集合を全部並べた名前でも、バイト数と文字数は一致する。
        let all_of_them: String = alphabet.chars().collect();
        let name = parsed(&all_of_them);
        assert_eq!(
            name.as_str().len(),
            name.as_str().chars().count(),
            "識別子は ASCII だけなのでバイト数と文字数が一致すること"
        );

        // 多バイト文字は長さに関係なく拒否される（1文字でも）。
        assert_eq!(Handle::parse("葵"), Err(HandleError::Charset));
        assert_eq!(Handle::parse("aoi🐈"), Err(HandleError::Charset));
        assert_eq!(Handle::parse("\u{20BB7}"), Err(HandleError::Charset));
    }

    /// 上限は「短いラベル」として妥当な範囲に収まっている。
    ///
    /// **下げる方向の変更は保存済みの行を一覧から消す**（`MAX_CHARS` の doc）ので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn handle_limit_stays_in_the_intended_range() {
        assert!(
            (20..=100).contains(&Handle::MAX_CHARS),
            "上限は短いラベルに見合う長さであること: {}",
            Handle::MAX_CHARS
        );
        // 名前はペルソナ本文より短いこと（単位が違う）。
        const {
            assert!(Handle::MAX_CHARS < Persona::MAX_CHARS);
        }
    }

    /// **エラーの `Display` に名前も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn handle_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(Handle::MAX_CHARS);
        let err = Handle::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "名前を含めないこと: {message}");
        assert!(
            !message.contains(&Handle::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// **見える記号・絵文字・全角英字も拒否する**（ADR-0017）。
    ///
    /// 0004b-1 より前は「制御文字でなければ通す」だったので、ここに並ぶ値はすべて
    /// 正当な識別子だった。**逆向きになったことを明示的に固定する。**
    #[test]
    fn visible_symbols_outside_the_alphabet_are_rejected() {
        for raw in ["葵", "🐈", "Ａｏｉ", "#1", "aoi v2", "aoi.v2", "aoi/v2"] {
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Charset),
                "raw = {raw:?}"
            );
        }
    }

    // ------------------------------------------------------------------
    // ペルソナ本文
    // ------------------------------------------------------------------

    fn persona(raw: &str) -> Persona {
        Persona::parse(raw).expect("正当なペルソナとして解釈できること")
    }

    /// 前後の空白は除去する。
    #[test]
    fn persona_trims_surrounding_blanks() {
        for (raw, expected) in [
            (" 静かな司書。 ", "静かな司書。"),
            ("\u{3000}静かな司書。\u{3000}", "静かな司書。"),
            ("\n静かな司書。\n", "静かな司書。"),
        ] {
            assert_eq!(persona(raw).as_str(), expected, "raw = {raw:?}");
        }
    }

    /// **中間の改行・空白は保持する。** ペルソナは複数段落の散文でありうる。
    #[test]
    fn persona_keeps_inner_blanks_and_newlines() {
        for raw in ["静かな司書。\n\n読書と紅茶が好き。", "a\tb", "葵 碧の姉"] {
            assert_eq!(persona(raw).as_str(), raw, "raw = {raw:?}");
        }
    }

    /// **制御文字は拒否しない**（キャラクター名との非対称は意図したもの）。
    ///
    /// 名前は照合キー・短いラベルだが、ペルソナは自由文で改行が正当。
    /// プロンプトへの整形（制御文字の無害化）は `prompt::SafeText` の責務。
    #[test]
    fn persona_does_not_reject_control_characters() {
        for raw in ["1行目\n2行目", "a\u{0007}b", "a\tb"] {
            assert!(Persona::parse(raw).is_ok(), "raw = {raw:?}");
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::Control),
                "名前は同じ値を拒否すること: raw = {raw:?}"
            );
        }
    }

    /// **空のペルソナは作れない。**
    ///
    /// `prompt` は空のペルソナの節を丸ごと落とすので、作れてしまうと
    /// 「ペルソナ無しのキャラクター」が**黙って成立する。**
    /// 綴り間違い（`base_persona` を空で送る）は**作成時に弾く**——
    /// 拒否する理由は「復旧できないから」ではなく「黙って成立するから」
    /// （`Persona` の doc の 0012a の注記）。
    #[test]
    fn persona_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                Persona::parse(raw),
                Err(PersonaError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// **「広い意味で空白」だけのペルソナは作れない**（作業単位 0009）。
    ///
    /// 空だけを拒否していると `"\u{2028}"` のペルソナが `parse` を通り、
    /// `prompt::SafeText::sanitize` がそれを空にするので**ペルソナが丸ごと消える。**
    /// 節ごと落ちるぶん見た目は静かで、**ペルソナの無いキャラクターが黙って成立する**
    /// （`Persona` の doc の 0012a の注記——直せるかどうかではなく、気付けないことが問題）。
    ///
    /// **中間の行区切りは正当**（複数段落の散文）なので、そちらは通すこと。
    #[test]
    fn persona_rejects_values_that_sanitize_to_nothing() {
        for separator in LINE_BREAKS {
            assert_eq!(
                Persona::parse(&separator.to_string()),
                Err(PersonaError::Empty),
                "separator = {separator:?}"
            );
            assert_eq!(
                Persona::parse(&format!("\u{3000}{separator} {separator}\t")),
                Err(PersonaError::Empty),
                "空白と混ぜても空: separator = {separator:?}"
            );
            assert!(
                Persona::parse(&format!("静かな司書。{separator}紅茶が好き。")).is_ok(),
                "中間の行区切りは正当な段落の区切りであること: separator = {separator:?}"
            );
        }
        // 制御文字だけの本文も同じ（`SafeText` が空白に潰して行ごと落とす）。
        assert_eq!(Persona::parse("\u{0000}\u{0007}"), Err(PersonaError::Empty));
    }

    /// **見えない文字だけのペルソナは作れない**（作業単位 0009a）。
    ///
    /// 0009 の条件（無害化すると空になる）では U+2000 / U+200B が素通りし、
    /// **`# ペルソナ` の下に見えない引用行が1本載るだけ**のキャラクターが作れた。
    #[test]
    fn persona_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                Persona::parse(&character.to_string()),
                Err(PersonaError::Empty),
                "見えない文字だけの本文が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                Persona::parse(&format!("\u{3000}{character}\n{character} ")),
                Err(PersonaError::Empty),
                "空白・改行と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                Persona::parse(&format!("静かな司書。{character}紅茶が好き。")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn persona_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(Persona::MAX_CHARS);
        assert_eq!(
            Persona::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(Persona::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        let over_limit = "あ".repeat(Persona::MAX_CHARS + 1);
        assert_eq!(
            Persona::parse(&over_limit),
            Err(PersonaError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    ///
    /// バイト上限にすると日本語のペルソナだけが英語の 1/3 しか書けなくなる。
    #[test]
    fn persona_length_is_counted_in_characters_not_bytes() {
        let japanese = "猫".repeat(Persona::MAX_CHARS);
        assert!(
            japanese.len() > Persona::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(Persona::parse(&japanese).is_ok(), "バイト数で弾かないこと");
        assert_eq!(
            Persona::parse(&"🐈".repeat(Persona::MAX_CHARS + 1)),
            Err(PersonaError::TooLong),
            "1スカラー値=1文字として数えること"
        );
    }

    /// 長さの判定は**正規化の後**（空白を足しただけで弾かれない）。
    #[test]
    fn persona_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} \n", "あ".repeat(Persona::MAX_CHARS));
        assert_eq!(
            Persona::parse(&padded).map(|value| value.as_str().chars().count()),
            Ok(Persona::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **貼り付け事故（会話ログ・巨大なテキスト）は入口で止まる。**
    ///
    /// ペルソナは削れない枠に載るので、1MB のペルソナを持つキャラクターは
    /// 以後のすべてのチャットが `BudgetExhausted` になる。
    /// **気付くのは「そのキャラクターと話せなくなってから」**なので入口で止める
    /// （`Persona` の doc の 0012a の注記）。
    #[test]
    fn persona_rejects_a_pasted_document() {
        let huge = "あ".repeat(1_000_000);
        assert!(huge.len() >= 1_000_000, "前提: 十分に大きいこと");
        assert_eq!(Persona::parse(&huge), Err(PersonaError::TooLong));

        let conversation_log = "ユーザ: こんにちは\n葵: こんにちは。\n".repeat(10_000);
        assert_eq!(
            Persona::parse(&conversation_log),
            Err(PersonaError::TooLong)
        );
    }

    /// 上限は**記憶より長く**、しかし想定の桁を外れない。
    ///
    /// ペルソナはキャラクター定義の全文なので1件の記憶より長くてよい。
    /// **下げる方向の変更は既存データを壊す**（`MAX_CHARS` の doc）ので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn persona_limit_stays_in_the_intended_range() {
        // **境界そのものを固定する**（作業単位 12b で 1000 → 600 に下げた）。
        // 範囲だけだと、属性の枠を作るために更に削る変更が黙って通る。
        assert_eq!(
            Persona::MAX_CHARS,
            600,
            "ADR-0009 が決めた値であること。変えるなら ADR を先に直すこと"
        );
        assert!(
            (500..=2_000).contains(&Persona::MAX_CHARS),
            "上限は千文字前後であること: {}",
            Persona::MAX_CHARS
        );
        // ペルソナの上限は記憶1件（`MemoryContent::MAX_CHARS` = 500）より長いこと。
        // 記憶は事実1件、ペルソナは定義の全文で、単位が違う。
        const {
            assert!(Persona::MAX_CHARS > crate::domain::MemoryContent::MAX_CHARS);
        }
    }

    /// **エラーの `Display` に本文も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn persona_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(Persona::MAX_CHARS);
        let err = Persona::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "本文を含めないこと: {message}");
        assert!(
            !message.contains(&Persona::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// エラーは理由を区別できる。
    #[test]
    fn persona_error_variants_are_distinguishable() {
        assert_ne!(PersonaError::Empty, PersonaError::TooLong);
        assert_ne!(
            PersonaError::Empty.to_string(),
            PersonaError::TooLong.to_string()
        );
        // 識別子のエラーと文言が被らない（応答で取り違えない）。
        assert_ne!(
            PersonaError::Empty.to_string(),
            HandleError::Empty.to_string()
        );
    }

    /// `Display` は正規化後の値をそのまま返す。
    #[test]
    fn persona_display_matches_as_str() {
        let value = persona(" 静かな司書。 ");
        assert_eq!(value.to_string(), "静かな司書。");
        assert_eq!(value.to_string(), value.as_str());
    }

    // ------------------------------------------------------------------
    // 出力形式指示
    // ------------------------------------------------------------------

    fn output_format(raw: &str) -> OutputFormat {
        OutputFormat::parse(raw).expect("正当な形式指示として解釈できること")
    }

    /// **形式指示にも上限がある。**
    ///
    /// 削れない枠に載るうえ、ここは「指示として従え」と宣言される唯一の節。
    /// 上限が無いと面積無制限の指示チャネルになる。
    #[test]
    fn output_format_rejects_a_pasted_document() {
        assert_eq!(
            OutputFormat::parse(&"あ".repeat(10_000)),
            Err(OutputFormatError::TooLong)
        );
        let conversation_log = "ユーザ: こんにちは\n葵: こんにちは。\n".repeat(10_000);
        assert_eq!(
            OutputFormat::parse(&conversation_log),
            Err(OutputFormatError::TooLong)
        );
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn output_format_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(OutputFormat::MAX_CHARS);
        assert_eq!(
            OutputFormat::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(OutputFormat::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        assert_eq!(
            OutputFormat::parse(&"あ".repeat(OutputFormat::MAX_CHARS + 1)),
            Err(OutputFormatError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    #[test]
    fn output_format_length_is_counted_in_characters_not_bytes() {
        let japanese = "囲".repeat(OutputFormat::MAX_CHARS);
        assert!(
            japanese.len() > OutputFormat::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(
            OutputFormat::parse(&japanese).is_ok(),
            "バイト数で弾かないこと"
        );
        assert!(OutputFormat::parse(&"\u{20BB7}".repeat(OutputFormat::MAX_CHARS)).is_ok());
        assert_eq!(
            OutputFormat::parse(&"🐈".repeat(OutputFormat::MAX_CHARS + 1)),
            Err(OutputFormatError::TooLong)
        );
    }

    /// 長さの判定は**正規化の後**（空白を足しただけで弾かれない）。
    #[test]
    fn output_format_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} \n", "囲".repeat(OutputFormat::MAX_CHARS));
        assert_eq!(
            OutputFormat::parse(&padded).map(|value| value.as_str().chars().count()),
            Ok(OutputFormat::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **空の形式指示は作れない。** 未設定は `Option::None` で表す。
    ///
    /// `Some("")` を通すと「未設定」の表現が2通りになり、
    /// 「設定したのに節が出ない」が黙って成立する。
    #[test]
    fn output_format_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                OutputFormat::parse(raw),
                Err(OutputFormatError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// **「広い意味で空白」だけの形式指示は作れない**（作業単位 0009）。
    ///
    /// `Persona` と同じ穴。空だけを拒否していると `"\u{2028}"` が通り、
    /// 無害化で空になって**節ごと落ちる**（「設定したのに効かない」が黙って成立する）。
    #[test]
    fn output_format_rejects_values_that_sanitize_to_nothing() {
        for separator in LINE_BREAKS {
            assert_eq!(
                OutputFormat::parse(&separator.to_string()),
                Err(OutputFormatError::Empty),
                "separator = {separator:?}"
            );
            assert_eq!(
                OutputFormat::parse(&format!("\u{3000}{separator} {separator}\t")),
                Err(OutputFormatError::Empty),
                "空白と混ぜても空: separator = {separator:?}"
            );
            assert!(
                OutputFormat::parse(&format!("台詞は「」。{separator}地の文はそのまま。")).is_ok(),
                "中間の行区切りは正当な複数行の指示であること: separator = {separator:?}"
            );
        }
        assert_eq!(
            OutputFormat::parse("\u{0000}\u{0007}"),
            Err(OutputFormatError::Empty)
        );
    }

    /// **見えない文字だけの形式指示は作れない**（作業単位 0009a）。
    ///
    /// `Persona` と同じ穴。通してしまうと `# 出力形式` の節が
    /// **見えない引用行1本**になり、「設定したのに効かない」が黙って成立する。
    #[test]
    fn output_format_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                OutputFormat::parse(&character.to_string()),
                Err(OutputFormatError::Empty),
                "見えない文字だけの指示が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                OutputFormat::parse(&format!("\u{3000}{character}\n{character} ")),
                Err(OutputFormatError::Empty),
                "空白・改行と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                OutputFormat::parse(&format!("台詞は「」。{character}地の文はそのまま。")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// 前後の空白は除去し、**中間の改行は保持する**（複数行の指示は正当）。
    #[test]
    fn output_format_trims_the_edges_and_keeps_inner_newlines() {
        assert_eq!(
            output_format(" 台詞は「」で囲む。 ").as_str(),
            "台詞は「」で囲む。"
        );
        assert_eq!(
            output_format("台詞は「」。\n地の文はそのまま。").as_str(),
            "台詞は「」。\n地の文はそのまま。"
        );
    }

    /// **制御文字は拒否しない**（`Persona` と同じ扱い。無害化は `prompt` の責務）。
    #[test]
    fn output_format_does_not_reject_control_characters() {
        assert!(OutputFormat::parse("1行目\n2行目").is_ok());
        assert!(OutputFormat::parse("a\u{0007}b").is_ok());
    }

    /// 上限はペルソナより小さく、想定の桁を外れない。
    ///
    /// ADR-0007 は「形式指示のトークンコストがほぼゼロ」を前提にしているので、
    /// ペルソナと同じ桁まで広げたらここで気付けるようにしておく。
    #[test]
    fn output_format_limit_stays_in_the_intended_range() {
        assert!(
            (50..=300).contains(&OutputFormat::MAX_CHARS),
            "上限は1〜2文に見合う長さであること: {}",
            OutputFormat::MAX_CHARS
        );
        const {
            assert!(OutputFormat::MAX_CHARS < Persona::MAX_CHARS);
        }
        // 実際に配っているサンプルの形式指示は余裕をもって収まること。
        assert!(
            OutputFormat::parse("台詞は「」で囲み、それ以外は地の文として書いてください。").is_ok()
        );
    }

    /// **エラーの `Display` に本文も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn output_format_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(OutputFormat::MAX_CHARS);
        let err = OutputFormat::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "本文を含めないこと: {message}");
        assert!(
            !message.contains(&OutputFormat::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// エラーは理由を区別でき、他の型のエラーとも文言が被らない。
    #[test]
    fn output_format_error_variants_are_distinguishable() {
        assert_ne!(OutputFormatError::Empty, OutputFormatError::TooLong);
        assert_ne!(
            OutputFormatError::Empty.to_string(),
            OutputFormatError::TooLong.to_string()
        );
        assert_ne!(
            OutputFormatError::TooLong.to_string(),
            PersonaError::TooLong.to_string(),
            "ペルソナのエラーと取り違えないこと"
        );
        assert_ne!(
            OutputFormatError::TooLong.to_string(),
            HandleError::TooLong.to_string()
        );
    }

    /// `Display` は正規化後の値をそのまま返す。
    #[test]
    fn output_format_display_matches_as_str() {
        let value = output_format(" 台詞は「」で囲む。 ");
        assert_eq!(value.to_string(), "台詞は「」で囲む。");
        assert_eq!(value.to_string(), value.as_str());
    }

    // ------------------------------------------------------------------
    // 表示名（プロンプトに載る呼び名 / ADR-0011）
    // ------------------------------------------------------------------

    fn display_name(raw: &str) -> DisplayName {
        DisplayName::parse(raw).expect("正当な表示名として解釈できること")
    }

    /// 前後の空白は除去し、**中間の空白は保持する**（`"葵 碧"` は正当な呼び名）。
    #[test]
    fn display_name_trims_the_edges_and_keeps_inner_blanks() {
        for (raw, expected) in [
            (" 葵 ", "葵"),
            ("\u{3000}葵\u{3000}", "葵"),
            ("\u{00A0}aoi\u{00A0}", "aoi"),
            ("\u{000B}\u{000C}葵\r", "葵"),
            (" \u{3000}\t\r\n葵 碧\u{00A0} ", "葵 碧"),
        ] {
            assert_eq!(display_name(raw).as_str(), expected, "raw = {raw:?}");
        }
        for raw in ["葵 碧", "葵\u{3000}碧", "a b c"] {
            assert_eq!(display_name(raw).as_str(), raw, "raw = {raw:?}");
        }
    }

    /// **空の表示名は作れない。**
    ///
    /// 空だとプロンプトに**名前のない見出し**（`# 名前` の下が空）が載る。
    #[test]
    fn display_name_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                DisplayName::parse(raw),
                Err(DisplayNameError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// 制御文字を含むなら拒否する（`Handle` と同じく短いラベルであるため）。
    #[test]
    fn display_name_rejects_control_characters() {
        for raw in [
            "a\u{0007}b",   // BEL
            "葵\u{0000}碧", // NUL
            "a\nb",         // 中間の LF
            "a\tb",         // 中間の TAB
            "a\u{0085}b",   // NEL（trim 対象外の制御文字）
            "aoi\u{007F}",  // DEL
            " a\rb ",       // 前後を trim しても中間に残る
        ] {
            assert_eq!(
                DisplayName::parse(raw),
                Err(DisplayNameError::Control),
                "raw = {raw:?}"
            );
        }
    }

    /// **行区切りを含むなら拒否する**（作業単位 0009）。
    ///
    /// U+2028 / U+2029 は `char::is_control` が偽なので、制御文字の検査だけでは通る。
    ///
    /// # 守っているのは「**1行のラベルは1行である**」という衛生条件
    ///
    /// **注入防御ではない。** `prompt::writer::quote_line` は `Seg::Text` を
    /// `to_inline()` で1行に畳むので、`"葵\u{2028}碧"` は `> 葵 碧` になるだけで
    /// 偽の役割行にはならない。**この理由を書いておかないと**、後から
    /// 「`to_inline` があるのだから要らない」として外される
    /// （`0006` で `Mutex` を「効果を測れない」として外したのと同じ形）。
    /// 呼び名は1行として保存し1行として読むと決めた値なので、
    /// **表示上は2行になる値を入れさせない。**
    ///
    /// 「呼び名が見えなくなる」方は `display_name_rejects_invisible_characters` の担当。
    ///
    /// **`LINE_BREAKS` の全文字を回す**（固定リストにすると、集合に文字を足した人が
    /// 気付けない）。
    #[test]
    fn display_name_rejects_every_line_separator() {
        for separator in LINE_BREAKS {
            assert_eq!(
                DisplayName::parse(&format!("葵{separator}碧")),
                Err(DisplayNameError::Control),
                "separator = {separator:?}"
            );
            assert!(
                DisplayName::parse(&separator.to_string()).is_err(),
                "行区切りだけの表示名も作れないこと: separator = {separator:?}"
            );
        }
    }

    /// **何も見えない表示名は `Control` ではなく `Empty`**（作業単位 0009b）。
    ///
    /// 理由と、これが無いと何が緑を通るかは
    /// `handle_reports_empty_rather_than_control_when_nothing_is_visible` を参照。
    /// **表示名だけは HTTP のエラーコードにも現れる**ので、
    /// `tests/server_admin.rs::invalid_display_names_are_rejected_with_distinguishable_reasons`
    /// の `display_name_empty` / `display_name_control` の割り当てがここに従う。
    #[test]
    fn display_name_reports_empty_rather_than_control_when_nothing_is_visible() {
        for raw in ["\u{0007}", "\u{0000}", "\u{007F}", "\u{0007}\u{0000}"] {
            assert_eq!(
                DisplayName::parse(raw),
                Err(DisplayNameError::Empty),
                "何も見えない値は「空」として報告すること: raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                DisplayName::parse(&separator.to_string()),
                Err(DisplayNameError::Empty),
                "行区切りだけの値も「空」として報告すること: separator = {separator:?}"
            );
        }
        for raw in ["\u{0007}葵", "葵\u{0007}", "葵\u{2028}碧"] {
            assert_eq!(
                DisplayName::parse(raw),
                Err(DisplayNameError::Control),
                "見える文字があるなら制御文字として報告すること: raw = {raw:?}"
            );
        }
    }

    /// **見えない文字だけの表示名は作れない**（作業単位 0009a）。
    ///
    /// レビュアーの実測で `DisplayName::parse("\u{2000}")` が `Ok` を返し、
    /// プロンプトに `> \u{2000}`（呼び名が1文字も見えない引用行）が載っていた。
    /// **呼び名が消えていることは応答を読んでも分からない**ので、入口で止める
    /// （`Persona` の doc の 0012a の注記——`PATCH` で直せても、気付く手段は増えない）。
    #[test]
    fn display_name_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                DisplayName::parse(&character.to_string()),
                Err(DisplayNameError::Empty),
                "見えない文字だけの表示名が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                DisplayName::parse(&format!(" {character}\u{3000}{character}\t")),
                Err(DisplayNameError::Empty),
                "trim 対象の空白と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                DisplayName::parse(&format!("葵{character}碧")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// **`@` は禁止しない**（ADR-0011）。
    ///
    /// `@` を禁じているのは `model` フィールドの予約語（`aoi@new`）と衝突するからで、
    /// 表示名は `model` に現れない。禁じる理由が無い。
    #[test]
    fn display_name_allows_the_at_sign() {
        for raw in ["a@b", "@", "@葵", "葵@42", "aoi@new"] {
            assert_eq!(display_name(raw).as_str(), raw, "raw = {raw:?}");
            assert_eq!(
                Handle::parse(raw),
                Err(HandleError::AtSign),
                "識別子の側は同じ値を拒否すること: raw = {raw:?}"
            );
        }
    }

    /// **正規化してから検証する**（順序が逆だと挙動が変わるので明示的に固定する）。
    #[test]
    fn display_name_normalizes_before_validating() {
        // TAB / CR / LF は制御文字でもあるが、前後にあるだけなら「空」として扱われる。
        assert_eq!(DisplayName::parse("\t"), Err(DisplayNameError::Empty));
        assert_eq!(DisplayName::parse("\r\n"), Err(DisplayNameError::Empty));
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn display_name_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(DisplayName::MAX_CHARS);
        assert_eq!(
            DisplayName::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(DisplayName::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        assert_eq!(
            DisplayName::parse(&"あ".repeat(DisplayName::MAX_CHARS + 1)),
            Err(DisplayNameError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// **貼り付け事故は表示名でも入口で止まる。**
    ///
    /// 表示名は**削れない枠**に載るので、上限が無いと巨大な値を1件入れただけで
    /// そのキャラクターの以後の全チャットが `BudgetExhausted` になる。
    #[test]
    fn display_name_rejects_a_pasted_document() {
        assert_eq!(
            DisplayName::parse(&"あ".repeat(10_000)),
            Err(DisplayNameError::TooLong)
        );
        let persona_text = "静かな図書館の司書。物腰は落ち着いていて、".repeat(10);
        assert_eq!(
            DisplayName::parse(&persona_text),
            Err(DisplayNameError::TooLong),
            "ペルソナ本文を表示名欄に貼っても止まること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    #[test]
    fn display_name_length_is_counted_in_characters_not_bytes() {
        let japanese = "葵".repeat(DisplayName::MAX_CHARS);
        assert!(
            japanese.len() > DisplayName::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(
            DisplayName::parse(&japanese).is_ok(),
            "バイト数で弾かないこと"
        );
        assert!(DisplayName::parse(&"\u{20BB7}".repeat(DisplayName::MAX_CHARS)).is_ok());
        assert_eq!(
            DisplayName::parse(&"🐈".repeat(DisplayName::MAX_CHARS + 1)),
            Err(DisplayNameError::TooLong)
        );
    }

    /// 長さの判定は**正規化の後**（空白を足しただけで弾かれない）。
    #[test]
    fn display_name_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} \t", "葵".repeat(DisplayName::MAX_CHARS));
        assert_eq!(
            DisplayName::parse(&padded).map(|value| value.as_str().chars().count()),
            Ok(DisplayName::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **正当な識別子は必ず表示名としても正当**（`server` の既定値がそこに依存する）。
    ///
    /// `display_name` を省略したときは `handle` の値で埋める（ADR-0011 / 0004a）。
    /// 上限が識別子より短いと、**識別子としては作れるのに作成が失敗する**
    /// 名前が生まれる。長さだけでなく、拒否する文字集合も識別子より緩いこと。
    #[test]
    fn every_valid_handle_is_also_a_valid_display_name() {
        const {
            assert!(DisplayName::MAX_CHARS >= Handle::MAX_CHARS);
        }

        for raw in [
            "aoi",
            "s10",
            "aoi-v2",
            "aoi_2",
            "-",
            "_",
            "0",
            Handle::ALLOWED_CHARACTERS,
            &"a".repeat(Handle::MAX_CHARS),
        ] {
            let name = Handle::parse(raw).expect("識別子として正当であること");
            assert_eq!(
                DisplayName::parse(name.as_str()).map(|value| value.as_str().to_owned()),
                Ok(name.as_str().to_owned()),
                "識別子をそのまま表示名にできること: raw = {raw:?}"
            );
        }
    }

    /// 上限は「短いラベル」として妥当な範囲に収まっている。
    ///
    /// **下げる方向の変更は保存済みの行を引けなくする**（`MAX_CHARS` の doc）ので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn display_name_limit_stays_in_the_intended_range() {
        assert!(
            (20..=100).contains(&DisplayName::MAX_CHARS),
            "上限は短いラベルに見合う長さであること: {}",
            DisplayName::MAX_CHARS
        );
        const {
            assert!(DisplayName::MAX_CHARS < Persona::MAX_CHARS);
        }
    }

    /// **エラーの `Display` に表示名も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn display_name_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(DisplayName::MAX_CHARS);
        let err = DisplayName::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "表示名を含めないこと: {message}");
        assert!(
            !message.contains(&DisplayName::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// エラーは理由を区別でき、**識別子のエラーとも文言が被らない**。
    ///
    /// 同じ文言にすると、応答を見た人が「どちらの欄が悪いのか」を判断できない。
    #[test]
    fn display_name_error_variants_are_distinguishable() {
        let variants = [
            DisplayNameError::Empty,
            DisplayNameError::Control,
            DisplayNameError::TooLong,
        ];

        for (index, left) in variants.iter().enumerate() {
            assert!(
                !left.to_string().is_empty(),
                "理由を説明するメッセージがあること"
            );
            for right in &variants[index + 1..] {
                assert_ne!(left, right);
                assert_ne!(
                    left.to_string(),
                    right.to_string(),
                    "文言でも区別できること"
                );
            }
        }

        assert_ne!(
            DisplayNameError::Empty.to_string(),
            HandleError::Empty.to_string(),
            "識別子のエラーと取り違えないこと"
        );
        assert_ne!(
            DisplayNameError::TooLong.to_string(),
            HandleError::TooLong.to_string()
        );
        assert_ne!(
            DisplayNameError::Control.to_string(),
            HandleError::Control.to_string()
        );
    }

    /// `Display` は正規化後の値をそのまま返す（SQL へのバインド値と一致する）。
    #[test]
    fn display_name_display_matches_as_str() {
        let value = display_name(" 葵 碧 ");
        assert_eq!(value.to_string(), "葵 碧");
        assert_eq!(value.to_string(), value.as_str());
    }

    // ------------------------------------------------------------------
    // 更新の差分（ADR-0016 / 作業単位 0012a）
    // ------------------------------------------------------------------

    /// **既定は「1列も変えない」。** 省略された列を既定値で潰さないことの土台。
    #[test]
    fn an_empty_update_changes_nothing() {
        let update = CharacterUpdate::default();
        assert_eq!(update.display_name, None);
        assert_eq!(update.base_persona, None);
        assert_eq!(update.output_format_instruction, FieldUpdate::Keep);
    }

    /// **「変えない」と「消す」は別の値。**
    ///
    /// 同じ値になった瞬間、部分更新のたびに形式指示が黙って消える
    /// （`Option<OutputFormat>` の1段で表そうとすると必ずこうなる）。
    #[test]
    fn keep_and_clear_are_distinct_values() {
        let keep: FieldUpdate<OutputFormat> = FieldUpdate::Keep;
        let clear: FieldUpdate<OutputFormat> = FieldUpdate::Clear;
        let set = FieldUpdate::Set(
            OutputFormat::parse("台詞は「」で囲む。").expect("正当な形式指示であること"),
        );

        assert_ne!(keep, clear, "「変えない」と「消す」を同一視しないこと");
        assert_ne!(keep, set);
        assert_ne!(clear, set);
        assert_eq!(FieldUpdate::<OutputFormat>::default(), keep, "既定は Keep");
    }

    // ------------------------------------------------------------------
    // 指し示す列（ADR-0016 §3 / 作業単位 0012b）
    // ------------------------------------------------------------------

    /// **`ALL` に数え漏れが無い。**
    ///
    /// 綴りが実在の列名と一致することは
    /// `tests/store_schema.rs::every_pointed_at_column_is_a_real_character_column` が
    /// `PRAGMA table_info` と突き合わせて見るが、あちらが回るのは `ALL` の中身だけ。
    /// **`ALL` から漏れた variant は、あの突き合わせを素通りする。**
    ///
    /// variant を足すと下の `match` がコンパイルできなくなり、
    /// 足した腕は `ALL` に入っていることを主張するので、`ALL` を直すまで赤のまま。
    #[test]
    fn every_variant_is_listed_in_all() {
        // **網羅の `match`。** ワイルドカードの腕を足さないこと（足した瞬間に見張り番が死ぬ）。
        let listed = |column: CharacterColumn| match column {
            CharacterColumn::Handle
            | CharacterColumn::DisplayName
            | CharacterColumn::BasePersona
            | CharacterColumn::OutputFormatInstruction
            | CharacterColumn::StatScale
            | CharacterColumn::StatDriftMode
            | CharacterColumn::AxisDiscoverySlots
            | CharacterColumn::ArchivedAt => CharacterColumn::ALL.contains(&column),
        };

        for column in CharacterColumn::ALL {
            assert!(listed(column), "{column:?} が ALL に無い");
        }
        assert_eq!(
            CharacterColumn::ALL.len(),
            8,
            "列を足したなら ALL の要素数も増えていること"
        );
    }

    /// 列名は**互いに違う**（1つの綴りが2つの列を指さない）。
    ///
    /// 実在の列名かどうかは統合テスト側（`PRAGMA table_info`）の担当。
    /// ここは DB を要らない側だけを見る。
    #[test]
    fn column_names_are_distinct() {
        for (index, left) in CharacterColumn::ALL.iter().enumerate() {
            assert!(!left.as_str().is_empty(), "{left:?} の列名が空");
            for right in &CharacterColumn::ALL[index + 1..] {
                assert_ne!(
                    left.as_str(),
                    right.as_str(),
                    "{left:?} と {right:?} が同じ列名を指している"
                );
            }
        }
    }
}
