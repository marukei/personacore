//! `character` / `stat_alloc` の読み書き。
//!
//! この層は**薄く保つ**。名前や軸名の検証・正規化は `domain` 側の `parse` で済んでおり、
//! ここは「検証済みの値を SQL にバインドする」だけを担う。
//!
//! 引数に `&str` を取る公開 API を生やさないこと。生やした瞬間に
//! 「検証を通っていない文字列が DB に届く経路」ができる。
//!
//! 主キーも裸の `i64` では受け取らない（`CharacterId`）。`session` / `message` が
//! 加わると同型の識別子が並び、取り違えが型検査を素通りするため。

use chrono::{DateTime, Utc};
use sqlx::Row;

use super::{Store, StoreError};
use crate::domain::{
    AxisDiscoverySlots, Character, CharacterColumn, CharacterEntry, CharacterId, CharacterUpdate,
    DisplayName, FieldUpdate, Handle, OutputFormat, Persona, StatAlloc, StatAxis, StatDriftMode,
    StatScale, UnreadableCharacter,
};

/// 一覧に使う SELECT。列の並びは `Character` の組み立てと対応させる。
///
/// # `stat_drift_mode` を読むのは、この文と [`SELECT_BY_HANDLE`] の2つだけ（作業単位 A-8）
///
/// **`character` の行を `Character` へ組み立てる文はこの2つしか無い。**
/// 他の文（[`SELECT_ID_BY_HANDLE`] / [`SELECT_STAT_SCALE_BY_ID`] / [`SELECT_STAT_ALLOCS`] /
/// `store::drift` の尺度の副問い合わせ）は1列しか読まないので、新しい列の影響を受けない。
/// **列を落とすと、キャラクターを読む経路が全部止まる**（`try_get` が `ColumnNotFound`）
/// ——チャット・変容・一覧・`/v1/models` のどれもが、この列を前提にしている。
///
/// # `axis_discovery_slots` も同じ2つだけが読む（作業単位 A-9a / ADR-0046 §決定1）
///
/// **過去の書き手は全部 `DEFAULT 0` を通る**——`0015` の `ADD COLUMN` が既存の行を 0 で埋め、
/// `create_character_with_drift_mode` の INSERT はこの列を書かない（既定の 0 が入る）。
/// 書くのは [`UPDATE_CHARACTER`] だけで、値は `AxisDiscoverySlots` を通っている。
/// **だから読み始めても既存の行のデコードは割れない**（CLAUDE.md §2 の ⚠）。
/// **ただし上の注記と同じく、列を落とすとキャラクターを読む経路が全部止まる。**
///
/// # `archived_at` も同じ2つだけが読む（作業単位 A-10 / ADR-0049 §決定1）
///
/// **過去の書き手は全部 NULL を通る**——`0017` の `ADD COLUMN` は DEFAULT を持たず、
/// INSERT はこの列を書かない。書くのは [`UPDATE_CHARACTER`] だけで、綴りは
/// `created_at` の DEFAULT と同じ `strftime`。**読みは `Option<String>` で受けて
/// [`archived_at_from_row`] が日時に直す**（`try_get::<DateTime>` にすると、
/// 読めない値1つで一覧が丸ごと落ちる）。
const SELECT_ALL: &str = "SELECT id, handle, display_name, base_persona, output_format_instruction, \
     stat_scale, stat_drift_mode, axis_discovery_slots, created_at, archived_at \
     FROM character ORDER BY handle";

/// 識別子による完全一致検索。**trim しない**（ADR-0004: 検索側で trim すると
/// `"aoi"` と `" aoi "` の両方に一致し、書き込み先が不定になる）。
const SELECT_BY_HANDLE: &str = "SELECT id, display_name, base_persona, \
     output_format_instruction, stat_scale, stat_drift_mode, axis_discovery_slots, created_at, \
     archived_at FROM character WHERE handle = ?";

/// 識別子で id だけを引く。**完全一致**（`SELECT_BY_HANDLE` と同じ照合）。
///
/// 更新した行の id を返すため、および削除が0件だった理由を引き直すために使う。
/// **他の列を読まない**のが要点で、読むと `parse` を通したくなり、
/// 「直したい行ほど読めない」（ADR-0016 §4）に逆戻りする。
///
/// # かつては正規化キー（`trim(name, ...)`）で引いていた
///
/// ゴースト行（`" aoi "`）を `"aoi"` として指すためだったが、
/// **その照合は 0004b-2 で捨てた**（`migrations/0006_drop_character_name_key_idx.sql`）。
/// 「一致する行は高々1件」を支えていた `character_name_key_idx` を落とす以上、
/// `trim` を残すと**本物のキャラクターまで巻き添えにする**（あの移行の 3 に実測がある）。
const SELECT_ID_BY_HANDLE: &str = "SELECT id FROM character WHERE handle = ?";

/// 更新の WHERE 句。**完全一致**（`SELECT_BY_HANDLE` / `SELECT_ID_BY_HANDLE` と同じ照合）。
///
/// # かつては正規化キー（`trim(name, ...)`）だった
///
/// 更新と削除は**修復と掃除の入口**（ADR-0016）なので、ゴースト行
/// （`" aoi "` のように前後に空白の付いた名前の行）にも届くようにしてあった。
/// **0004b-2 でその照合を捨てた**（ADR-0017）。理由は3つとも
/// `migrations/0006_drop_character_name_key_idx.sql` に書いてある。要点だけ:
///
/// * 「一致する行は高々1件」を保証していたのは `character_name_key_idx` で、
///   **その索引を落とす以上 `trim` は残せない**（残すと `"aoi"` と `" aoi "` の
///   両方に一致し、本物の行が黙って書き換わる／消える。実測）。
/// * ゴースト行を掃除する導線は **0004b-1 の時点で既に切れている**。
///   一覧は `handle: null` を返すので、運用者は打つべき綴りを API からは知り得ない。
/// * `trim` が拾えたのは `TRIMMABLE` の8文字で埋まった行だけで、
///   `"Aoi"` や `"葵"` の行には最初から届いていない。
///
/// **完全一致に揃えたことで、引き当てが2行に一致する状態が作れなくなった。**
/// 見張り番は `tests/store_character.rs::
/// an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly`。
///
/// # 6つの列だけを書く
///
/// * `display_name` / `base_persona` / `stat_scale` は `coalesce(?, 現在値)`。
///   **NULL を渡せば触らない。** どれも NOT NULL の列で、
///   `DisplayName` / `Persona` は空を作れず `StatScale` は `1` 以上なので、
///   「NULL = 変えない」と「値 = 置き換える」が衝突しない。
/// * `output_format_instruction` は NULL になりうるので**上の手が使えない。**
///   「変えない / 設定する / 消す」の3値を、`?3`（置き換えるか）と `?4`（新しい値）に分ける。
/// * `stat_drift_mode`（ADR-0045 / 作業単位 A-8）も同じ3値で、`?6`（置き換えるか）と
///   `?7`（新しい値）。**`Clear` は「未設定＝起動時の既定に従う」へ戻す。**
/// * `axis_discovery_slots`（ADR-0046 / 作業単位 A-9a）は `coalesce(?9, 現在値)`
///   （NOT NULL の列なので `stat_scale` と同じ2値）。**`handle` の `?8` を動かさないよう、
///   番号は末尾に足した**（bind も `handle` の後ろ）。
/// * **`archived_at`（しまった時刻 / ADR-0049 §決定1 / 作業単位 A-10）は `?10`。**
///   NULL なら触らない、真ならしまう（**すでに時刻があればそのまま**）、偽なら戻す（NULL にする）。
///   **時刻は呼び出し側から受け取らない**——書くのはこの文の `strftime` で、綴りは
///   `created_at` の DEFAULT（`migrations/0001_initial.sql`）と同じ。
/// * **`handle` はこの文に現れない。** 識別子は不変（ADR-0016 §1）。
///
/// **引き当ては行を読まずに WHERE で行う。** `find_character_by_handle` を先に呼ぶ形にすると
/// 全フィールドが `parse` を通り、**壊れた行は永久に直せなくなる**（ADR-0016 §4）。
///
/// # 位置指定（`?N`）とバインド順は**別々に書かれている**
///
/// `sqlx` の `.bind()` は呼んだ順に `?1` から割り当てるので、
/// **この文の `?N` を並べ替えても、bind の並びを直さなければ黙って別の列へ書く。**
/// 0011a-1 で `stat_scale` を足したとき `handle` の番号が `?5` → `?6` にずれ、
/// A-8 で `stat_drift_mode` を足したとき `?6` → `?8` にずれている。
/// 見張り番は `tests/server_admin.rs::a_partial_update_leaves_the_other_columns_alone`
/// （4列すべてに互いに違う値を入れてあるので、取り違えはどの組でも赤くなる）と、
/// 5列を1回で書く `tests/store_character.rs::every_updatable_column_lands_in_its_own_place`。
///
/// # **保存済みの値より小さい尺度は書かせない**（ADR-0015 / 作業単位 0011a-2）
///
/// 許すと、その `PATCH` 一回で `stat_allocs` が丸ごと失敗するようになり、
/// **以後のチャットが必ず失敗する。** ADR-0015 が閉じると宣言した穴を、
/// アプリ自身が新しい経路から開けることになる。
///
/// **判定と更新は `DELETE_CHARACTER` と同じ形**——`NOT EXISTS` を同じ文に置く。
/// 数えてから書くと、その隙に入った `PUT .../stats` の値が判定をすり抜ける。
///
/// **`>` であって `>=` ではない。** 尺度 10 に値 10 は正当（ADR-0015）。
///
/// **`?5` が NULL（尺度を変えない）なら、この節は常に真。**
/// SQL の `value > NULL` は NULL であって真にならないので、
/// EXISTS は偽 → `NOT EXISTS` は真になる。つまり
/// **尺度を触らない更新がこの節に引っ掛かることは無い。**
///
/// 見張り番は `tests/store_character.rs::
/// a_partial_update_reaches_a_character_that_already_has_axes`。
/// **`the_stat_scale_survives_the_round_trip_and_partial_updates` では見張れない**——
/// あちらの対象は `stat_alloc` の行を1つも持たず、行が無ければ
/// `NOT EXISTS` は NULL の扱いと関係なく真になるので、
/// `?5` に既定値を与える変異（`coalesce(?5, -1)`）が緑を通る（実測 810/810 緑）。
/// **軸を持つキャラクターへの、尺度を含まない部分更新**でしか観測できない。
const UPDATE_CHARACTER: &str = "UPDATE character SET \
     display_name = coalesce(?1, display_name), \
     base_persona = coalesce(?2, base_persona), \
     output_format_instruction = CASE WHEN ?3 THEN ?4 ELSE output_format_instruction END, \
     stat_scale = coalesce(?5, stat_scale), \
     stat_drift_mode = CASE WHEN ?6 THEN ?7 ELSE stat_drift_mode END, \
     axis_discovery_slots = coalesce(?9, axis_discovery_slots), \
     archived_at = CASE WHEN ?10 IS NULL THEN archived_at \
                        WHEN ?10 THEN coalesce(archived_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                        ELSE NULL END \
     WHERE handle = ?8 \
     AND NOT EXISTS (SELECT 1 FROM stat_alloc \
                     WHERE stat_alloc.character_id = character.id AND stat_alloc.value > ?5)";

/// 主キーで**尺度だけ**を引く（作業単位 0011a-2）。
///
/// **`replace_stat_allocs` がトランザクションの中で使う。** 外で読むと、その間に
/// `PATCH` が尺度を変えて**検査した尺度と書き込む先の尺度が別物**になる。
///
/// `SELECT_ID_BY_HANDLE` と同じく**他の列を読まない。** 読むと `parse` を通したくなり、
/// 「直したい行ほど読めない」（ADR-0016 §4）に逆戻りする。
///
/// # **ただし、読んだ1列は `parse` を通す**（0011a-2 のレビューで実測）
///
/// 尺度は突き合わせに使うので `stat_scale_from_row` を通さざるを得ない。つまり
/// **`SELECT_ID_BY_HANDLE` と違い、この文は「壊れた行でも読める」を実現していない。**
/// **`character.stat_scale` 列そのものが壊れた行では、置き換えも
/// `InvalidStoredValue` で止まる**（`StatScaleMismatch` ですらない）。
///
/// **置き換えが届くのは `stat_alloc` 側だけが壊れている場合。**
/// 尺度の列が壊れた行は**先に `PATCH`（`update_character`）で尺度を直す。**
/// あちらは行を読まずに WHERE だけで書くので、壊れた尺度にも届く。
///
/// 手順と各段の実測は `replace_stat_allocs` の doc（この文の唯一の呼び出し元）。
const SELECT_STAT_SCALE_BY_ID: &str = "SELECT stat_scale FROM character WHERE id = ?";

/// **読みから始まって書きに昇格するトランザクションの開始文**（ADR-0004 / 作業単位 0011a-2）。
///
/// `replace_stat_allocs` は「保存済みの尺度を読む → DELETE / INSERT」の形なので、
/// 素の `BEGIN`（DEFERRED）だと**最初の読みで読み取りスナップショットが固定され**、
/// 続く書きへの昇格が `SQLITE_BUSY_SNAPSHOT`(517) になる。
/// これは **`busy_timeout` の対象外で、待たずに即座に失敗する**（`session.rs` の同名の定数）。
///
/// **WAL のスナップショットは DB 全体を指す**ので、
/// 「同じキャラクターへの `PATCH` と競合する場合だけ」の話ではない。
/// **無関係な書き込みが1つ commit されただけで昇格が落ちる。**
/// 実測（2026-08-20 / テストバイナリを1本ずつ直列に 5 回 / 各 64 周）——素の `BEGIN` のとき、
/// 尺度を下げる `PATCH` と同時に走らせた置き換えは、採取できた 315 周のうち
/// **298 周が `database is locked`**（`StatScaleMismatch` は 0 周）。
/// さらに、同じ DB へ**無関係な** `update_character` を 200µs 間隔で回しながら測ると、
/// **競合すらしていない逐次の置き換え**（周回の初期値設定）が落ち、5 回中 4 回は
/// そこでテストが止まった。`BEGIN IMMEDIATE` にすると同じ負荷で 0 件になる。
///
/// 壊れ方が重いのは、**そこが尺度を超えた行の唯一の復旧経路**だからで、
/// 会話が走っている最中は**直したいときほど詰まる。**
///
/// 見張り番は `tests/store_character.rs::replacing_stats_never_races_a_shrinking_scale`
/// （「両方は成功しない」ではなく**負けた側の理由**まで照合しているので、
/// この開始文を素の `BEGIN` に戻すと `StoreError::Query` が現れて赤くなる）。
///
/// **`session.rs` の定数と共有しない。** 中身は同じ文字列だが、
/// 「なぜここで要るのか」は関数ごとに別で、共有すると片方の理由が消えたときに
/// もう片方も一緒に外れる。**`delete_character` は現に要らない**
/// （最初の文が書きなので昇格が起きない。実測はそちらの doc）。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// 性格ステータスを**そのキャラクターの尺度と一緒に**引く（作業単位 0011a-2）。
///
/// # なぜ2文に分けないのか
///
/// 値の検証には尺度が要る。別々に読むと、その間に `PATCH` が尺度を変えて
/// **読んだ値と違う尺度で検証する**状態が作れる。1文なら SQLite が原子的に見る。
///
/// # `LEFT JOIN` にする理由 —— **孤児を黙って消さない**
///
/// 内部結合にすると、**親の居ない `stat_alloc` の行が黙って消える**（`Ok([])` になり、
/// 性格ステータスを設定してあるキャラクターが「一軸も持たない」ように見える）。
/// `LEFT JOIN` なら `stat_scale` が NULL として現れ、
/// `StoreError::InvalidStoredValue` になって**気付ける。**
///
/// # **「孤児は作れない」は誤り**（0011a-2 のレビューで撤回）
///
/// ここには当初「外部キーは `Store::connect` が接続ごとに有効化しているので
/// 到達不能」と書いてあった。**`PRAGMA foreign_keys` は接続ごとの設定なので、
/// プールから借りた接続の上で倒せる**（`ignore_check_constraints` でゴースト行を
/// 作るのと同じ手）。倒したまま親を消せば `ON DELETE CASCADE` も走らず、子だけが残る。
///
/// **`JOIN` への変異が 810/810 緑だったのは到達不能だからではなく、
/// テストが1本も無かったから。** 見張り番を足した（`tests/store_character.rs::
/// stat_allocs_of_an_orphaned_row_stop_the_read_instead_of_disappearing`）。
///
/// **到達不能と書くと、次に「無用な `LEFT JOIN`」として外される経路ができる。**
/// 脅威モデル（DB を直接編集された場合の担保）は、`insert_ghost_character` /
/// `set_stat_scale_ignoring_checks` と同じもの。
const SELECT_STAT_ALLOCS: &str = "SELECT stat_alloc.axis AS axis, stat_alloc.value AS value, \
     character.stat_scale AS stat_scale \
     FROM stat_alloc LEFT JOIN character ON character.id = stat_alloc.character_id \
     WHERE stat_alloc.character_id = ? ORDER BY stat_alloc.rowid";

/// 削除。**判定と削除を1文で不可分に行う**（ADR-0016 §2 / 作業単位 0012a）。
///
/// **数えてから消さないこと。** `SELECT count(*)` と `DELETE` に分けると、
/// その隙に始まった会話が `ON DELETE CASCADE` で `session` → `message` ごと
/// **黙って消える**（エラーにならないので誰も気付かない）。
/// `NOT EXISTS` を同じ文に置けば、判定と削除の間は原子的になる。
///
/// 照合は `UPDATE_CHARACTER` と同じ**完全一致**（0004b-2 で正規化キーから切り替えた。
/// 理由と実測は `UPDATE_CHARACTER` の doc と
/// `migrations/0006_drop_character_name_key_idx.sql`）。
///
/// **削除の側が特に重い。** `trim` のまま索引だけを落とすと、
/// ゴースト行を掃除したつもりで**同じ綴りの本物のキャラクターまで消える**
/// （`ON DELETE CASCADE` で `session` → `message` ごと。実測）。
///
/// 0件だった理由（該当なし / 会話がある）は**同じトランザクションの中で**引き直す。
/// 外に出すと、その間に会話が始まったり行が消えたりして理由が入れ替わる。
///
/// # **しまったキャラクターは、会話があっても消す**（ADR-0049 §決定1 / 作業単位 A-10）
///
/// 条件は「会話が無い」**または「しまってある」**（`archived_at IS NOT NULL`）。
/// **しまったかどうかも、この同じ文で見る。** 読んでから消すと、その隙に**戻された**
/// キャラクターを会話・記憶ごと消しうる（戻した利用者には成功が返っているのに、中身が消える）。
/// 見張り番は `tests/store_character_archive.rs::deleting_never_races_a_restore`。
///
/// **消えるのは `character` の1行と、それを指す全部の行**（`ON DELETE CASCADE`。
/// 表ごとの見張り番は `tests/store_character_archive.rs::
/// a_put_away_character_is_deleted_with_every_row_that_points_at_it`）。
/// **控えは残さない**（ADR-0049 §決定1。中身ごと消したい、という利用者の答え）。
///
/// # ⚠ 読めない `archived_at` も「しまってある」と見なす
///
/// この文は `archived_at IS NOT NULL` しか見ず、**日時として読めるかは確かめない。**
/// したがって時刻が壊れた行（一覧が `CharacterColumn::ArchivedAt` で指し示す行）は、
/// **会話があっても消せる。** 壊れた値は `PATCH` の `archived` でしか書けない列に
/// DB を直接書いたときにしか現れず、その行は上の一覧にもしまった一覧にも出ない
/// （読み出せなかった行に出る）。消したくないなら、先に `archived: false` で戻すこと。
const DELETE_CHARACTER: &str = "DELETE FROM character WHERE handle = ? \
     AND (archived_at IS NOT NULL \
          OR NOT EXISTS (SELECT 1 FROM session WHERE session.character_id = character.id))";

impl Store {
    /// キャラクターを作成し、作成した行の id を返す。
    ///
    /// 識別子は `Handle` を通っているので、DB には規則を満たす値だけが入る
    /// （**正規化はしない**。ADR-0017）。
    ///
    /// # 重複は1種類だけになった（0004b-2）
    ///
    /// 以前は衝突相手によって2つに分かれていた（正当な重複 → `StoreError::Query`、
    /// ゴースト行との衝突 → `StoreError::ConflictingUnnormalizedName`）。
    /// **後者は起こりえなくなった。** 正規化キーの UNIQUE 索引を落とした
    /// （`migrations/0006_drop_character_name_key_idx.sql`）ので、`handle` に残る一意制約は
    /// **列の `UNIQUE`（完全一致）だけ**であり、UNIQUE 違反は
    /// 「まったく同じ綴りの行が既にある」以外の意味を持たない。
    ///
    /// **ゴースト行があっても作成は成功する。** `" aoi "` の行が居座っていても
    /// `aoi` は普通に作れる（以前はここが塞がれ、一覧にも出ず・チャットも 404・
    /// しかし作れもしない、という袋小路になっていた）。
    ///
    /// **`display_name` は `handle` とは別の型で受け取る**（ADR-0011）。
    /// 同じ `&str` で受けると引数の取り違えが型検査を素通りし、
    /// **プロンプトに識別子が載る**という、この列を足した意味そのものが消える。
    ///
    /// # 尺度に既定値を持たせない（ADR-0015 / 作業単位 0011a-1）
    ///
    /// `stat_scale` は**必ず引数で受け取る。** `Option` にして
    /// 「`None` なら `StatScale::DEFAULT`」とすると、既定を決める場所が
    /// ここと `server` の2つになり、**片方だけ変えても誰も気付けない。**
    /// 省略の解釈は入口（`server::admin`）1箇所に置く。
    ///
    /// # 走らせ方は未設定で作る（ADR-0045 §決定1）
    ///
    /// `character.stat_drift_mode` は NULL（＝起動時の既定に従う）になる。
    /// 値を書いて作るのは [`Self::create_character_with_drift_mode`]。
    /// **この関数の振る舞いは A-8 の前と1バイトも変わらない**（既存の呼び出しがそのまま通る）。
    pub async fn create_character(
        &self,
        handle: &Handle,
        display_name: &DisplayName,
        base_persona: &Persona,
        output_format_instruction: Option<&OutputFormat>,
        stat_scale: StatScale,
    ) -> Result<CharacterId, StoreError> {
        self.create_character_with_drift_mode(
            handle,
            display_name,
            base_persona,
            output_format_instruction,
            stat_scale,
            None,
        )
        .await
    }

    /// **走らせ方まで指定して**キャラクターを作る（ADR-0045 §決定3 / 作業単位 A-8）。
    ///
    /// `stat_drift_mode` が `None` なら未設定（NULL）で作る——[`Self::create_character`] と同じ。
    /// **既定の値で埋めない**（埋めると「既定に従う」と「既定と同じ値を選んだ」が区別できない /
    /// `Character::stat_drift_mode` の doc）。
    ///
    /// **書くのは [`StatDriftMode::label`] の綴り。** 検証済みの型しか受け取らないので、
    /// 別名の綴りが DB に届く経路は無い（読み出しは `StatDriftMode::parse`）。
    pub async fn create_character_with_drift_mode(
        &self,
        handle: &Handle,
        display_name: &DisplayName,
        base_persona: &Persona,
        output_format_instruction: Option<&OutputFormat>,
        stat_scale: StatScale,
        stat_drift_mode: Option<StatDriftMode>,
    ) -> Result<CharacterId, StoreError> {
        let done = sqlx::query(
            "INSERT INTO character \
             (handle, display_name, base_persona, output_format_instruction, stat_scale, \
              stat_drift_mode) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(handle.as_str())
        .bind(display_name.as_str())
        .bind(base_persona.as_str())
        // **`&str` を直接受け取らない。** 検証を通っていない形式指示が
        // 削れない枠へ届く経路を型として作らない（`Persona` と同じ）。
        .bind(output_format_instruction.map(OutputFormat::as_str))
        .bind(stat_scale.get())
        .bind(stat_drift_mode.map(StatDriftMode::label))
        .execute(&self.pool)
        .await
        .map_err(StoreError::Query)?;

        let id = done.last_insert_rowid();
        tracing::debug!(character_id = id, "キャラクターを作成しました");
        Ok(CharacterId::from_row(id))
    }

    /// 全キャラクターを名前順で返す（`GET /admin/characters` と `GET /v1/models` が使う）。
    ///
    /// **読めなかった行も返す**（ADR-0016 §3 / 作業単位 0012b）。詳しくは
    /// `CharacterEntry` の doc にある。ここに書くのは「なぜその形が要るか」だけ。
    ///
    /// # 一覧は**修復の入口**である
    ///
    /// 更新 API（`update_character`）は**識別子で引く**ので、
    /// **どの行が壊れているかを知る手段が無いと修復に届かない。**
    /// この関数が返す `UnreadableCharacter` が、その唯一の導線になる
    /// （壊れた行は `find_character_by_handle` では読めない）。
    ///
    /// # 以前は「1件でも壊れていたら丸ごと失敗」だった
    ///
    /// **検証を後から厳しくすると、保存済みの行が不正になる**（0009 / 0009a で実際に起きた）。
    ///
    /// * 0009 以前は `"\u{2028}"` の表示名が `POST /admin/characters` から作れた。
    /// * 0009a 以前は `"\u{2000}"`（EN QUAD）や `"\u{200B}"`（ZWSP）だけの
    ///   表示名・ペルソナ・形式指示が同じ経路から作れた。
    ///
    /// これを `?` で返していたため、そうしたキャラクターを1体でも作っていた利用者は
    /// `GET /admin/characters` と `GET /v1/models` が**全キャラクターぶん 500** になり、
    /// **正常なキャラクターの一覧すら見られない状態で、修復手段が DB の直接編集しかなかった。**
    ///
    /// **かといって黙って読み飛ばすのも駄目。** 名前が正常な行を落とすと
    /// 「一覧に出ない・チャットは 404・しかし作ろうとすると『既に存在します』」という
    /// **ADR-0004 が潰した袋小路がそのまま復活する。**
    ///
    /// # **救ったのは `parse` 由来の壊れ方だけ**（「もう落ちない」ではない）
    ///
    /// 列の**デコード**に失敗する行は、今でも一覧を丸ごと落とす。下の `try_get` の `?` が
    /// そのまま出口で、たとえば `created_at` に日時として読めない値が入っていると
    /// `StoreError::Query(ColumnDecode { index: "created_at", .. })` になり、
    /// **正常な行を含めて1件も返らない**（0012b のレビューで実測）。
    ///
    /// **構造的な限界であって、書き忘れではない。** `character_entry` は
    /// `created_at: DateTime<Utc>` を引数に取る形なので、
    /// **`created_at` が壊れた行は `Unreadable` として表現することすらできない**
    /// （`UnreadableCharacter` は id と名前と列名しか持たない）。
    ///
    /// 直していないのは**到達性が2段違う**から。`parse` 由来の壊れ方は
    /// `/admin/` から作れた（0009 以前の表示名）のに対し、こちらは
    /// DB の直接編集か、外部で作った DB を持ち込んだ場合にしか現れない。
    ///
    /// # **1件を指す読み出し（`find_character_by_handle`）は今までどおり止める**
    ///
    /// あちらは「このキャラクターで話す」ための経路なので、壊れた値を握り潰して
    /// 応答を続けると**キャラクターの振る舞いだけが黙って変わる。**
    /// 一覧は「どの行があるか」を見るための経路で、性質が違う。
    ///
    /// # **`memory` とは方針が逆**（揃え忘れではない）
    ///
    /// `src/store/memory.rs` の `memory_from_row` は、不正な行を1件でも見つけたら
    /// `InvalidStoredValue` を返して**そのキャラクターの想起を全滅させる。**
    /// 記憶は1件落としてもリクエストが成功してしまい、変わるのは
    /// **キャラクターの振る舞いだけ**（「なんとなく前と違う」としか観測できない）。
    /// 一覧は落ちた行が**そのまま画面に出る**ので、指し示せば必ず気付ける。
    /// 片方に揃えないこと。詳しい理由は `memory_from_row` の doc にある。
    ///
    /// # 検証を厳しくするときに考えること
    ///
    /// **既存データを詰めるマイグレーションが要るかどうかを必ず先に考える。**
    /// `MAX_CHARS` を「後から下げられない」としているのと同じ話が、
    /// 文字集合の側にも当てはまる（一覧から修復できるようになった今でも、
    /// **識別子そのものが壊れた行は API からは直せない**）。
    pub async fn list_characters(&self) -> Result<Vec<CharacterEntry>, StoreError> {
        let rows = sqlx::query(SELECT_ALL)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
            let stored_handle: String = row.try_get("handle").map_err(StoreError::Query)?;
            let stored_display_name: String =
                row.try_get("display_name").map_err(StoreError::Query)?;
            let stored_persona: String = row.try_get("base_persona").map_err(StoreError::Query)?;
            let stored_format: Option<String> = row
                .try_get("output_format_instruction")
                .map_err(StoreError::Query)?;
            // **`try_get::<u32>` にしない**（`stat_allocs` の `value` と同じ理由）。
            // スキーマの CHECK は `PRAGMA ignore_check_constraints` で外せるので、
            // 範囲外の整数が格納されうる。`u32` で直接デコードすると
            // `StoreError::Query(ColumnDecode ..)` になって**一覧が丸ごと落ち**、
            // 「読めない行として指し示す」経路に入らない。
            //
            // **整数でない値（TEXT / REAL）が入っていた場合は、今でも一覧ごと落ちる。**
            // `created_at` のデコード失敗と同じ構造的な限界（`list_characters` の doc）。
            let stored_stat_scale: i64 = row.try_get("stat_scale").map_err(StoreError::Query)?;
            // **NULL は「未設定」**（ADR-0045 §決定1）。`parse` は `character_entry` が通す。
            //
            // **BLOB が入っていた場合は、今でも一覧ごと落ちる**（`stat_scale` の上の注記と同じ種類の
            // 構造的な限界 / 作業単位 A-8 のレビュー指摘で実測）。
            // **整数・小数は落ちない**——列が TEXT 宣言なので、CHECK を外して入れても
            // 型アフィニティで TEXT に変換されて入り（`typeof` = `text`）、`parse` を通らない行として
            // 指し示される。**変換されないのは BLOB だけ**で、それが `try_get::<Option<String>>` の
            // 型不一致（`StoreError::Query(ColumnDecode ..)`）になる。
            // 見張り番は `tests/store_character.rs::
            // a_drift_mode_that_is_not_text_is_pointed_at_unless_it_is_a_blob`。
            let stored_drift_mode: Option<String> =
                row.try_get("stat_drift_mode").map_err(StoreError::Query)?;
            // **`try_get::<u32>` にしない**（`stat_scale` と同じ理由。範囲外の整数を
            // 「読めない行」として指し示すため）。整数でない値が入っていれば、今でも一覧ごと落ちる。
            let stored_slots: i64 = row
                .try_get("axis_discovery_slots")
                .map_err(StoreError::Query)?;
            let created_at = row.try_get("created_at").map_err(StoreError::Query)?;
            // **NULL は「しまっていない」**（ADR-0049 §決定1）。日時に直すのは `character_entry`。
            // **BLOB は今でも一覧ごと落ちる**（`stat_drift_mode` と同じ構造的な限界。
            // `0017` の CHECK が TEXT 以外を止めるが、CHECK は外せる）。
            let stored_archived_at: Option<String> =
                row.try_get("archived_at").map_err(StoreError::Query)?;

            entries.push(character_entry(
                id,
                &stored_handle,
                &stored_display_name,
                &stored_persona,
                stored_format.as_deref(),
                stored_stat_scale,
                stored_drift_mode.as_deref(),
                stored_slots,
                created_at,
                stored_archived_at.as_deref(),
            ));
        }

        Ok(entries)
    }

    /// 識別子で1件引く。完全一致。見つからなければ `None`。
    pub async fn find_character_by_handle(
        &self,
        handle: &Handle,
    ) -> Result<Option<Character>, StoreError> {
        let row = sqlx::query(SELECT_BY_HANDLE)
            .bind(handle.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
        let stored_display_name: String = row.try_get("display_name").map_err(StoreError::Query)?;
        let stored_persona: String = row.try_get("base_persona").map_err(StoreError::Query)?;
        let stored_format: Option<String> = row
            .try_get("output_format_instruction")
            .map_err(StoreError::Query)?;
        // 理由は `list_characters` 側の同じ読み出しに書いてある。
        let stored_stat_scale: i64 = row.try_get("stat_scale").map_err(StoreError::Query)?;
        let stored_drift_mode: Option<String> =
            row.try_get("stat_drift_mode").map_err(StoreError::Query)?;
        let stored_slots: i64 = row
            .try_get("axis_discovery_slots")
            .map_err(StoreError::Query)?;
        let stored_archived_at: Option<String> =
            row.try_get("archived_at").map_err(StoreError::Query)?;
        Ok(Some(Character {
            id: CharacterId::from_row(id),
            // WHERE で完全一致させているので、格納値は引数と同一。
            handle: handle.clone(),
            // **表示名は引数から作れない。** 格納値を検証して返す（`base_persona` と同じ）。
            display_name: display_name_from_row(id, &stored_display_name)?,
            // **`None` に潰さない。** 「存在しない」と返すと、作成し直そうとしたときに
            // 409「既に存在します」と食い違い、原因に到達できなくなる（ADR-0004）。
            base_persona: persona_from_row(id, &stored_persona)?,
            output_format_instruction: output_format_from_row(id, stored_format.as_deref())?,
            stat_scale: stat_scale_from_row(id, stored_stat_scale)?,
            stat_drift_mode: stat_drift_mode_from_row(id, stored_drift_mode.as_deref())?,
            axis_discovery_slots: axis_discovery_slots_from_row(id, stored_slots)?,
            created_at: row.try_get("created_at").map_err(StoreError::Query)?,
            // **読めない時刻を「しまっていない」に落とさない**（`CharacterColumn::ArchivedAt` の doc）。
            archived_at: archived_at_from_row(id, stored_archived_at.as_deref())?,
        }))
    }

    /// 表示名・ペルソナ・出力形式指示を**部分更新**する（ADR-0016 / 作業単位 0012a）。
    ///
    /// # **既存の行を `parse` せずに書く**（この関数の存在理由）
    ///
    /// **`find_character_by_handle` を先に呼んではいけない。** あちらは全フィールドを
    /// `parse` するので、**直したい行ほど読めない。** 0009 以前に作れた U+2028 の表示名、
    /// 上限導入前の巨大なペルソナ——修復の対象はどれも `InvalidStoredValue` になる行であり、
    /// 読んでから書く実装は**修復の入口として機能しない。**
    /// 引き当ては SQL の WHERE 句だけで済ませ、書く値は引数の検証済みの型から取る。
    ///
    /// # 引き当ては**完全一致**（0004b-2 で正規化キーから切り替えた）
    ///
    /// 直せるのは「識別子は読めるが他の列が壊れている行」だけになった。
    /// **識別子そのものが読めない行（ゴースト行 / `"Aoi"` / `"葵"`）はここに届かない。**
    /// 一覧が `handle: null` で指し示すだけで、直す手段は `sqlite3` になる
    /// （理由と実測は `UPDATE_CHARACTER` の doc と
    /// `migrations/0006_drop_character_name_key_idx.sql`）。
    ///
    /// # 部分更新
    ///
    /// `CharacterUpdate` で指定されなかった列は**触らない**（既定値で潰さない）。
    /// 形式指示だけは NULL になりうるので3値（`FieldUpdate`）で受ける。
    /// 「変えない」と「消す」を同じ値にすると、表示名だけを変える更新のたびに
    /// 形式指示が黙って消える。
    ///
    /// # 識別子は変えられない
    ///
    /// 引数に新しい `Handle` を受け取る口が無い（ADR-0016 §1）。
    /// `handle` は引き当てにしか使わず、UPDATE の SET には現れない。
    ///
    /// 対象が無ければ `StoreError::UnknownCharacter`。
    ///
    /// # トランザクションを張る理由
    ///
    /// 更新した行の id を返すため、書き込みの後に引き直している。
    /// 素で2文に分けると、その間に行が消えたときに
    /// **更新は成功したのにエラーを返す**（あるいは別の行の id を返す）。
    /// 最初の文が書きなので `BEGIN IMMEDIATE` は要らない（`append_message` と同じ）。
    pub async fn update_character(
        &self,
        handle: &Handle,
        update: &CharacterUpdate,
    ) -> Result<CharacterId, StoreError> {
        // 「変えない / 設定する / 消す」を (置き換えるか, 新しい値) に落とす。
        // **`Keep` を `Clear` と同じ組にしない。** そこが3値である理由そのもの。
        let (replaces_format, new_format) = match &update.output_format_instruction {
            FieldUpdate::Keep => (false, None),
            FieldUpdate::Set(value) => (true, Some(value.as_str())),
            FieldUpdate::Clear => (true, None),
        };
        // 走らせ方も同じ3値（ADR-0045 §決定3）。**書くのは `label` の綴りだけ。**
        let (replaces_drift_mode, new_drift_mode) = match update.stat_drift_mode {
            FieldUpdate::Keep => (false, None),
            FieldUpdate::Set(mode) => (true, Some(mode.label())),
            FieldUpdate::Clear => (true, None),
        };

        let mut tx = self.pool.begin().await.map_err(StoreError::Query)?;

        let done = sqlx::query(UPDATE_CHARACTER)
            // **`&str` を直接受け取らない**（`create_character` と同じ）。
            // `None` は「その列に触らない」の意味で、`coalesce` が現在値を残す。
            .bind(update.display_name.as_ref().map(DisplayName::as_str))
            .bind(update.base_persona.as_ref().map(Persona::as_str))
            .bind(replaces_format)
            .bind(new_format)
            // **`?5`。**
            .bind(update.stat_scale.map(StatScale::get))
            // **`?6` / `?7`。** `handle` は `?8` へずれている（`UPDATE_CHARACTER` の doc）。
            .bind(replaces_drift_mode)
            .bind(new_drift_mode)
            .bind(handle.as_str())
            // **`?9`**（`handle` の後ろ / `UPDATE_CHARACTER` の doc）。`None` は「変えない」。
            .bind(update.axis_discovery_slots.map(AxisDiscoverySlots::get))
            // **`?10`**（しまう／戻す / ADR-0049 §決定1）。`None` は「変えない」。
            .bind(update.archived)
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        if done.rows_affected() == 0 {
            // **0件だった理由は同じトランザクションの中で引き直す**（`delete_character` と同じ）。
            // 外に出すと、その間に行が消えたり軸が変わったりして理由が入れ替わる。
            //
            // **行の有無だけを見る**（他の列を `parse` しない）。更新は修復の入口なので、
            // 壊れた行についても正しい理由を返せなければならない。
            let found = sqlx::query(SELECT_ID_BY_HANDLE)
                .bind(handle.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(StoreError::Query)?;

            // 名前はログにも出さない（ここは検証済みの値だが、出す必要が無い）。
            // commit しないので何も残らない。
            return Err(if found.is_some() {
                // WHERE の残る条件は `NOT EXISTS`（尺度）だけなので、
                // **行があるのに0件＝尺度が保存済みの値より小さい**。
                tracing::warn!(
                    "保存されている性格ステータスの値より小さい尺度への更新を拒否しました"
                );
                StoreError::StatScaleBelowStoredValue
            } else {
                tracing::warn!("存在しないキャラクターの更新を拒否しました");
                StoreError::UnknownCharacter
            });
        }

        // 更新した行の id。**書きロックを取った後の読みなので昇格の問題は起きない。**
        let row = sqlx::query(SELECT_ID_BY_HANDLE)
            .bind(handle.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        let id: i64 = row.try_get("id").map_err(StoreError::Query)?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(character_id = id, "キャラクターを更新しました");
        Ok(CharacterId::from_row(id))
    }

    /// キャラクターを削除する。**しまっていない・会話のあるキャラクターは拒否する**
    /// （ADR-0016 §2 を ADR-0049 §決定1 が広げた / 作業単位 A-10）。
    ///
    /// | しまってある | 会話 | 結果 |
    /// |---|---|---|
    /// | いいえ | 無い | 消す（作り損ないの掃除。ADR-0016 のまま） |
    /// | いいえ | ある | `CharacterHasSessions`（1行も消えない） |
    /// | はい | 問わない | **消す。会話・記憶・性格値なども全部**（カスケード） |
    ///
    /// **完全に消す前に必ず「しまう」を通る**ので、一覧から1回の誤操作で中身を失う事故は起きない
    /// （ADR-0049 §決定1「2段にする理由」）。
    ///
    /// # 数えてから消さない
    ///
    /// 判定（会話が無いこと）と削除は**1文**で行う（`DELETE_CHARACTER`）。
    /// `SELECT count(*)` と `DELETE` に分けると、その隙に始まった会話が
    /// `ON DELETE CASCADE` で `session` → `message` ごと**黙って消える。**
    /// 逐次のテストでは差が出ないので、実装を分けても気付けない
    /// （`deleting_never_races_a_starting_conversation` が同時実行で見張っている）。
    ///
    /// # 「該当なし」と「会話がある」を区別する
    ///
    /// 0件だった理由は**同じトランザクションの中で**引き直す。外に出すと、その間に
    /// 状態が変わって理由が入れ替わる（打ち間違いと仕様上の拒否は対処が正反対）。
    ///
    /// # **素の `BEGIN` でよい**（`BEGIN IMMEDIATE` は要らない）
    ///
    /// **1文目の DELETE が、0行しか消さない場合でも書きロックを取るため。**
    /// `latest_or_create_session` が `BEGIN IMMEDIATE` を要るのは「読んでから書く」形で
    /// 昇格が `SQLITE_BUSY_SNAPSHOT`(517) になるからだが、ここは最初の文が書きなので
    /// その形にならない（`append_message` と同じ）。続く「理由の引き直し」は、
    /// **既に書きロックを握った後の読み**である。
    ///
    /// 実測（sqlite3 3.53.4 / WAL。先行接続が `BEGIN;` の後に1文実行し、
    /// 別接続が `BEGIN IMMEDIATE` を試みる）:
    ///
    /// ```text
    /// DELETE ... WHERE id = 999（0行）  → 別接続は database is locked
    /// DELETE ... WHERE id = 1  （1行）  → 別接続は database is locked
    /// SELECT count(*)                   → 別接続は書きロックを取れる
    /// ```
    ///
    /// **「0行の DELETE は書きロックを取らない」は偽。** 以前ここには
    /// その前提で `BEGIN IMMEDIATE` を使う理由が書いてあったが、上のとおり成り立たない。
    /// **誤った理由を残す方が、理由が無いより危ない**（確かめた人が理由ごと
    /// 周辺の仕掛けを消しにくる）。
    ///
    /// 対象が無ければ `StoreError::UnknownCharacter`、
    /// しまっておらず会話があれば `StoreError::CharacterHasSessions`。**どちらの場合も1行も消えない。**
    ///
    /// # 子テーブルが増えたときのカスケード
    ///
    /// 削除は `character` を1行消すだけだが、キャラクターを指す表は全部 `ON DELETE CASCADE`
    /// なので**黙って連れて消える。** 記憶も消える —— **「記憶を消さない」は直すときの規則で、
    /// キャラクターを丸ごと捨てる操作には及ばない**（ADR-0049「記憶を消さない制約との線引き」）。
    ///
    /// **表を足したら人の目が入る。** `tests/store_character_archive.rs::
    /// a_put_away_character_is_deleted_with_every_row_that_points_at_it` が、
    /// 数える表の集合を `sqlite_master` と突き合わせている（`tests/store_schema.rs` の
    /// 表の完全一致リストも同じく赤くなる）。そこで「この表も消えてよいのか」を決めること。
    pub async fn delete_character(&self, handle: &Handle) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(StoreError::Query)?;

        let done = sqlx::query(DELETE_CHARACTER)
            .bind(handle.as_str())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        if done.rows_affected() == 0 {
            // **理由の引き直しも同じトランザクションの中。** 行の有無だけを見るので、
            // ここでも他の列を `parse` しない（壊れた行も掃除できなければならない）。
            let found = sqlx::query(SELECT_ID_BY_HANDLE)
                .bind(handle.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(StoreError::Query)?;

            // commit しない（1行も書いていないのでロールバックで足りる）。
            //
            // **行があるのに0件＝しまっておらず、会話がある**（WHERE の残る条件はそれだけ）。
            return Err(if found.is_some() {
                tracing::warn!("しまっていない、会話のあるキャラクターの削除を拒否しました");
                StoreError::CharacterHasSessions
            } else {
                tracing::warn!("存在しないキャラクターの削除を拒否しました");
                StoreError::UnknownCharacter
            });
        }

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::info!("キャラクターを削除しました");
        Ok(())
    }

    /// そのキャラクターの性格ステータスを**丸ごと置き換える**。
    ///
    /// 削除と挿入は**同一トランザクション**で行う。分けると、失敗したときに
    /// 「軸が全部消えたキャラクター」が残る。空の `allocs` を渡すと全軸を消す。
    ///
    /// 同一軸を重複して渡した場合は主キー違反になり、置き換え全体がロールバックされる。
    ///
    /// **存在しない `character_id` でも、`allocs` が空なら成功する。** DELETE が0件で成功し
    /// INSERT が1件も走らないため外部キーに触れない。DELETE の冪等性として許容している
    /// （`tests/store_character.rs::stat_allocs_of_unknown_character` で固定）。
    ///
    /// # **保存されている尺度で作られた値だけを受け付ける**（ADR-0015 / 作業単位 0011a-2）
    ///
    /// `StatValue` は「ある尺度を超えないこと」しか覚えていない。
    /// **その尺度がこのキャラクターのものかは型では守れない**
    /// （`3/100` と `3/10` は別の性格なのに、どちらも `3` として通る）。
    /// だからここで**保存済みの尺度と突き合わせる。**
    ///
    /// **これは「値 ≤ 尺度」の検査を兼ねる。** `StatValue` が自分の尺度以下であることは
    /// 構築時に保証済みなので、尺度が一致すれば値も収まっている。**二重に書かない。**
    ///
    /// # 尺度はトランザクションの中で読む。**ただし `BEGIN IMMEDIATE` で入る**
    ///
    /// 外で読むと、その間に `PATCH` が尺度を変えて
    /// **検査した尺度と書き込む先の尺度が別物**になる。
    ///
    /// **中で読むなら開始文が要る。** 素の `BEGIN` のまま読みから始めると
    /// 書きへの昇格が `SQLITE_BUSY_SNAPSHOT`(517) になり、
    /// **`busy_timeout` を待たずに `database is locked` で落ちる**
    /// （実測と壊れ方は `BEGIN_IMMEDIATE` の doc）。
    ///
    /// # **既存の軸は読まない**（修復の入口を塞がない）
    ///
    /// 尺度を超える値が既に保存されていても、この関数は届く。
    /// `stat_allocs` は1行壊れると丸ごと失敗するので、
    /// **`stat_alloc` 側が壊れた行の復旧経路はここだけ**（見張り番は
    /// `tests/store_character.rs::a_stored_value_beyond_the_characters_scale_stops_the_read`）。
    ///
    /// # **`character.stat_scale` 列が壊れた行には届かない**（0011a-2 のレビューで実測）
    ///
    /// 軸は読まないが**尺度は読む**（突き合わせに要る）ので、その1列を
    /// `stat_scale_from_row` に通す。**尺度の列そのものが壊れていると、
    /// 突き合わせに入る前に `InvalidStoredValue` で止まる。**
    /// **その行を直す入口は `PATCH`（`update_character`）**——あちらは行を読まずに
    /// WHERE だけで書くので、壊れた尺度にも届く。
    ///
    /// **保存済みの値が大きいと3手になる。** 実測（2026-08-20 / 尺度の列を `0` に潰し、
    /// `stat_alloc` に `value = 50` を直接 INSERT した行。見張り番は
    /// `tests/store_character.rs::a_broken_stat_scale_column_puts_the_replacement_out_of_reach`）:
    ///
    /// ```text
    /// stat_allocs(id)                       → Err(InvalidStoredValue)
    /// replace_stat_allocs(id, [1/7])        → Err(InvalidStoredValue)   置き換えが届かない
    /// update_character(stat_scale = 20)     → Err(StatScaleBelowStoredValue)
    /// update_character(stat_scale = 100)    → Ok      ← 1手目: 保存済みの値以上へ上げる
    /// stat_allocs(id)                       → Ok([ロジック = 50/100])
    /// replace_stat_allocs(id, [1/7])        → Err(StatScaleMismatch)
    /// replace_stat_allocs(id, [1/100])      → Ok      ← 2手目: 新しい尺度で入れ直す
    /// update_character(stat_scale = 7)      → Ok      ← 3手目: 目的の尺度まで下げる
    /// ```
    ///
    /// **「先に尺度を下げてから入れ直す」順序は取れない**（2行目・6行目）。
    /// ADR-0015 の ⚠ もこの制限に触れているが、**手順の正はここ。**
    pub async fn replace_stat_allocs(
        &self,
        character_id: CharacterId,
        allocs: &[StatAlloc],
    ) -> Result<(), StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        // **行が無ければ検査しない。** 尺度が存在しないので突き合わせる相手が無く、
        // `allocs` が空なら成功（DELETE の冪等性）、空でなければ続く INSERT が
        // 外部キー違反で止める。**ここで `UnknownCharacter` を名乗らない**のは、
        // 「宛先が無い」ことの報告を1箇所（外部キー）に保つため。
        let stored_scale: Option<i64> = sqlx::query_scalar(SELECT_STAT_SCALE_BY_ID)
            .bind(character_id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        if let Some(stored_scale) = stored_scale {
            let scale = stat_scale_from_row(character_id.get(), stored_scale)?;
            if allocs.iter().any(|alloc| alloc.value.scale() != scale) {
                // **尺度も値もログに出さない**（他の拒否と同じ。手掛かりは id）。
                tracing::warn!(
                    character_id = character_id.get(),
                    "保存されている尺度と違う尺度で作られた性格ステータスの値を拒否しました"
                );
                // commit しないので1行も書き換わらない。
                return Err(StoreError::StatScaleMismatch);
            }
        }

        sqlx::query("DELETE FROM stat_alloc WHERE character_id = ?")
            .bind(character_id.get())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        for alloc in allocs {
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(character_id.get())
                .bind(alloc.axis.as_str())
                // **尺度は書かない。** 尺度は `character` の列で、軸ごとには持たない。
                .bind(alloc.value.get())
                .execute(&mut *tx)
                .await
                .map_err(StoreError::Query)?;
        }

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            axes = allocs.len(),
            "性格ステータスを置き換えました"
        );
        Ok(())
    }

    /// そのキャラクターの性格ステータスを**設定時の順序**で返す。
    ///
    /// `ORDER BY rowid` は挿入順。`replace_stat_allocs` が丸ごと入れ直すため、
    /// 呼び出し側が渡した並びがそのまま返る（プロンプトへの並びを呼び出し側で決められる）。
    ///
    /// **値 0 の軸も返す。** 落とすのは `prompt` 層の責務。
    ///
    /// 前後に空白のある軸名が格納されていた場合は正規化して返す。
    /// 正規化しても不正な値なら `StoreError::InvalidStoredValue`。
    ///
    /// **原因を「DB の直接編集」に限定しないこと**（作業単位 0009 / 0009a）。
    /// 0009a 以前は `"\u{2000}"` の軸名を `/admin/` から書けたので、
    /// **適用後にそのキャラクターの性格ステータス取得が丸ごと止まる。**
    /// 詳細は `Store::list_characters` の doc。
    ///
    /// # 値は**そのキャラクターの尺度**を通して組み立てる（ADR-0015 / 作業単位 0011a-2）
    ///
    /// 尺度を超える値は `InvalidStoredValue`。**スキーマの CHECK は
    /// 下限（0以上・整数型）しか見ない**（尺度と値を結ぶ制約はどこにも無い）ので、
    /// 直接 INSERT すれば範囲外の行は作れる。
    ///
    /// **1行壊れると丸ごと失敗する。** これは `memory_from_row` と同じ方針で、
    /// **復旧は `replace_stat_allocs` = `PUT /admin/characters/{handle}/stats`。**
    ///
    /// **⚠ ここには「既知の制約として `docs/06-roadmap.md` に積んである」と書いてあったが、
    /// 該当する項目は無かった**（B-19 のレビューが5つの綴りで走査 / 2026-08-28。
    /// **B-19 以前からの誤り**）。**積む先は「性格ステータスの修復導線」**
    /// （`docs/06-roadmap.md` / 司令塔が起票）で、
    /// **画面から辿れないこと**（下の「API の入口は閉じない…」）も同じ項目にまとめる。
    ///
    /// # **正規化すると同じ軸になる行が2つあれば、それも「読めない」**（作業単位 B-19）
    ///
    /// `stat_alloc` の主キーは `(character_id, axis)` なので **DB は一意のつもりでいる。**
    /// ところが `axis` は**生のテキスト**なので `パッション` と ` パッション` は同居でき、
    /// **一意でなくしているのはこの読みの正規化**である。
    ///
    /// 受理していたときに何が起きたか（B-16 のレビューの実測）——
    /// `GET .../drift` が**同じ集計を2回引き当て**、`ups + downs` の総和（6）が
    /// 票の総数（3）を超えた。`GET .../stats` の側でも同じ軸が2行並び、
    /// **どちらの値が本当かを応答から決められない。**
    ///
    /// ## 却下した案
    ///
    /// * **落として1行にする**（先に来た行を採る）——**消した方の値が応答のどこにも現れない。**
    ///   このリポジトリは一貫して「存在する行を落とさずに指し示す」立場を取る
    ///   （孤児の `LEFT JOIN`・`unreadable`・値 0 の軸・上書きされた記憶）。
    ///   **どちらを残すかの根拠も無い。**
    /// * **正規化をやめる**（`parse` の結果と綴りが違う行を拒否する）——**重複していない
    ///   ` パッション` 1行まで読めなくなる。** 「軸は照合キーではないので、
    ///   取りこぼすより正規化して返す方が安全」（作業単位 0009）を覆すうえ、
    ///   **重複した状態そのものは相変わらず作れる**（拒否の理由が変わるだけ）。
    ///
    /// ## 止まる範囲は「この読みを通る全部」
    ///
    /// 2つの管理入口だけでなく、**本文のプロンプト組立**（`server::chat` は 500）と
    /// **変容の依頼**（`server::drift` は `StatsUnreadable` として飛ばす）も止まる。
    /// **2つの入口の意味が食い違わないのは、検査がこの1箇所にあるから**である
    /// ——呼び出し側へ写すと、片方だけが落ちても緑のまま（CLAUDE.md `D-6`）。
    ///
    /// ## **API の入口は閉じない。ただし画面からは辿れない**
    ///
    /// `replace_stat_allocs`（= `PUT .../stats`）は**既存の軸を読まない**ので届き、
    /// 入れ直す軸は `StatAxis` を通っているので**正規化の上でも一意**になる
    /// （`PUT` の応答は読み直したものなので、**この読みをもう一度通っている**）。
    /// 見張り番は `tests/store_character.rs::a_duplicated_axis_can_still_be_replaced` と
    /// `tests/server_admin_drift.rs::
    /// two_rows_that_normalize_to_the_same_axis_stop_both_endpoints_until_the_stats_are_replaced`。
    ///
    /// **⚠ 真なのは API についてだけ**（B-19 のレビューの実測 / 2026-08-28）。
    /// **画面（`web/src/lib/StatsPanel.svelte`）は `statsState.kind === "failed"` の枝で
    /// エラーの `<p>` しか描かない。** 「軸を追加」も「この内容で置き換える」も
    /// **`{:else}` 側のフォームの中にしか無く、`replaceStats` の呼び出し元はその1箇所だけ**なので、
    /// **この状態の画面からは `PUT` を出せない**（残るのは再読み込み・一覧・削除）。
    /// **その振る舞いは FE 側で「期待される形」として固定済み**
    /// （`web/tests/stats-panel.test.ts` の
    /// 「読めなかったとき」→「いまの値が読めなくても、変容の側は出る（逆も同じ）」）。
    ///
    /// **つまり利用者が自力で直すには `curl` か `sqlite3` が要る。**
    /// **画面からの修復導線は積むだけで、この作業単位では実装していない**
    /// （上の「復旧は…」と同じ項目）。
    pub async fn stat_allocs(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<StatAlloc>, StoreError> {
        let rows = sqlx::query(SELECT_STAT_ALLOCS)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut allocs = Vec::with_capacity(rows.len());
        for row in rows {
            let stored: String = row.try_get("axis").map_err(StoreError::Query)?;
            // 軸名は保存時に正規化済み。直接編集された行は正規化して受理し、
            // 正規化しても不正な値（制御文字入り等）は握り潰さずエラーにする。
            // キャラクター識別子と違い軸は照合キーではないため、取りこぼすより正規化して返す方が安全。
            let axis = StatAxis::parse(&stored).map_err(|_| StoreError::InvalidStoredValue)?;

            // **正規化すると同じ軸になる行が2つ以上あれば、この読みは丸ごと止まる**
            // （作業単位 B-19。理由と却下した案はこの関数の doc）。
            //
            // **`allocs` の全部と突き合わせる。** 直前の1行とだけ比べると、
            // 間に別の軸を挟んだ重複（`ORDER BY rowid` は挿入順なので普通に起きる）を
            // 見落とす。軸の件数は小さい（ADR-0024 §1）ので線形の探索で足りる
            // ——集合を組むより**読んで分かる形**を選ぶ（`server::admin::unallocated_axes`
            // と同じ判断）。
            if allocs.iter().any(|kept: &StatAlloc| kept.axis == axis) {
                // 軸名も値もログに出さない——**他の拒否（尺度不一致・孤児・範囲外）と
                // 揃えた作法**であって、見張り番があるわけではない
                // （絶対制約は「ログには出してよい」側。手掛かりは id）。
                tracing::warn!(
                    character_id = character_id.get(),
                    "正規化すると同じになる性格ステータスの軸が複数あります。置き換えで直せます"
                );
                return Err(StoreError::InvalidStoredValue);
            }

            // **親の行が無ければ尺度が NULL になる**（`SELECT_STAT_ALLOCS` は `LEFT JOIN`）。
            // 黙って読み飛ばさない——外部キーが無効な接続で作られた孤児を隠すことになる。
            let stored_scale: Option<i64> = row.try_get("stat_scale").map_err(StoreError::Query)?;
            let Some(stored_scale) = stored_scale else {
                tracing::warn!(
                    character_id = character_id.get(),
                    "親のいない性格ステータスの行があります。データベースを直接確認してください"
                );
                return Err(StoreError::InvalidStoredValue);
            };
            let scale = stat_scale_from_row(character_id.get(), stored_scale)?;

            // **`try_get::<u32>` にしない。** スキーマの CHECK は下限（0以上・整数型）しか
            // 見ないため `4294967296` が格納されうる。`u32` で直接デコードすると
            // `StoreError::Query(ColumnDecode ..)` になり、直上の軸名と分類が割れる
            // （どちらも「DB 直接編集」由来）。ログを見た人がクエリ側を疑う遠回りを避ける。
            let stored_value: i64 = row.try_get("value").map_err(StoreError::Query)?;
            // **上側は `scale.value` と重なる。** `u32` に収まらない大きな値は
            // 尺度（上限 100）にも収まらないので、そちら側では
            // `try_from` を素通しにしても答えは変わらない（実測: `.or(Some(u32::MAX))` は等価）。
            //
            // **下側は重ならない。** 負の値を弾いているのは `try_from` **だけ**で、
            // `StatScale::value` は `u32` しか受け取らないので**そこには届かない。**
            // 負値を潰して受理すると（`stored_value.max(0)`）`-5` は `0` として読め、
            // **値 0 の軸は `prompt` が丸ごと落とす**ので、その軸だけが静かに消える。
            // 見張り番は `tests/store_character.rs::
            // a_negative_stored_value_stops_the_read_instead_of_being_read_as_zero`。
            let value = u32::try_from(stored_value)
                .ok()
                .and_then(|raw| scale.value(raw).ok())
                .ok_or_else(|| {
                    tracing::warn!(
                        character_id = character_id.get(),
                        "尺度に収まらない性格ステータスの値があります。置き換えで直せます"
                    );
                    StoreError::InvalidStoredValue
                })?;
            allocs.push(StatAlloc { axis, value });
        }

        Ok(allocs)
    }

    /// **DB にある全キャラクターの、性格の軸の本数の最大**（作業単位 B-60d-1 のレビュー）。
    ///
    /// 本文の数えた計画が、同じ上流で走る裏方の1要求分を差し引くときに使う
    /// （`server::counted_plan::background_request`）。本文と重なるのは**直前のターンの裏方**で、
    /// それは**別のキャラクターの仕事でありうる**（裏方の枠はキャラクターをまたいで1つ）ので、
    /// そのキャラクターの本数ではなく最大を取る。
    ///
    /// | 数えるもの | 理由 |
    /// |---|---|
    /// | **値 0 の軸も数える** | 変容の依頼には値 0 の軸も載る（ADR-0027 §決定4） |
    /// | **しまったキャラクターも数える** | しまう直前に始まったターンの裏方は、しまった後も走りうる（ターンは断るが、走り出した裏方は止めない）。多めに数えても遅くなるだけ |
    /// | 親のいない行も数える（`stat_alloc` だけを見る） | 同上（多めの側） |
    ///
    /// **1文の集計**（入力は無いので束縛する値も無い）。軸の行を読み下さないので、
    /// 壊れた行（読めない軸名・範囲外の値）があっても止まらない —— 本数だけを数える。
    pub async fn max_stat_axes(&self) -> Result<usize, StoreError> {
        let row = sqlx::query(SELECT_MAX_STAT_AXES)
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        let axes: i64 = row.try_get("axes").map_err(StoreError::Query)?;
        Ok(usize::try_from(axes).unwrap_or(0))
    }
}

/// [`Store::max_stat_axes`] の集計。**キャラクターごとの行数の最大**（行が無ければ 0）。
const SELECT_MAX_STAT_AXES: &str = "SELECT COALESCE(MAX(axes), 0) AS axes \
     FROM (SELECT COUNT(*) AS axes FROM stat_alloc GROUP BY character_id)";

/// 1行を「読めた行」か「読めなかった行」に振り分ける（ADR-0016 §3 / 作業単位 0012b）。
///
/// # 途中で打ち切らない
///
/// **壊れている列を全部数えてから振り分ける。** 最初の1つで抜けると
/// 「表示名を直したのにまだ読めない」を繰り返すことになり、修復が何往復も掛かる。
///
/// # 読めた行は `parse` の結果だけから組み立てる
///
/// `invalid_columns` が空かどうかでは分岐しない。**分岐の根拠を1本にしておく**と、
/// 「壊れているのに読めた行として返す」状態が構造として作れない
/// （数え漏れがあっても、その行は読めない側に落ちる）。
#[expect(
    clippy::too_many_arguments,
    reason = "格納値を列ごとに受ける形のまま、A-8 で走らせ方の列、A-9a で枠の数の列、A-10 でしまった時刻の列が増えた"
)]
fn character_entry(
    id: i64,
    stored_handle: &str,
    stored_display_name: &str,
    stored_persona: &str,
    stored_format: Option<&str>,
    stored_stat_scale: i64,
    stored_drift_mode: Option<&str>,
    stored_slots: i64,
    created_at: DateTime<Utc>,
    stored_archived_at: Option<&str>,
) -> CharacterEntry {
    let mut invalid_columns = Vec::new();

    // **識別子は `parse` を通るかどうかだけで決まる**（0004b-2）。
    //
    // 以前はここが2段だった——`parse` を通したうえで、格納値が**正規化済みである**
    // ことまで確かめていた（`normalized_name`）。**ADR-0017 で `parse` が
    // 正規化をやめたので、後段は常に真になり死んだ検査になっていた**
    // （`Handle::parse(raw)` は `raw` をそのまま保持するので
    // `handle.as_str() == stored` が成り立たない入力が存在しない）。
    //
    // 分岐の根拠が1本になったので、「読める識別子」と「指せる識別子」も一致する。
    // 見張り番は `tests/store_character.rs::
    // a_row_that_parses_is_exactly_a_row_the_repair_api_can_reach`。
    let handle = Handle::parse(stored_handle).ok();
    if handle.is_none() {
        // 格納名はログにも出さない（何が入っているか分からない）。手掛かりは id。
        invalid_columns.push(CharacterColumn::Handle);
    }

    let display_name = DisplayName::parse(stored_display_name).ok();
    if display_name.is_none() {
        invalid_columns.push(CharacterColumn::DisplayName);
    }

    let base_persona = Persona::parse(stored_persona).ok();
    if base_persona.is_none() {
        invalid_columns.push(CharacterColumn::BasePersona);
    }

    // **NULL は「未設定」であって不正ではない**（`Some(None)` = 読めた・値は無し）。
    let output_format_instruction = match stored_format {
        None => Some(None),
        Some(stored) => OutputFormat::parse(stored).ok().map(Some),
    };
    if output_format_instruction.is_none() {
        invalid_columns.push(CharacterColumn::OutputFormatInstruction);
    }

    // **スキーマの CHECK があっても `parse` を省かない。** CHECK は
    // `PRAGMA ignore_check_constraints` で外せるので、範囲外の値は格納されうる。
    let stat_scale = u32::try_from(stored_stat_scale)
        .ok()
        .and_then(|value| StatScale::parse(value).ok());
    if stat_scale.is_none() {
        invalid_columns.push(CharacterColumn::StatScale);
    }

    // **NULL は「未設定」であって不正ではない**（形式指示と同じ形 / ADR-0045 §決定1）。
    // **CHECK があっても `parse` を省かない**（尺度と同じ理由）。
    let stat_drift_mode = match stored_drift_mode {
        None => Some(None),
        Some(stored) => StatDriftMode::parse(stored).ok().map(Some),
    };
    if stat_drift_mode.is_none() {
        invalid_columns.push(CharacterColumn::StatDriftMode);
    }

    // 枠の数（ADR-0046 §決定1）。**CHECK があっても `parse` を省かない**（尺度と同じ理由）。
    let axis_discovery_slots = u32::try_from(stored_slots)
        .ok()
        .and_then(|raw| AxisDiscoverySlots::parse(raw).ok());
    if axis_discovery_slots.is_none() {
        invalid_columns.push(CharacterColumn::AxisDiscoverySlots);
    }

    // **NULL は「しまっていない」であって不正ではない**（形式指示と同じ形 / ADR-0049 §決定1）。
    let archived_at = match stored_archived_at {
        None => Some(None),
        Some(stored) => parse_archived_at(stored).map(Some),
    };
    if archived_at.is_none() {
        invalid_columns.push(CharacterColumn::ArchivedAt);
    }

    match (
        handle,
        display_name,
        base_persona,
        output_format_instruction,
        stat_scale,
        stat_drift_mode,
        axis_discovery_slots,
        archived_at,
    ) {
        (
            Some(handle),
            Some(display_name),
            Some(base_persona),
            Some(output_format_instruction),
            Some(stat_scale),
            Some(stat_drift_mode),
            Some(axis_discovery_slots),
            Some(archived_at),
        ) => CharacterEntry::Readable(Character {
            id: CharacterId::from_row(id),
            handle,
            display_name,
            base_persona,
            output_format_instruction,
            stat_scale,
            stat_drift_mode,
            axis_discovery_slots,
            created_at,
            archived_at,
        }),
        _ => {
            // **格納値は出さない。列名だけを出す**（定数なので安全）。
            tracing::warn!(
                character_id = id,
                invalid_columns = ?invalid_columns,
                "読み出せないキャラクターの行があります。一覧が返す識別子で修復できます"
            );
            CharacterEntry::Unreadable(UnreadableCharacter {
                id: CharacterId::from_row(id),
                // **上の `match` が `handle` を消費しているので、ここでは作り直す。**
                // 値は同じ（`parse` は入力を書き換えない）。
                handle: Handle::parse(stored_handle).ok(),
                invalid_columns,
            })
        }
    }
}

/// 格納されている表示名を検証して返す。
///
/// **ペルソナと同じ扱い**（握り潰さず `InvalidStoredValue` で止める）。表示名も
/// 削れない枠に載るので、上限を超えた値を通すと以後のチャットが
/// `BudgetExhausted` になり続ける。空の値も同様に止める
/// （通すと `prompt` が `# 名前` の下に空の引用行を書く）。
///
/// **名前（識別子）とは扱いが違う。** あちらは正規化されていない行を「読めた行」に
/// 混ぜないが、それは「完全一致では**そもそも引けない**行」だから成立する話で、
/// 表示名は照合キーではない。前後の空白は正規化して受理する。
///
/// **格納値そのものはログにも出さない。** 手掛かりとして残すのは id だけ。
fn display_name_from_row(character_id: i64, stored: &str) -> Result<DisplayName, StoreError> {
    DisplayName::parse(stored).map_err(|_| {
        tracing::warn!(
            character_id,
            "表示名が不正なキャラクターがあります。データベースを直接確認してください"
        );
        StoreError::InvalidStoredValue
    })
}

/// 格納されているペルソナ本文を検証して返す。
///
/// **握り潰さない。** ここを `unwrap_or_default` のような形にすると、
/// ペルソナの無いキャラクターとして黙って応答し続ける（`prompt` は空の節を落とす）。
/// 名前の正規化と違って前後の空白は正規化して受理する（照合キーではないため）。
///
/// **格納値そのものはログにも出さない。** 上限超過の値は数千文字ありうるうえ、
/// 何が入っているか分からない。手掛かりとして残すのは id だけ。
fn persona_from_row(character_id: i64, stored: &str) -> Result<Persona, StoreError> {
    Persona::parse(stored).map_err(|_| {
        tracing::warn!(
            character_id,
            "ペルソナ本文が不正なキャラクターがあります。データベースを直接確認してください"
        );
        StoreError::InvalidStoredValue
    })
}

/// 格納されている出力形式指示を検証して返す。NULL は未設定として `None`。
///
/// **ペルソナと同じ扱い**（握り潰さず `InvalidStoredValue` で止める）。形式指示も
/// 削れない枠に載るので、上限を超えた値を通すと以後のチャットが
/// `BudgetExhausted` になり続ける。しかもここは「指示として従え」と宣言される
/// 唯一の節なので、想定外の長さの値は黙って流さない。
///
/// **空文字も `None` に潰さない。** 潰すと「未設定」の表現が NULL と `''` の
/// 2通りになり、`OutputFormat::parse` が空を拒否している意味が消える
/// （`''` は `store` 経由では書き込めず、DB を直接編集した場合にしか現れない）。
///
/// **格納値そのものはログにも出さない。** 手掛かりとして残すのは id だけ。
fn output_format_from_row(
    character_id: i64,
    stored: Option<&str>,
) -> Result<Option<OutputFormat>, StoreError> {
    let Some(stored) = stored else {
        return Ok(None);
    };

    OutputFormat::parse(stored).map(Some).map_err(|_| {
        tracing::warn!(
            character_id,
            "出力形式指示が不正なキャラクターがあります。データベースを直接確認してください"
        );
        StoreError::InvalidStoredValue
    })
}

/// 格納されている性格ステータスの尺度を検証して返す（ADR-0015 / 作業単位 0011a-1）。
///
/// **スキーマに CHECK があっても検証を省かない。** CHECK は
/// `PRAGMA ignore_check_constraints` で外せる（`tests/common` の抜け穴と同じ手）ので、
/// `0` や `101` の行は作れる。素通しにすると
/// **尺度の下限（`StatScale::MIN`）を守る理由がそのまま消える**
/// ——尺度 0 のキャラクターは、性格ステータスを設定してあるのに一行も載らなくなる。
///
/// **格納値そのものはログにも出さない**（他の列と同じ。手掛かりは id）。
fn stat_scale_from_row(character_id: i64, stored: i64) -> Result<StatScale, StoreError> {
    u32::try_from(stored)
        .ok()
        .and_then(|value| StatScale::parse(value).ok())
        .ok_or_else(|| {
            tracing::warn!(
                character_id,
                "性格ステータスの尺度が不正なキャラクターがあります。データベースを直接確認してください"
            );
            StoreError::InvalidStoredValue
        })
}

/// 格納されている変容の走らせ方を検証して返す。NULL は未設定として `None`（ADR-0045 §決定1）。
///
/// **尺度と同じ扱い**（CHECK があっても検証を省かない。握り潰さず `InvalidStoredValue` で止める）。
/// 壊れた値を「未設定」に落とすと、`on` にしたはずのキャラクターが既定で走り、
/// **変容は本文に出ない**（ADR-0024 §9）ので誰も気付けない。
///
/// **格納値そのものはログにも出さない**（他の列と同じ。手掛かりは id）。
fn stat_drift_mode_from_row(
    character_id: i64,
    stored: Option<&str>,
) -> Result<Option<StatDriftMode>, StoreError> {
    let Some(stored) = stored else {
        return Ok(None);
    };

    StatDriftMode::parse(stored).map(Some).map_err(|_| {
        tracing::warn!(
            character_id,
            "変容の走らせ方が不正なキャラクターがあります。データベースを直接確認してください"
        );
        StoreError::InvalidStoredValue
    })
}

/// 格納されている枠の数を検証して返す（ADR-0046 §決定1 / 作業単位 A-9a）。
///
/// **尺度と同じ扱い**（CHECK があっても検証を省かない。握り潰さず `InvalidStoredValue` で止める）。
/// 壊れた値を 0 に落とすと、枠を開けたはずのキャラクターで候補が黙って出なくなる。
///
/// **格納値そのものはログにも出さない**（他の列と同じ。手掛かりは id）。
fn axis_discovery_slots_from_row(
    character_id: i64,
    stored: i64,
) -> Result<AxisDiscoverySlots, StoreError> {
    u32::try_from(stored)
        .ok()
        .and_then(|raw| AxisDiscoverySlots::parse(raw).ok())
        .ok_or_else(|| {
            tracing::warn!(
                character_id,
                "軸を見いだす枠の数が不正なキャラクターがあります。データベースを直接確認してください"
            );
            StoreError::InvalidStoredValue
        })
}

/// しまった時刻の格納値を日時に直す（ADR-0049 §決定1 / 作業単位 A-10）。読めなければ `None`。
///
/// 書き手（[`UPDATE_CHARACTER`] の `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`）の綴りは RFC 3339 なので、
/// **RFC 3339 として読む**。それ以外の綴りは、DB を直接書いた場合にしか現れない。
fn parse_archived_at(stored: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stored)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// 格納されているしまった時刻を検証して返す。NULL は「しまっていない」として `None`。
///
/// **尺度と同じ扱い**（握り潰さず `InvalidStoredValue` で止める）。読めない時刻を
/// 「しまっていない」に落とすと、しまったはずのキャラクターと黙って話せてしまう。
///
/// **格納値そのものはログにも出さない**（他の列と同じ。手掛かりは id）。
fn archived_at_from_row(
    character_id: i64,
    stored: Option<&str>,
) -> Result<Option<DateTime<Utc>>, StoreError> {
    let Some(stored) = stored else {
        return Ok(None);
    };

    parse_archived_at(stored).map(Some).ok_or_else(|| {
        tracing::warn!(
            character_id,
            "しまった時刻が不正なキャラクターがあります。戻すと直ります"
        );
        StoreError::InvalidStoredValue
    })
}
