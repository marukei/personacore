//! **性格ステータスの変容を1軸だけ書き込む経路**
//! （[ADR-0024](../../docs/adr/0024-stat-drift.md) §7 / 作業単位 16b）。
//!
//! この層は**薄く保つ**。判断（許可リスト・対象外・飽和）は `domain::apply_drift` で
//! 済んでおり、ここは「[`DriftChange`] を1文にバインドして、動いたかどうかを返す」だけを担う。
//!
//! # なぜ `character.rs` の全置換を使わないのか（ADR-0024 §7）
//!
//! `character.rs` の全置換（`PUT /admin/characters/{handle}/stats` が使う DELETE + INSERT）は、
//! 変容に使うと「全部読む → 1つ変える → 全部書く」になり、
//! **読んでから書くまでの窓で入った他の軸の更新を巻き戻す。**
//! 巻き戻しは**エラーにならない**ので、運用中は
//! 「`PUT .../stats` で直したはずの軸が、次の往復で元へ戻る」としか観測できない。
//!
//! ここは**現在値を条件に含めた単文**なので、「読んでから書く」が1文に閉じる。
//! したがって
//!
//! * `BEGIN IMMEDIATE` が要らない（トランザクションを張らない）。
//! * 取りこぼしは **0 行更新として観測できる**（[`DriftWrite::NotMoved`]）。
//!
//! # 0 行更新は**エラーではない**
//!
//! 0 行になる原因は「窓の中で値が変わった」「尺度が下げられた」「宛先が無い」で、
//! **どれもこの1往復では正常な結果**である（変容は1ターン1段なので、
//! 動かなかったターンは「動かなかった」だけ）。
//! `Err` にすると、裏方のタスクに**本文と無関係な分岐**が増えるうえ、
//! **「DB が書けない」と「動かなかった」が同じ値になる。**
//! 前者だけを `StoreError::Query` に残す。
//!
//! # LLM の出力はここまで届かない
//!
//! [`DriftChange`] のフィールドは private で、唯一の構築経路は
//! `domain::apply_drift`（**保存済みの軸名との完全一致**という許可リスト）。
//! したがって SQL へ渡る軸名は**保存済みの綴り**であり、
//! しかも**プレースホルダ**で渡る（CLAUDE.md 絶対制約）。

use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError};
use crate::domain::{
    CharacterId, DriftApply, DriftChange, DriftDelta, DriftVote, SessionId, StatAxis,
};

/// **1軸だけを1段動かす文**（ADR-0024 §7）。
///
/// ```text
/// UPDATE stat_alloc SET value = 新しい値
///  WHERE 持ち主が一致 AND 軸名が一致 AND いまの値が読んだときの値と一致
///    AND 新しい値がそのキャラクターの尺度に収まる
/// ```
///
/// # 条件が4つある理由（どれも落とすと別の壊れ方をする）
///
/// | 条件 | 落とすと |
/// |---|---|
/// | `character_id = ?1` | **別のキャラクターの同名の軸まで動く**（軸名は任意の文字列で、綴りは重なる） |
/// | `axis = ?2` | そのキャラクターの**全軸**が同じ値になる |
/// | `value = ?3` | 「読んでから書く」の窓で入った更新を**黙って踏み潰す**（ADR-0024 §7 の「取りこぼしたら 0 行更新として観測できる」が消える） |
/// | 尺度の突き合わせ | **そのキャラクターのものではない尺度で作られた値**が入る |
///
/// # 尺度は**同じ文の中で**突き合わせる（ADR-0015）
///
/// `StatValue` は「どの尺度で検証したか」を覚えているが、
/// **その尺度がそのキャラクターのものかは型では守れない**（`StatValue` の doc）。
/// 別の文で読んでから書くと、その間に `PATCH` が尺度を変えて
/// **検査した尺度と書き込む先の尺度が別物**になる（`character.rs` の全置換が
/// `BEGIN IMMEDIATE` を張っているのと同じ問題）。
/// 相関副問い合わせなら SQLite が1文として原子的に見るので、
/// **トランザクションを張らずに同じ保証が得られる。**
///
/// 親の行が無ければ副問い合わせは NULL を返し、比較も NULL（＝偽）になるので
/// **0 行更新**に落ちる。孤児の行に書き込む経路は存在しない。
///
/// # 下端（`DRIFT_FLOOR`）はここでは見ない
///
/// 「`1` より下へ落とさない」は `domain::apply_drift` が飽和として弾いており、
/// **この文へ届く `after` は必ず 1 以上**である。ここに写しを置くと、
/// 片方だけが緩んだときに黙ってずれる（CLAUDE.md「同じ発見を2箇所に書かない」）。
/// スキーマ側の `value >= 0` は別の目的（型アフィニティと負値）の CHECK で、
/// **こちらの下端とは条件が違う。**
///
/// # 5つ目の条件 —— **それを生んだ会話が今も在る**（作業単位 A-10 / ADR-0049 §決定3）
///
/// `?5` は往復の会話の id。**その会話が今も在り、同じキャラクターのものであるときだけ**動かす。
/// 落とすと、完全に消したキャラクターと**同じ id で作り直した**キャラクターの、
/// たまたま同じ軸名・同じ値の行が動く（`character.id` は使い回される /
/// `StoreError::TurnSessionGone` の doc）。**`?5` が NULL なのはテスト専用の
/// [`Store::apply_stat_drift`] だけ**で、本番の [`Store::apply_stat_drift_in_session`] は必ず渡す。
const UPDATE_STAT_ALLOC: &str = "UPDATE stat_alloc SET value = ?4 \
     WHERE character_id = ?1 AND axis = ?2 AND value = ?3 \
     AND ?4 <= (SELECT stat_scale FROM character \
                WHERE character.id = stat_alloc.character_id) \
     AND (?5 IS NULL OR EXISTS (SELECT 1 FROM session \
                                WHERE session.id = ?5 AND session.character_id = ?1))";

/// **読みから書きへ昇格するトランザクションの開始文**（`character.rs` の同名の定数と同じ理由。
/// **共有しない**）。累積の書き戻しは、会話を確かめる読みから始まるので要る。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// **書き込みが届いたかどうか**（ADR-0024 §7）。
///
/// # `bool` にしない理由
///
/// 呼び出し側は**この値を数える**（ADR-0024 の「危険」を事後にログから読むため）。
/// `bool` だと `if moved { .. }` と書いたときに
/// 「動いた／動かなかった」のどちらを数えているのかがソースから読めず、
/// **反転させる変異が読み手に見えなくなる。**
///
/// # **入口は [`DriftWrite::label`] ひとつ**（16b のレビュー）
///
/// `moved() -> bool` も持っていたが、**本番から一度も呼ばれていなかった**
/// （`pub` なので `dead_code` も鳴らない）。doc が書いていた用途「ログの数値欄」は
/// `label()` が担っている。**見張り番の無い公開 API を置かない**という
/// 16a の判断（`prompt::drift::DriftPrompt::messages()` を消したのと同じ）に揃えて外した。
/// **必要になったら、そのときに見張り番と一緒に足すこと。**
///
/// # `Option<()>` でもない
///
/// 「無かった」ではなく「**動かなかった**」という結果である。
/// 動かない理由（競合・尺度が下げられた・宛先が無い）は
/// **どれもこの1文からは区別できない**ので、理由を型に持たせない。
/// 区別が要るようになったら、そのときに文を分けること。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use = "動いたかどうかを数えないと、ADR-0024 の危険がログから読めなくなる"]
pub enum DriftWrite {
    /// 1行動いた。
    Moved,
    /// **0 行だった。** 競合したか、尺度が下げられたか、宛先が無い。
    ///
    /// **エラーではない**（モジュール doc）。
    NotMoved,
}

impl DriftWrite {
    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない
    /// （`domain::DriftOutcome::label` と同じ扱い）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Moved => "moved",
            Self::NotMoved => "not_moved",
        }
    }
}

impl Store {
    /// **性格ステータスを1軸だけ1段動かす**（ADR-0024 §7）。
    ///
    /// 動いたかどうかを返す。**0 行更新はエラーではない**（[`DriftWrite`] の doc）。
    ///
    /// # 受け取るのは [`DriftChange`] だけ
    ///
    /// 軸名も前後の値も、**`domain::apply_drift` の許可リストを通った値**である
    /// （フィールドが private なので、このシグネチャは
    /// 「照合を通っていない軸名」を受け取れない）。
    ///
    /// **`ValidTag` と同じ強さだと読まないこと。** `apply_drift` は
    /// `domain::drift` モジュールの中にあるので、**同じモジュールの中では
    /// 構造体リテラルで組める**（`DriftChange` の doc）。
    /// ここが主張できるのは「`store` の呼び出し側からは組めない」までである。
    ///
    /// # トランザクションを張らない
    ///
    /// 判定と書き込みが1文に閉じているため（`UPDATE_STAT_ALLOC` の doc）。
    /// 張らないことは**裏方が書きロックを長く持たない**ことでもある
    /// （CLAUDE.md「トランザクションを長く保持すると `busy_timeout` を食い潰す」）。
    ///
    /// # 2 行以上動くことはない
    ///
    /// `stat_alloc` の主キーが `(character_id, axis)` なので、
    /// 上の条件に一致する行は**高々1行**である。
    /// それでも `== 1` ではなく `> 0` で判定するのは、
    /// **主キーが変わった日に「動いたのに動かなかったと数える」側へ倒れないため。**
    ///
    /// # **許可証を要求する**（ADR-0026 §決定1）
    ///
    /// `permit` は `domain::StatDriftMode::applying` からしか手に入らず、
    /// **`On` 以外では `None` になる。**
    /// **`server::drift` の現状のコードには、記録専用モード（`shadow`）から
    /// この文へ至る枝が存在しない。**
    ///
    /// # **「到達する経路が型として無い」とは書かない**（16d のレビューで実測）
    ///
    /// **`StatDriftMode::On.applying()` はどこからでも呼べる。**
    /// 呼び出し側を `mode.applying()` から `StatDriftMode::On.applying()` へ変える変異は
    /// **コンパイルが通る。** 型が消しているのは**呼び出し側の分岐**ではなく、
    /// **分岐の中身を間違って書ける余地**——
    /// 「許可証を持っていないのにこの文を呼ぶ」形だけである。
    ///
    /// **`ValidTag` と同じ強さだと読まないこと。** `DriftApply` は
    /// `domain::drift` の中では構造体リテラルで組める（あちらの doc）。
    /// ここが主張できるのは「**`store` の呼び出し側からは組めない**」までである。
    ///
    /// **記録専用モードで値が動かないことの権威は、振る舞いを見る2本**——
    /// `server::drift::tests::the_shadow_mode_never_touches_the_stored_values` と
    /// `tests/server_stat_drift.rs::the_shadow_mode_records_a_vote_through_the_real_bind`。
    ///
    /// > **⚠ 統合の1本が権威になったのは 2026-08-25 から**（作業単位 19 のレビュー）。
    /// > **`shadow` の腕だけが既定の `K` で回っていて、既定の `K` では `on` でも
    /// > 1往復では動かない**ため、**「値が動かない」がモードではなくつまみの結果**だった。
    /// > **両腕を同じ安い `K` に揃えて直した**（実測: 直す前は緑・直した後は赤）。
    ///
    /// 許可証は**値として受け取って消費する**（`_permit`）。
    /// 参照にすると、1枚で何度でも呼べる。使い道は「持っていること」そのものなので、
    /// 本体では読まない。
    ///
    /// # **テスト専用**（作業単位 A-10）
    ///
    /// 会話で縛らない（`?5` が NULL）。本番の書き手は [`Self::apply_stat_drift_in_session`]。
    #[cfg(any(test, feature = "testing"))]
    pub async fn apply_stat_drift(
        &self,
        character_id: CharacterId,
        change: &DriftChange,
        permit: DriftApply,
    ) -> Result<DriftWrite, StoreError> {
        self.update_stat_alloc(character_id, None, change, permit)
            .await
    }

    /// **往復の会話で縛って**1軸だけ1段動かす（本番の書き手 / 作業単位 A-10 / ADR-0049 §決定3）。
    ///
    /// [`Self::apply_stat_drift`] の doc の約束はそのまま。加えて、`session_id` の会話が
    /// 今も在り `character_id` のものであるときだけ動かす（同じ文 / [`UPDATE_STAT_ALLOC`] の `?5`）。
    ///
    /// **0 行だったら、会話の持ち主を引き直す**（`session::turn_session_refusal`）。
    /// 会話が無ければ [`StoreError::TurnSessionGone`]（キャラクターが完全に消された）、
    /// 別のキャラクターの会話なら [`StoreError::SessionOfAnotherCharacter`]（取り違え）、
    /// そのキャラクターの会話なら今までどおり [`DriftWrite::NotMoved`]。
    /// **引き直しは理由を分けるだけ**で、書くかどうかはもう決まっている（書いていない）ので、
    /// 引き直しとの間の窓は害にならない。
    pub async fn apply_stat_drift_in_session(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        change: &DriftChange,
        permit: DriftApply,
    ) -> Result<DriftWrite, StoreError> {
        let write = self
            .update_stat_alloc(character_id, Some(session_id), change, permit)
            .await?;
        if write == DriftWrite::NotMoved
            && let Some(refusal) =
                super::session::turn_session_refusal(&self.pool, character_id, session_id).await?
        {
            return Err(refusal);
        }
        Ok(write)
    }

    /// [`UPDATE_STAT_ALLOC`] を1回流す（`session_id` が `None` なら会話で縛らない）。
    async fn update_stat_alloc(
        &self,
        character_id: CharacterId,
        session_id: Option<SessionId>,
        change: &DriftChange,
        _permit: DriftApply,
    ) -> Result<DriftWrite, StoreError> {
        let done = sqlx::query(UPDATE_STAT_ALLOC)
            .bind(character_id.get())
            // **軸名は保存済みの綴り**（許可リストを通っている / ADR-0024 §1）。
            .bind(change.axis().as_str())
            // **読んだときの値。** これが「読んでから書く」を1文に閉じている。
            .bind(change.before().get())
            .bind(change.after().get())
            // **`?5`**（往復の会話 / 作業単位 A-10）。
            .bind(session_id.map(SessionId::get))
            .execute(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        if done.rows_affected() > 0 {
            tracing::debug!(
                character_id = character_id.get(),
                "性格ステータスを1軸動かしました"
            );
            return Ok(DriftWrite::Moved);
        }

        // **値も軸名もログに出さない**（`character.rs` の拒否と同じ。手掛かりは id）。
        // 数えるのは呼び出し側（`server::drift`）——ここで数えると、
        // **走った回の総数が分からないので率が作れない。**
        tracing::debug!(
            character_id = character_id.get(),
            "性格ステータスは動きませんでした"
        );
        Ok(DriftWrite::NotMoved)
    }
}

/// **票を1件足す文**（ADR-0026 §決定3）。**追記のみ。**
///
/// `id` / `created_at` はスキーマの DEFAULT に任せる。
///
/// # 更新も削除も対になる文を持たない
///
/// 票は「なぜこの値になったのか」の記録でもあるので、後から書き換えると
/// **記録としての意味が消える**（ADR-0022 の「消さない」と同じ立場）。
/// **見張り番は `no_statement_updates_or_deletes_a_vote`**
/// （`src/**` と `migrations/**` を走査する）。
/// **走査対象のファイルにテーブル名を綴らないこと**——綴ると走査が自分自身を落とす
/// （`store::memory::table_word` と同じ作法）。
///
/// # **会話が今も在り、同じキャラクターのものであるときだけ書く**（作業単位 A-10 / ADR-0049 §決定3）
///
/// 票は会話の id を持つので、外部キーが「会話が在る」ことは見る。**持ち主の一致まで同じ文で見る**
/// （`VALUES` ではなく `SELECT … WHERE EXISTS`）——外部キーだけだと、別のキャラクターの会話の
/// id を渡した票が書ける。条件が偽なら 0 行で、呼び出し側には `StoreError::TurnSessionGone`
/// （会話が無い）か `StoreError::SessionOfAnotherCharacter`（別のキャラクターの会話）。
const INSERT_VOTE: &str = "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
     SELECT ?1, ?2, ?3, ?4 \
     WHERE EXISTS (SELECT 1 FROM session WHERE session.id = ?2 AND session.character_id = ?1)";

/// **そのキャラクターの直近の票**（ADR-0026 §決定2 の入力）。
///
/// # 新しい順に返す
///
/// 蓄積が要るのは「直近 N 件」なので、**上限を SQL 側で切る**
/// （全件読んで Rust で切ると、票は消えないので読む量が会話の長さに比例して増える）。
/// 並べ替えは呼び出し側の仕事にしない——**どちらの順で返すかを1箇所に決める。**
///
/// # 第2キーに `id` を置く（`SELECT_ACTIVE_MEMORIES` と同じ理由）
///
/// `created_at` はミリ秒精度で**同値になりうる**。同着のとき順序が挿入順まかせになると、
/// 「直近 N 件」が回ごとに違う集合を指す。**1ターンに立つ票は高々1件**だが、
/// 復元・移行・測定用の一括投入では同着が普通に起きる。
const SELECT_RECENT_VOTES: &str = "SELECT session_id, axis, delta FROM stat_drift_vote \
     WHERE character_id = ? ORDER BY created_at DESC, id DESC LIMIT ?";

/// **保存済みの票1件。** `store` の外へ出る形。
///
/// `created_at` を載せないのは、**蓄積が時刻を見ないから**
/// （ADR-0026 §決定2 は「溜めて消費する」であって窓ではない。
/// 窓はこの ADR が実測のうえで却下している）。
/// 時刻が要る用途（測定・監査）は表を直接読む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDriftVote {
    /// どの往復で立ったか。**帰属を落とさない。**
    session_id: SessionId,
    /// 軸と向き。**読み出しの検証を通っている。**
    vote: DriftVote,
}

impl StoredDriftVote {
    /// どの往復で立った票か。
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// 軸と向き。
    #[must_use]
    pub const fn vote(&self) -> &DriftVote {
        &self.vote
    }
}

/// `DriftDelta` の DB 表現。**綴りの正は [`DriftDelta::label`]。**
///
/// **写しを作らない**（作業単位 16d の指示）。`migrations/0010_stat_drift_vote.sql` の
/// `CHECK (delta IN ('up', 'down'))` と同じ綴りであることは、
/// `tests/store_drift_vote.rs::every_direction_is_accepted_by_the_schema` が
/// **実際に書き込んで**確かめる。
///
/// # 集計キーと同じ関数でよいのか
///
/// **よい。** ログの欄と DB の綴りが同じであることは、
/// 測定でログと票を突き合わせるときにそのまま効く（ADR-0026 §決定1）。
/// 綴りを変える変異は `domain::drift::all_lists_every_delta_exactly_once`
/// （リテラルを直書きして押さえている）と上の統合テストの両方が赤にする。
const fn delta_to_db(delta: DriftDelta) -> &'static str {
    delta.label()
}

/// DB 表現から `DriftDelta` へ。想定外の綴りは `None`（呼び出し側でエラーに落とす）。
///
/// **trim や小文字化はしない**（`store::memory::subject_from_db` と同じ）。
/// 受理を広げると、CHECK を通らない値をアプリ層だけが受け入れる非対称ができる。
fn delta_from_db(stored: &str) -> Option<DriftDelta> {
    DriftDelta::ALL
        .into_iter()
        .find(|delta| delta_to_db(*delta) == stored)
}

impl Store {
    /// **票を1件記録する**（ADR-0026 §決定3）。**値は動かさない。**
    ///
    /// 受け取るのは [`DriftVote`] だけ——構築経路が `domain::DriftOutcome::vote` に
    /// 限られているので、**許可リストと §4 の門を通っていない軸名はここへ届かない**
    /// （`DriftChange` と同じ形）。軸名は**プレースホルダ**で渡る（CLAUDE.md 絶対制約）。
    ///
    /// # トランザクションを張らない
    ///
    /// 1文で完結する（`apply_stat_drift` と同じ）。裏方が書きロックを長く持たない。
    ///
    /// # 失敗しても本文には影響しない
    ///
    /// 呼び出し側（`server::drift`）は結果を数えてログに残すだけで、応答はとうに返っている。
    pub async fn record_drift_vote(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        vote: &DriftVote,
    ) -> Result<(), StoreError> {
        let done = sqlx::query(INSERT_VOTE)
            .bind(character_id.get())
            .bind(session_id.get())
            // **軸名は保存済みの綴り**（許可リストを通っている / ADR-0024 §1）。
            .bind(vote.axis().as_str())
            .bind(delta_to_db(vote.delta()))
            .execute(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        // **会話が無い・別のキャラクターの会話**（作業単位 A-10 / `INSERT_VOTE` の doc）。
        // 理由は分ける（消えた → `TurnSessionGone` / 取り違え → `SessionOfAnotherCharacter`）。
        if done.rows_affected() == 0 {
            let refusal =
                super::session::turn_session_refusal(&self.pool, character_id, session_id).await?;
            return Err(refusal.unwrap_or(StoreError::TurnSessionGone));
        }

        // **軸名も向きもここでは出さない**（数えるのは呼び出し側 / `apply_stat_drift` と同じ）。
        tracing::debug!(
            character_id = character_id.get(),
            session_id = session_id.get(),
            "性格の変容の票を記録しました"
        );
        Ok(())
    }

    /// **そのキャラクターの直近の票を、新しい順に最大 `limit` 件**返す。
    ///
    /// 蓄積（ADR-0026 §決定2 / 作業単位 16e）の入力になる読み。
    ///
    /// # **1行でも読めなければ、この読みは丸ごと止まる**
    ///
    /// `Store::recall_memories` / `Store::stat_allocs` と同じ側に倒してある。
    /// 読めない行を**黙って飛ばすと、票の数が静かに減る**——
    /// 蓄積は数を数えて閾値と比べる仕掛けなので、
    /// **「1票足りない」は「変容が1回起きない／余計に起きる」に直結し、
    /// しかもどこにも現れない。**
    ///
    /// | 1行壊れたときに | |
    /// |---|---|
    /// | **止まるもの** | そのキャラクターの**蓄積の判定だけ**（16e） |
    /// | **止まらないもの** | 本文の応答・記憶の抽出・変容の判定・**票の記録**（追記は読まない） |
    ///
    /// **止まっても票は溜まり続ける**ので、壊れた行を直せば続きから読める。
    /// 直す導線は今のところ `sqlite3` だけ——**票に更新も削除も無い**（追記のみ）ので、
    /// アプリ経由で消す手段は用意していない。
    ///
    /// # 壊れうるのは2つ
    ///
    /// * **軸名**（`StatAxis::parse` を通らない値。空・制御文字・長すぎる）
    /// * **向き**（`up` / `down` 以外。CHECK があるので、通常の経路では入らない）
    pub async fn recent_drift_votes(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<StoredDriftVote>, StoreError> {
        let rows = sqlx::query(SELECT_RECENT_VOTES)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut votes = Vec::with_capacity(rows.len());
        for row in &rows {
            votes.push(vote_from_row(row)?);
        }
        tracing::debug!(
            character_id = character_id.get(),
            votes = votes.len(),
            "性格の変容の票を読み出しました"
        );
        Ok(votes)
    }
}

/// 1行を [`StoredDriftVote`] に組み立てる。**panic しない。**
fn vote_from_row(row: &SqliteRow) -> Result<StoredDriftVote, StoreError> {
    let session_id: i64 = row.try_get("session_id").map_err(StoreError::Query)?;
    let axis: String = row.try_get("axis").map_err(StoreError::Query)?;
    let delta: String = row.try_get("delta").map_err(StoreError::Query)?;

    // **読み出しでも検証する**（`store::memory::memory_from_row` と同じ作法）。
    let axis = StatAxis::parse(&axis).map_err(|_| StoreError::InvalidStoredValue)?;
    let delta = delta_from_db(&delta).ok_or(StoreError::InvalidStoredValue)?;

    Ok(StoredDriftVote {
        session_id: SessionId::from_row(session_id),
        vote: DriftVote::from_row(axis, delta),
    })
}

/// **そのキャラクターの累積を全部読む**
/// （[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §決定1〜3 の持ち越し）。
///
/// 行が無い軸は**累積 0**として扱う（呼び出し側が既定で埋める）ので、
/// **初回のターンでも何も特別なことをしなくてよい。**
///
/// # `updated_at` も読む（作業単位 D-5a）
///
/// [`UPSERT_CHARGE`] は**毎回この列を進めている**のに、
/// **どの `SELECT` も読んでいなかった**——あちらの doc が
/// 「**表から取れる時間の手掛かりはこの1列だけ**」と書いていた列が、
/// `sqlite3` からしか取れない状態だった。
///
/// **⚠ 読む以上、この列が壊れているとこの読みは丸ごと止まる**
/// （[`Store::drift_charges`] の doc に、増えた止まり方と却下した案）。
const SELECT_CHARGES: &str =
    "SELECT axis, charge, updated_at FROM stat_drift_charge WHERE character_id = ?";

/// **累積を1軸ぶん書き戻す文。**
///
/// # なぜ `INSERT ... ON CONFLICT DO UPDATE` なのか
///
/// 累積の行は**「無い」と「0」が同じ意味**なので、
/// 「先に読んで無ければ INSERT、あれば UPDATE」と書き分ける理由が1つも無い。
/// 書き分けると、**読んでから書くまでの窓**（ADR-0024 §7 と同じ形）が増える。
///
/// # `updated_at` も一緒に進める
///
/// 進めないと、**いつまで変容が走っていたのかが表から読めない。**
/// 累積は「何が起きたか」の履歴を持たない（履歴は票とログの側）ので、
/// **表から取れる時間の手掛かりはこの1列だけ**である。
const UPSERT_CHARGE: &str = "INSERT INTO stat_drift_charge (character_id, axis, charge) \
     VALUES (?, ?, ?) \
     ON CONFLICT (character_id, axis) DO UPDATE \
     SET charge = excluded.charge, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// **保存済みの累積1件**（作業単位 D-5a）。`store` の外へ出る形。
///
/// # なぜ `(StatAxis, f64)` の組をやめたのか
///
/// **列が3つになったから**ではない。**帰属を型で束ねるため**である
/// ——組のまま3要素にすると、`charges.iter().find(|(axis, _, _)| ...)` の
/// **添字を1つずらす変異が、どの型検査にも掛からない。**
/// [`StoredDriftVote`] / [`DriftVoteTally`] と同じ形に揃えてある。
///
/// # 書き込み側（[`Store::save_drift_charges`]）は組のまま
///
/// **同じ型にしない。** 時刻を決めるのは SQL（`strftime`）であって呼び出し側ではないので、
/// 書き込みの引数に `updated_at` が現れると
/// **「渡した時刻が書かれる」と読める形**ができる（実際には無視される）。
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDriftCharge {
    /// どの軸のものか。**読み出しの検証を通っている。**
    axis: StatAxis,
    /// 溜まっている累積。**有限であることを確かめてある。**
    charge: f64,
    /// **最後に書き戻された時刻。**
    ///
    /// 累積は履歴を持たない（履歴は票とログの側）ので、
    /// **表から取れる時間の手掛かりはこの1列だけ**である（[`UPSERT_CHARGE`] の doc）。
    updated_at: DateTime<Utc>,
}

impl StoredDriftCharge {
    /// どの軸のものか。
    #[must_use]
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 溜まっている累積。
    #[must_use]
    pub const fn charge(&self) -> f64 {
        self.charge
    }

    /// 最後に書き戻された時刻（UTC）。
    #[must_use]
    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    /// **テスト専用の抜け穴**（作業単位 D-5a）。累積を仕込むためだけに使う。
    ///
    /// [`DriftVoteTally::from_counts_for_tests`] と同じ位置づけで、
    /// `testing` フィーチャ（と crate 内テスト）でしか存在しない。
    /// 本番ビルドには含まれないので、**検証を通っていない累積が外へ出る経路は増えない。**
    ///
    /// 要る理由は `server::admin` の組み立てを**純粋関数のまま**見張るため
    /// ——DB から来る累積は**常に軸名の昇順**なので、
    /// 実物を通すと「並べていない実装」が偶然そろってしまう。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn from_parts_for_tests(
        axis: StatAxis,
        charge: f64,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            axis,
            charge,
            updated_at,
        }
    }
}

impl Store {
    /// **そのキャラクターの累積を読む**（ADR-0027 §決定7 の「溜めて消費する」の持ち越し）。
    ///
    /// 返るのは [`StoredDriftCharge`]。**行が無い軸は返らない**——
    /// 呼び出し側（`server::drift` / `server::admin`）が保存済みの軸に合わせて埋める。
    ///
    /// # **1行でも読めなければ、この読みは丸ごと止まる**
    ///
    /// `Store::recent_drift_votes` と同じ側に倒してある。
    /// 黙って飛ばすと**その軸の累積だけが 0 に戻る**——
    /// 「その軸だけ変容が遅い」としか観測できず、どこにも現れない。
    ///
    /// | 1行壊れたときに | |
    /// |---|---|
    /// | **止まるもの** | そのキャラクターの**値の適用だけ**と、**管理 API の変容の一覧** |
    /// | **止まらないもの** | 本文の応答・記憶の抽出・変容の判定・**票の記録** |
    ///
    /// # 壊れうるのは3つ
    ///
    /// * **軸名**（`StatAxis::parse` を通らない値）
    /// * **累積**（`REAL` 以外、または有限でない値。CHECK があるので通常の経路では入らない）
    /// * **時刻**（`updated_at` が時刻として読めない値。**CHECK が無い**ので
    ///   `sqlite3` から作れる / 作業単位 D-5a）
    ///
    /// # ⚠ **3つ目は D-5a で増えた。止まる理由が1つ増えている**
    ///
    /// **「一貫している」とは書かない。正確ではない**（D-5a のレビューで訂正 / 2026-08-28）。
    ///
    /// **この家には作法が2つある。**
    /// `UnreadableCharacter` / `UnreadableMemory` は**壊れた行を落とさずに
    /// 「壊れている」と名乗って返し**、修復モードまで作った——
    /// **しかもそれは管理画面へ供給する読み経路のために入った作法で、
    /// この読みも管理画面へ供給する。**
    ///
    /// **それを採らなかったのは、応答に3つ目の状態が要るから。**
    /// `admin::DriftAxisView::charge_updated_at` の `null` は
    /// **既に「累積の行が無い」で埋まっている**ので、
    /// **「読めない」を `null` に潰すと `D-3c` の「言えないものを言わない」に反する。**
    ///
    /// **代わりに払っているのは、`updated_at` を1バイトも読まない
    /// `server::drift::apply_scores` まで止まること。**
    /// **D-5a の実測**: この列を `SELECT` から落とす変異は 51 本を赤にしたが、
    /// **うち 45 本は `updated_at` を一度も参照しない振る舞い**だった
    /// ——**それが「壊れた1行が本番で巻き込む範囲」の実測値である。**
    ///
    /// **釣り合うと判断した理由は2つ。**
    ///
    /// * **壊れた行を作れるのは `sqlite3` を手で叩く経路だけ**（正規の書き込みは
    ///   `strftime` の綴りしか入れない / `migrations/0012` に形式の CHECK は無い）。
    /// * **壊れたときは沈黙ではなく 500 になる**——画面が落ちるので気づける。
    ///   黙って飛ばすと**その軸の累積だけが 0 に戻り**、どこにも現れない。
    ///
    /// **⚠ 第3の道があったことも書いておく** —— **継ぎ目をデコードに置く**
    /// （`updated_at` を「読めた／読めない」の型にする）。
    /// 文は1つのまま・`apply_scores` は無変更・`admin` だけが「読めない」を名乗る。
    /// **採らない理由は上と同じ「応答に3つ目の状態が要る」**である。
    ///
    /// 見張り番は `tests/store_drift_charge.rs::
    /// a_stored_time_that_is_not_a_time_stops_the_whole_read`。
    ///
    /// # ⚠ 将来 `updated_at` に CHECK を足すなら（**足すとは決めていない**）
    ///
    /// **CHECK は新しい行しか止めない。** 既に入っている壊れた行はそのまま残るので、
    /// **「既存の壊れた行をどうするか」まで決めないと、この止まり方は1ミリも変わらない。**
    /// **前例は `migrations/0012`** ——±Inf の行を新しい表へ**写さない**（捨てる）と決めた。
    pub async fn drift_charges(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<StoredDriftCharge>, StoreError> {
        let rows = sqlx::query(SELECT_CHARGES)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut charges = Vec::with_capacity(rows.len());
        for row in &rows {
            charges.push(charge_from_row(row)?);
        }
        tracing::debug!(
            character_id = character_id.get(),
            charges = charges.len(),
            "性格の変容の累積を読み出しました"
        );
        Ok(charges)
    }

    /// **累積を書き戻す**（ADR-0027 §決定7）。**値を動かす許可証が要る。**
    ///
    /// # なぜ許可証が要るのか（ADR-0026 §決定1）
    ///
    /// 累積は**値を動かす機構の状態**である。記録専用モード（`shadow`）で
    /// 溜めておくと、`on` に切り替えた瞬間に**溜まっていた分がまとめて放出される。**
    /// 記録専用モードの存在理由は「値が動かないので決定性が保たれる」ことなので、
    /// **切り替えた直後だけ荒れる**のは、その前提を裏から壊す。
    ///
    /// **`ValidTag` と同じ強さだと読まないこと。** `DriftApply` は
    /// `domain::drift` の中では構造体リテラルで組める（あちらの doc）。
    /// ここが主張できるのは「**`store` の呼び出し側からは組めない**」までである。
    /// 振る舞いの権威は `server::drift::tests::the_shadow_mode_never_touches_the_stored_values`。
    ///
    /// # トランザクションを1つ張る
    ///
    /// **1ターンで複数の軸の累積が動く**（全軸採点では毎ターン全軸が動く）。
    /// 途中で落ちると、**動いた軸と動かなかった軸で帳尻が合わなくなる**
    /// ——次のターンは食い違ったまま進み、どこにも現れない。
    ///
    /// **中で外部通信を `await` しない**（CLAUDE.md 絶対制約）。
    /// 入っているのは同じ表への `INSERT` だけで、**呼ぶ前に上流との往復は済んでいる。**
    ///
    /// 軸名は**保存済みの綴り**（`domain::apply_drift` / `Store::stat_allocs` を通った値）で、
    /// **プレースホルダ**で渡る。
    ///
    /// # **テスト専用**（作業単位 A-10）
    ///
    /// 会話で縛らない。本番の書き手は [`Self::save_drift_charges_in_session`]。
    #[cfg(any(test, feature = "testing"))]
    pub async fn save_drift_charges(
        &self,
        character_id: CharacterId,
        charges: &[(StatAxis, f64)],
        permit: DriftApply,
    ) -> Result<(), StoreError> {
        self.write_drift_charges(character_id, None, charges, permit)
            .await
    }

    /// **往復の会話で縛って**累積を書き戻す（本番の書き手 / 作業単位 A-10 / ADR-0049 §決定3）。
    ///
    /// [`Self::save_drift_charges`] の doc の約束はそのまま。加えて、**書きロックを取ってから**
    /// （`BEGIN IMMEDIATE`）`session_id` の会話が今も在り `character_id` のものかを確かめ、
    /// 無ければ1行も書かずに [`StoreError::TurnSessionGone`]、別のキャラクターの会話なら
    /// [`StoreError::SessionOfAnotherCharacter`]。
    ///
    /// **確かめてから書くが、隙は無い。** `BEGIN IMMEDIATE` は始めた時点で書きロックを取るので、
    /// 確かめてから commit するまで、他の接続はキャラクターを消せない（消す `DELETE` も書き込み）。
    /// 累積は複数の軸を1つのトランザクションで書くので、1文に寄せる代わりにこの形にした。
    pub async fn save_drift_charges_in_session(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        charges: &[(StatAxis, f64)],
        permit: DriftApply,
    ) -> Result<(), StoreError> {
        self.write_drift_charges(character_id, Some(session_id), charges, permit)
            .await
    }

    /// 累積を書き戻す本体（`session_id` が `None` なら会話で縛らない）。
    async fn write_drift_charges(
        &self,
        character_id: CharacterId,
        session_id: Option<SessionId>,
        charges: &[(StatAxis, f64)],
        _permit: DriftApply,
    ) -> Result<(), StoreError> {
        if charges.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;
        // **書きロックの中で会話を確かめる**（`session::turn_session_refusal`）。
        if let Some(session_id) = session_id
            && let Some(refusal) =
                super::session::turn_session_refusal(&mut *tx, character_id, session_id).await?
        {
            // commit しない（1行も書いていない）。
            return Err(refusal);
        }
        for (axis, charge) in charges {
            // **有限でない値を書かない。** 書けてしまうと、次の読み出しが
            // `InvalidStoredValue` になって**そのキャラクターの変容が永久に止まる**
            // （累積には削除の導線が無い）。
            if !charge.is_finite() {
                return Err(StoreError::InvalidStoredValue);
            }
            sqlx::query(UPSERT_CHARGE)
                .bind(character_id.get())
                .bind(axis.as_str())
                .bind(*charge)
                .execute(&mut *tx)
                .await
                .map_err(StoreError::Query)?;
        }
        tx.commit().await.map_err(StoreError::Query)?;

        // **軸名も値もログに出さない**（`apply_stat_drift` と同じ。手掛かりは id）。
        tracing::debug!(
            character_id = character_id.get(),
            charges = charges.len(),
            "性格の変容の累積を書き戻しました"
        );
        Ok(())
    }
}

/// 1行を [`StoredDriftCharge`] に組み立てる。**panic しない。**
fn charge_from_row(row: &SqliteRow) -> Result<StoredDriftCharge, StoreError> {
    let axis: String = row.try_get("axis").map_err(StoreError::Query)?;
    let charge: f64 = row.try_get("charge").map_err(StoreError::Query)?;
    // **時刻として読めない値はここで落ちる**（`record_from_row` と同じ作法）。
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(StoreError::Query)?;

    // **読み出しでも検証する**（`vote_from_row` と同じ作法）。
    let axis = StatAxis::parse(&axis).map_err(|_| StoreError::InvalidStoredValue)?;
    if !charge.is_finite() {
        return Err(StoreError::InvalidStoredValue);
    }
    Ok(StoredDriftCharge {
        axis,
        charge,
        updated_at,
    })
}

/// **人が読むための「直近の票」を引く文**（作業単位 D-4a）。
///
/// [`SELECT_RECENT_VOTES`] と**絞り込みも並びも同じで、返す列と用途が違う。**
/// あちらは蓄積の入力なので時刻を見ない（[`StoredDriftVote`] の doc）が、
/// こちらは**人が読む一覧**なので `created_at` を落とせない
/// ——「いつ動いたのか」が読めないと、`GET .../stats` の値と突き合わせられない。
///
/// # 並びを2箇所に持っていること
///
/// 片方だけ並びが変わると、**同じ「直近 N 件」という言葉が2つの意味を持つ。**
/// 見張り番は `the_two_recent_reads_ask_for_the_same_order`
/// （どちらの文面も直書きの断片で押さえる）。
///
/// # 第2キー（`id`）を落とさないこと
///
/// `created_at` はミリ秒精度で**同値になりうる**。
/// **全軸採点では1ターンで軸の数だけ票が立つ**ので、同着は本番で普通に起きる。
const SELECT_VOTE_HISTORY: &str = "SELECT session_id, axis, delta, created_at FROM stat_drift_vote \
     WHERE character_id = ? ORDER BY created_at DESC, id DESC LIMIT ?";

/// **そのキャラクターの票の総数**（作業単位 D-4a）。
///
/// # なぜ「返した件数」で代用しないのか
///
/// 一覧は上限で切られている（[`SELECT_VOTE_HISTORY`] の `LIMIT`）ので、
/// 件数を総数と呼ぶと**画面が「これで全部」と嘘をつく。**
///
/// **上限で切られない。** 軸として保存されていない綴りの票（`PUT .../stats` で
/// 消された軸）も数に入る——一覧には現れるので、落とすと
/// 「一覧に出ている票が総数に入っていない」という食い違いになる。
const COUNT_VOTES: &str = "SELECT count(*) AS votes FROM stat_drift_vote WHERE character_id = ?";

/// **軸ごと・向きごとの件数**（作業単位 D-4a）。
///
/// # 向きの綴りを SQL に書かない
///
/// `CASE WHEN delta = 'up' ...` と書けば1行で済むが、それは
/// [`delta_to_db`] の**写し**になる（しかもリテラルの埋め込みになる）。
/// 組ごとに数えて Rust 側で畳めば、綴りの判定は [`delta_from_db`] 1箇所のままで、
/// **読み出しの検証（想定外の綴りを弾く）もそのまま効く。**
///
/// **上限を持たない。** 軸の数だけしか行が返らない（1軸につき高々2行）。
const TALLY_VOTES: &str = "SELECT axis, delta, count(*) AS votes FROM stat_drift_vote \
     WHERE character_id = ? GROUP BY axis, delta";

/// **人が読むための票1件**（作業単位 D-4a）。
///
/// # [`StoredDriftVote`] を包んでいる理由
///
/// 帰属（往復）と中身（軸・向き）の**組み立てと検証を写さない**ため。
/// 違いは `created_at` を持つことだけで、そこは
/// **蓄積の側が意図して持っていない**（[`StoredDriftVote`] の doc）。
/// 向こうに列を足す形にすると、**壊れた `created_at` の行1つで蓄積が止まる**
/// ——読む必要が無いものを読むと、止まる理由が増える。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftVoteRecord {
    /// 往復と、軸・向き。**読み出しの検証を通っている。**
    stored: StoredDriftVote,
    /// いつ立った票か。**人が読む一覧の存在理由。**
    created_at: DateTime<Utc>,
}

impl DriftVoteRecord {
    /// どの往復で立った票か。
    #[must_use]
    pub const fn session_id(&self) -> SessionId {
        self.stored.session_id()
    }

    /// 軸と向き。
    #[must_use]
    pub const fn vote(&self) -> &DriftVote {
        self.stored.vote()
    }

    /// いつ立ったか（UTC）。
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// **1軸ぶんの上げ・下げの件数**（作業単位 D-4a）。
///
/// # 2つを別のフィールドにする理由
///
/// 差（`ups - downs`）だけを持つと、**「1回も動いていない」と
/// 「10 回ずつ押し合った」が同じ値になる。**
/// ADR-0024 の「危険 2. 一方向へ流れる」はこの2つの**比**で見るものなので、
/// 畳んだ時点でその問いに答えられなくなる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftVoteTally {
    /// どの軸のものか。**読み出しの検証を通っている。**
    axis: StatAxis,
    /// 上げの票の数。
    ups: u32,
    /// 下げの票の数。
    downs: u32,
}

impl DriftVoteTally {
    /// どの軸のものか。
    #[must_use]
    pub const fn axis(&self) -> &StatAxis {
        &self.axis
    }

    /// 上げの票の数。
    #[must_use]
    pub const fn ups(&self) -> u32 {
        self.ups
    }

    /// 下げの票の数。
    #[must_use]
    pub const fn downs(&self) -> u32 {
        self.downs
    }

    /// **テスト専用の抜け穴**（作業単位 B-16）。集計を仕込むためだけに使う。
    ///
    /// `CharacterId::from_raw_for_tests` と同じ位置づけで、
    /// `testing` フィーチャ（と crate 内テスト）でしか存在しない。
    /// 本番ビルドには含まれないので、**検証を通っていない集計が外へ出る経路は増えない。**
    ///
    /// 要る理由は `server::admin` の並べ替えを**純粋関数のまま**見張るため
    /// ——DB から来る集計は**常に軸名の昇順**なので、
    /// 実物を通すと「並べていない実装」が偶然そろってしまう。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn from_counts_for_tests(axis: StatAxis, ups: u32, downs: u32) -> Self {
        Self { axis, ups, downs }
    }

    /// 1組ぶんを足す。**向きごとに別の欄へ入れる。**
    ///
    /// 飽和加算にしてあるのは、`u32` を超える票が溜まった状態で
    /// **panic させないため**（本番コードパスに panic を置かない / CLAUDE.md）。
    /// そこまで溜まるとどのみち数として意味を失うが、
    /// **止まるより、上限に張り付いたまま読める方**を選ぶ。
    fn add(&mut self, delta: DriftDelta, votes: u32) {
        match delta {
            DriftDelta::Up => self.ups = self.ups.saturating_add(votes),
            DriftDelta::Down => self.downs = self.downs.saturating_add(votes),
        }
    }
}

impl Store {
    /// **そのキャラクターの直近の票を、新しい順に最大 `limit` 件**返す（作業単位 D-4a）。
    ///
    /// [`Store::recent_drift_votes`] との違いは `created_at` を返すことだけで、
    /// **使う側が違う**——あちらは蓄積（機械が読む）、こちらは管理 API（人が読む）。
    ///
    /// # **1行でも読めなければ、この読みは丸ごと止まる**
    ///
    /// `Store::recent_drift_votes` と同じ側に倒してある。
    /// **黙って飛ばすと、一覧から消えた票が総数にだけ残る**
    /// ——読み手には「数が合わない」としか見えず、どの行が壊れているかは分からない。
    ///
    /// **止まる範囲は、壊れている列で違う。**
    ///
    /// | 壊れている列 | 止まるもの |
    /// |---|---|
    /// | `created_at` | **この一覧だけ**（蓄積は時刻を読まない / [`DriftVoteRecord`] の doc） |
    /// | 軸名・向き | この一覧と**蓄積の読み**（同じ [`vote_from_row`] を通すため） |
    ///
    /// **どちらの場合も止まらないもの** —— 本文の応答・記憶の抽出・変容の判定・票の記録。
    ///
    /// # **見張り番**（レビューの実測で足した / 2026-08-27）
    ///
    /// **この決定は、doc に書いただけでは誰も見ていなかった**——
    /// `records.push(record_from_row(row)?)` を読み飛ばしに変える変異が **1602 全緑**だった。
    /// 権威は `tests/server_admin_drift.rs` の2本で、
    /// **止まる読みと止まらない読みを同じテストで**見ている:
    ///
    /// * `a_broken_time_stops_this_listing_but_not_the_total_or_the_accumulation`
    /// * `a_broken_axis_stops_this_listing_and_the_accumulation_but_never_the_total`
    pub async fn drift_vote_history(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<DriftVoteRecord>, StoreError> {
        let rows = sqlx::query(SELECT_VOTE_HISTORY)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            records.push(record_from_row(row)?);
        }
        tracing::debug!(
            character_id = character_id.get(),
            votes = records.len(),
            "性格の変容の履歴を読み出しました"
        );
        Ok(records)
    }

    /// **そのキャラクターの票の総数**（作業単位 D-4a）。**上限で切らない。**
    ///
    /// 数えるだけなので、軸名も向きも読まない
    /// ——**壊れた行があっても総数は返る**（一覧の側が止まる）。
    pub async fn count_drift_votes(&self, character_id: CharacterId) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_VOTES)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        // **`try_get::<u32>` にしない**（`stat_allocs` と同じ理由）。
        // 数え上げが `u32` に収まらない状態は、この表では起こりようがないが、
        // **収まらなかったときに panic しない形**にしておく。
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }

    /// **軸ごとの上げ・下げの件数**（作業単位 D-4a）。**上限で切らない。**
    ///
    /// 返るのは**票のある軸だけ。** 保存済みの軸に合わせて 0 で埋めるのは呼び出し側
    /// （`Store::drift_charges` と同じ立場）。
    /// **もう保存されていない軸**（`PUT .../stats` で消された綴り）も返る
    /// ——票は追記のみで消えないため。
    ///
    /// > **⚠ その「消された綴り」は、いまは応答の行になる**（作業単位 B-16）。
    /// > `server::admin::unallocated_axes` が、累積の側と合わせて拾う。
    /// > **ここから落とすと、その軸は `total_votes` にだけ残って行が消える。**
    ///
    /// # **1行でも読めなければ、この読みは丸ごと止まる**
    ///
    /// 飛ばすと**その軸の件数だけが静かに減る。**
    /// 「一方向へ流れていないか」を見るための数なので、
    /// 減った数は**判断を誤らせる方向**にしか働かない。
    ///
    /// **見張り番は `tests/server_admin_drift.rs::
    /// a_broken_row_outside_the_recent_window_still_stops_the_listing`。**
    /// **履歴の見張り番では、ここを緩める変異が捕まらない**——
    /// 壊れた行が直近の窓に入っていると**履歴の側が先に落ちて 500 のまま**になるので、
    /// **窓の外に置いた壊れた行**が要る（この読みだけが上限を持たない）。
    pub async fn drift_vote_tallies(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<DriftVoteTally>, StoreError> {
        let rows = sqlx::query(TALLY_VOTES)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut counted = Vec::with_capacity(rows.len());
        for row in &rows {
            counted.push(tally_from_row(row)?);
        }
        let tallies = fold_tallies(&counted);
        tracing::debug!(
            character_id = character_id.get(),
            axes = tallies.len(),
            "性格の変容の集計を読み出しました"
        );
        Ok(tallies)
    }
}

/// 1行を [`DriftVoteRecord`] に組み立てる。**panic しない。**
///
/// 往復・軸・向きの検証は [`vote_from_row`] に委譲する（**写さない**）。
fn record_from_row(row: &SqliteRow) -> Result<DriftVoteRecord, StoreError> {
    let stored = vote_from_row(row)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;
    Ok(DriftVoteRecord { stored, created_at })
}

/// 1行を `(軸名, 向き, 件数)` に組み立てる。**panic しない。**
fn tally_from_row(row: &SqliteRow) -> Result<(StatAxis, DriftDelta, u32), StoreError> {
    let axis: String = row.try_get("axis").map_err(StoreError::Query)?;
    let delta: String = row.try_get("delta").map_err(StoreError::Query)?;
    let votes: i64 = row.try_get("votes").map_err(StoreError::Query)?;

    // **読み出しでも検証する**（`vote_from_row` と同じ作法）。
    let axis = StatAxis::parse(&axis).map_err(|_| StoreError::InvalidStoredValue)?;
    let delta = delta_from_db(&delta).ok_or(StoreError::InvalidStoredValue)?;
    let votes = u32::try_from(votes).map_err(|_| StoreError::InvalidStoredValue)?;

    Ok((axis, delta, votes))
}

/// `(軸名, 向き, 件数)` の並びを、軸ごとの上げ・下げに畳む。**純粋関数。**
///
/// 並びは**最初に現れた軸の順**を保つ。呼び出し側（`server::admin`）は
/// 保存済みの軸の順に並べ替えるので順序に意味は無いが、
/// **入力の順で結果が変わらない**方がテストが読める。
fn fold_tallies(counted: &[(StatAxis, DriftDelta, u32)]) -> Vec<DriftVoteTally> {
    let mut tallies: Vec<DriftVoteTally> = Vec::new();
    for (axis, delta, votes) in counted {
        if let Some(found) = tallies.iter_mut().find(|tally| tally.axis == *axis) {
            found.add(*delta, *votes);
            continue;
        }
        let mut fresh = DriftVoteTally {
            axis: axis.clone(),
            ups: 0,
            downs: 0,
        };
        fresh.add(*delta, *votes);
        tallies.push(fresh);
    }
    tallies
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **2つの結末が、それぞれ決まった綴りを名乗る。**
    ///
    /// # `assert_ne!` だけでは足りない（変異検査で見つかった穴 / 2026-08-23）
    ///
    /// **2つを入れ替える変異が 1287 passed / 0 failed で緑を通った。**
    /// 呼び出し側の期待値は**この関数から作られている**
    /// （`server::drift` の見張り番は `DriftWrite::Moved.label()` と書く）ので、
    /// **綴りが入れ替われば期待値も一緒に入れ替わる。**
    /// CLAUDE.md「写しを1つにしても、その1つは自動的には守られない。
    /// 必要だったのは**関数の外にある値で押さえる見張り番**1本」。
    ///
    /// **`memory.rs::subject_uses_the_spelling_the_schema_expects` と同じ形**
    /// ——あちらも「往復するだけでは、2値を取り違えて対応させても成立する」と書いている。
    ///
    /// # 入れ替わると何が起きるか
    ///
    /// ログの書き込みの欄（`server::drift::record_run` が出す接頭辞つきの欄）が反転する。
    /// **値が動いた回が「動かなかった」として数えられ**、
    /// ADR-0024 §7 の「取りこぼしたら 0 行更新として観測できる」が**逆向きに嘘になる**
    /// ——競合が一度も起きていないのに、毎ターン競合しているように見える。
    #[test]
    fn each_result_uses_its_own_spelling() {
        // **綴りを直書きする。** ここが「関数の外にある値」。
        assert_eq!(DriftWrite::Moved.label(), "moved");
        assert_eq!(DriftWrite::NotMoved.label(), "not_moved");
        assert_ne!(DriftWrite::Moved.label(), DriftWrite::NotMoved.label());
    }

    /// **文が4つの条件を全部持っている。**
    ///
    /// 実挙動は `tests/store_stat_drift.rs` が固定しているが、
    /// **条件そのものが文から消えていないこと**もここで見ておく
    /// （`memory.rs::recall_query_filters_superseded_rows` と同じ形）。
    ///
    /// **どれか1つを落とす変異は、あちらの1本ずつが名指しで赤になる。**
    /// ここが守るのは「4つ揃っていること」の速報で、権威は統合テストの側にある。
    ///
    /// # ⚠ **「権威が向こうにある」を無条件に信じないこと**（16b のレビューで実測）
    ///
    /// 尺度の条件を `?4 <= ...` から **`?4 < ...`** へ狭める変異を当てたとき、
    /// **赤くなったのはこの速報1本だけ**で、振る舞いを見るテストは全部緑だった。
    /// **その条件については権威が存在しなかった。**
    /// いま権威は `tests/store_stat_drift.rs::
    /// a_drift_that_lands_exactly_on_the_scale_is_written`（上端ちょうどを書く）。
    ///
    /// **条件を1つ足したら、その条件が効く状態を作るテストも一緒に足すこと。**
    /// 文面の照合は「消えていないこと」しか言わない——**比較演算子が緩んでも、
    /// 部分文字列としては同じまま**でありうる。
    #[test]
    fn the_update_keeps_every_condition() {
        for condition in [
            "character_id = ?1",
            "axis = ?2",
            "value = ?3",
            "?4 <= (SELECT stat_scale",
        ] {
            assert!(
                UPDATE_STAT_ALLOC.contains(condition),
                "{condition} が条件から消えています: {UPDATE_STAT_ALLOC}"
            );
        }
    }

    /// **文字列連結でクエリを組み立てていない**（CLAUDE.md 絶対制約）。
    ///
    /// 値はすべて番号付きプレースホルダで渡る。
    /// 引用符に囲まれたリテラルが混ざっていたら、そこは組み立てである。
    #[test]
    fn the_update_binds_every_value_through_a_placeholder() {
        assert!(
            !UPDATE_STAT_ALLOC.contains('\''),
            "リテラルを埋め込まないこと: {UPDATE_STAT_ALLOC}"
        );
        for placeholder in ["?1", "?2", "?3", "?4"] {
            assert!(
                UPDATE_STAT_ALLOC.contains(placeholder),
                "{placeholder} が使われていません: {UPDATE_STAT_ALLOC}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // 票の綴りと文（ADR-0026 §決定3）
    // ---------------------------------------------------------------------

    /// **向きの DB 表現が、集計キーの綴りそのものである。**
    ///
    /// **写しを作らない**という 16d の判断を、ここで固定する。
    /// 綴りは `domain::drift::all_lists_every_delta_exactly_once` が
    /// リテラルで押さえており、スキーマの CHECK と合っていることは
    /// `tests/store_drift_vote.rs::every_direction_is_accepted_by_the_schema` が
    /// **実際に書き込んで**確かめる。**ここが見るのは委譲そのもの。**
    #[test]
    fn the_stored_direction_is_the_aggregation_key() {
        for delta in DriftDelta::ALL {
            assert_eq!(
                delta_to_db(delta),
                delta.label(),
                "DB の綴りと集計キーがずれた（写しができている）"
            );
        }
        assert_ne!(
            delta_to_db(DriftDelta::Up),
            delta_to_db(DriftDelta::Down),
            "2つの向きが同じ綴りになっている"
        );
    }

    /// **読み戻しは、書いた綴りだけを受ける。**
    ///
    /// # 受理を広げない（`store::memory::subject_from_db` と同じ）
    ///
    /// trim も小文字化もしないので、**CHECK を通らない値をアプリ層だけが
    /// 受け入れる非対称**ができない。
    #[test]
    fn reading_a_direction_back_accepts_only_the_spelling_that_was_written() {
        for delta in DriftDelta::ALL {
            assert_eq!(
                delta_from_db(delta_to_db(delta)),
                Some(delta),
                "書いた綴りが読み戻せること"
            );
        }
        for stored in [
            "", " up", "up ", "UP", "Up", "+1", "-1", "1", "increase", "sideways",
        ] {
            assert_eq!(delta_from_db(stored), None, "stored = {stored:?}");
        }
    }

    /// **票を足す文が、追記の形をしている。**
    ///
    /// 列を書き並べるのは、`id` と `created_at` をスキーマの DEFAULT に任せるため
    /// （`INSERT_MEMORY` と同じ形）。
    /// **帰属の2列（持ち主と往復）を落とさない**ことをここで固定する
    /// ——実挙動は `tests/store_drift_vote.rs` が持つ。
    #[test]
    fn the_insert_keeps_every_column_it_needs() {
        for column in ["character_id", "session_id", "axis", "delta"] {
            assert!(
                INSERT_VOTE.contains(column),
                "{column} が票の追記から消えています: {INSERT_VOTE}"
            );
        }
        assert!(
            INSERT_VOTE.starts_with("INSERT INTO "),
            "追記の形であること: {INSERT_VOTE}"
        );
    }

    /// **票の2つの文が、文字列連結ではなくプレースホルダで値を渡す**（絶対制約）。
    #[test]
    fn the_vote_statements_bind_every_value_through_a_placeholder() {
        for statement in [INSERT_VOTE, SELECT_RECENT_VOTES] {
            assert!(
                !statement.contains(quote_char()),
                "リテラルを埋め込まないこと: {statement}"
            );
            assert!(
                statement.contains(placeholder_char()),
                "プレースホルダが1つも無い: {statement}"
            );
        }
    }

    /// SQL のリテラルを囲む引用符。
    fn quote_char() -> char {
        '\''
    }

    /// SQL のプレースホルダ。
    fn placeholder_char() -> char {
        '?'
    }

    /// **直近の票を引く文が、絞り込みと並びと上限を全部持っている。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `character_id = ?` | **別のキャラクターの票が蓄積に混ざる** |
    /// | `ORDER BY created_at DESC` | 「直近」でなくなる（古い票から N 件） |
    /// | `, id DESC` | 同着の順が挿入順まかせになる |
    /// | `LIMIT ?` | 会話が長くなるほど読む量が増え続ける |
    ///
    /// 実挙動は `tests/store_drift_vote.rs` が持つ。ここは速報。
    #[test]
    fn the_recent_query_keeps_its_filter_order_and_limit() {
        for fragment in [
            "WHERE character_id = ?",
            "ORDER BY created_at DESC, id DESC",
            "LIMIT ?",
        ] {
            assert!(
                SELECT_RECENT_VOTES.contains(fragment),
                "{fragment} が消えています: {SELECT_RECENT_VOTES}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // 「票を書き換える文」の走査（ADR-0026 §決定3 の「追記のみ」）
    //
    // 形は `store::memory::no_statement_updates_or_deletes_a_memory` に倣う。
    // **判定は禁止語の列挙ではなく許可リスト**（新しい形が要るなら
    // `vote_form_is_allowed` を書き換えることになり、そこがレビューの掛かる場所になる）。
    // ---------------------------------------------------------------------

    /// 走査対象。**`migrations` を外さないこと。**
    ///
    /// 「もう適用済みだから消してよい票」を消す整理 SQL は、コードではなく
    /// **マイグレーションに書かれるのが最もありそう**である。
    ///
    /// **Rust 側は `src/store` ではなく `src` 全体**（あちらと同じ理由）。
    const VOTE_SCAN_ROOTS: [(&str, &str); 2] = [("src", "rs"), ("migrations", "sql")];

    /// テーブル名の近傍にあってはならない語。
    ///
    /// **`UPDATE` が入っているのが記憶の側との違い。**
    /// あちらは `superseded_by` の上書きという**正当な更新**を持つが、
    /// 票には**更新の用途が1つも無い**（ADR-0026 §決定3）。
    ///
    /// # ⚠ **`UPDATE` を外しても、いまは全緑になる**（条件付きの等価変異 / 実測）
    ///
    /// この一覧から `"UPDATE"` だけを外す変異は **1338 全緑**で通る。
    /// **無条件の等価ではない。** 寄りかかっている不変条件は**2本**ある:
    ///
    /// 1. **許可リストが `UPDATE` を通す形を1つも持たない**
    ///    （`vote_form_is_allowed` の既定は拒否）。
    ///    `UPDATE <表> SET ...` / `UPDATE main.<表>` / `UPDATE <表> AS v` は、
    ///    直前の語がどれも許可された語ではないので**許可リスト側が落とす。**
    /// 2. **`"CONFLICT"` が一覧に残っている。**
    ///    `INSERT INTO <表> ... ON CONFLICT (id) DO UPDATE SET ...` は
    ///    **表の位置が `INTO` の直後なので許可リストを通る。**
    ///    落としているのは `CONFLICT` の方である。
    ///    **`UPDATE` と `CONFLICT` を両方外すと
    ///    `the_vote_scan_catches_every_known_evasion` が赤になる**（実測）。
    ///
    /// **受理集合が同一なわけでもない。**
    /// `UPDATE character SET x = (SELECT COUNT(*) FROM <表>)` のように
    /// **別の表の更新の中で票を読む文**は、変異後は許可側へ落ちる。
    /// これは票を**読む**だけで壊さないので実害はゼロと判断し、テストは足していない。
    const VOTE_DESTRUCTIVE: [&str; 6] = [
        "UPDATE", "REPLACE", "DELETE", "DROP", "TRUNCATE", "CONFLICT",
    ];

    /// テーブル名（大文字）を**ソース上に綴らずに**組み立てる。
    ///
    /// この走査は自分自身のファイルも対象にするため、テーブル名をリテラルで書くと
    /// 必ず自分に引っかかる（`store::memory::table_word` と同じ作法）。
    fn vote_table_word() -> String {
        ["stat_drift", "_vote"].concat().to_uppercase()
    }

    /// **走査対象のどこにも「票を書き換えうる文」が無い**（ADR-0026 §決定3）。
    ///
    /// # 走査は生ソースを読む（記憶の側と違う）
    ///
    /// あちらは Rust の字句を辿って**文字列リテラルだけ**を取り出すが、
    /// ここは**ソース全体**をトークンに均す。理由は2つ:
    ///
    /// * **こちらの方が保守的**（見落とす側ではなく、誤検出する側に倒れる）。
    /// * 字句解析を写すと、**片方だけが直る**写しがもう1つ増える
    ///   （CLAUDE.md「同じ発見を2箇所に書かない」）。
    ///
    /// **代償は、テーブル名を Rust のコメントや識別子に綴れないこと。**
    /// 綴ると許可された形に当たらず、この見張り番が赤になる。
    /// **それは正しい振る舞いである**——静かに通り抜けるより、うるさく落ちる方を選ぶ。
    #[test]
    fn no_statement_updates_or_deletes_a_vote() {
        let table = vote_table_word();
        let targets = vote_scan_targets();

        assert!(
            targets.len() >= 5,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            targets.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut mentions = 0;
        let mut found = Vec::new();
        for (path, source) in &targets {
            mentions += vote_mentions_of(source, &table);
            for violation in vote_violations(source, &table) {
                found.push(format!("{path}: {violation}"));
            }
        }

        assert!(
            found.is_empty(),
            "票を書き換えうる文があります:\n{}",
            found.join("\n")
        );
        // **言及を1件も拾えていないなら、走査が壊れている。**
        // 内訳は追記と読み出しの2つと、マイグレーションの
        // `CREATE TABLE` / `CREATE INDEX ... ON` の2つ。
        assert!(
            mentions >= 4,
            "テーブル名の言及を拾えていない（走査が壊れている）: {mentions} 件"
        );
    }

    /// **走査本体が現に違反を見つけられる**ことの対照。
    ///
    /// **対照は植えたソースを読む**（実ファイルを読むと自己成就する / 15b の実測）。
    /// 走査本体（`vote_violations`）を**引数でソースを受け取る形**にしてあるので、
    /// 実ファイルと植えたソースを同じ判定に通せる。
    #[test]
    fn the_vote_scan_catches_every_known_evasion() {
        let table = vote_table_word();
        let name = table.to_lowercase();
        let evasions = [
            format!("UPDATE {name} SET delta = ? WHERE id = ?"),
            format!("UPDATE {name} SET delta = ?"),
            format!("UPDATE main.{name} SET axis = ?"),
            format!("UPDATE {name} AS v SET axis = ?"),
            format!("UPDATE OR REPLACE {name} SET delta = ?"),
            format!("DELETE FROM {name} WHERE character_id = ?"),
            format!("DELETE FROM main.{name}"),
            format!("INSERT OR REPLACE INTO {name} (character_id, delta) VALUES (?, ?)"),
            format!("REPLACE INTO {name} (character_id, delta) VALUES (?, ?)"),
            format!(
                "INSERT INTO {name} (character_id, session_id, axis, delta) VALUES (?, ?, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET delta = excluded.delta"
            ),
            format!("DROP TABLE {name}"),
            // 前の文の末尾の `ON` を借りる形（記憶の側が実際に踏んだ）。
            format!("PRAGMA foreign_keys = ON;\nDELETE FROM {name};"),
            format!("ON DELETE FROM {name} WHERE id = ?"),
            // **記憶の側では許可される形。** 票には更新の用途が無いので落とす。
            format!("UPDATE {name} SET superseded_by = ? WHERE id = ?"),
        ];

        for evasion in &evasions {
            assert!(
                !vote_violations(evasion, &table).is_empty(),
                "この形を落とせていません: {evasion}"
            );
        }
    }

    /// **許可リストに無い形は、破壊的な語が無くても落ちる**（既定は拒否）。
    ///
    /// # なぜ別のテストが要るのか（16d のレビューで実測）
    ///
    /// `vote_form_is_allowed` の `_ => false` を **`_ => true`** にしても
    /// **1338 全緑だった。**
    /// 上の evasion 一覧はどれも破壊的な語（`UPDATE` / `DELETE` / `CONFLICT` …）を含み、
    /// **`vote_destructive_in` が先に落とす**ので、
    /// **既定の枝が効く領域に1件も入っていなかった。**
    ///
    /// # 実害は「設計判断そのものが観測されない」こと
    ///
    /// モジュールの走査は **「禁止語の列挙ではなく許可リスト」** という判断の上に立っている。
    /// `_ => true` になった瞬間、**判定は禁止語の列挙に退化**し、
    /// **新しい形は `vote_form_is_allowed` を1文字も触らずに通る**
    /// ——レビューの掛かる場所が消える。
    ///
    /// ここに並べたのは**破壊的な語を1つも含まない、許可されていない形**である。
    #[test]
    fn the_vote_scan_rejects_forms_that_are_not_on_the_allowlist() {
        let table = vote_table_word();
        let name = table.to_lowercase();

        for statement in [
            // 表の名前が値の位置に来る形（`FROM` でも `INTO` でもない）。
            format!("SELECT 1 FROM other WHERE x = {name}"),
            format!("PRAGMA table_info({name})"),
        ] {
            assert!(
                vote_destructive_in(&vote_tokens(&statement)).is_none(),
                "前提: 破壊的な語を含まない文であること（既定の枝でしか落ちない）: {statement}"
            );
            assert!(
                !vote_violations(&statement, &table).is_empty(),
                "許可リストに無い形が通っています: {statement}"
            );
        }
    }

    /// **設計上ありうる形は落とさない。**
    ///
    /// ここを狭めると、追記と読み出しそのものが書けなくなって
    /// **走査ごと外される**羽目になる。
    #[test]
    fn the_vote_scan_accepts_the_statements_the_design_allows() {
        let table = vote_table_word();
        let name = table.to_lowercase();
        let allowed = [
            format!(
                "INSERT INTO {name} (character_id, session_id, axis, delta) VALUES (?, ?, ?, ?)"
            ),
            format!("SELECT session_id, axis, delta FROM {name} WHERE character_id = ? LIMIT ?"),
            format!("SELECT COUNT(*) FROM {name}"),
            // migrations の DDL。参照アクションの `ON DELETE CASCADE` を含む形も通す。
            format!(
                "CREATE TABLE {name} (id INTEGER PRIMARY KEY, \
                 character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE)"
            ),
            format!("CREATE INDEX {name}_recent_idx ON {name} (character_id, created_at)"),
            format!("PRAGMA foreign_keys = ON;\nSELECT axis FROM {name} WHERE character_id = ?;"),
        ];

        for statement in &allowed {
            let violations = vote_violations(statement, &table);
            assert!(
                violations.is_empty(),
                "正当な文を落としています: {statement} / {violations:?}"
            );
        }
    }

    /// **走査は `src` 全体と `migrations` を見る。**
    ///
    /// `store` に絞ると、`src/server/admin.rs` に1行書くだけで回避できる
    /// （ADR-0008 が「`server` に生の SQL を書けば走査の外に出る」と明示している）。
    #[test]
    fn the_vote_scan_covers_every_module_and_the_migrations() {
        let roots: Vec<&str> = VOTE_SCAN_ROOTS.iter().map(|(root, _)| *root).collect();
        assert!(roots.contains(&"src"), "src 全体を見ること: {roots:?}");
        assert!(
            roots.contains(&"migrations"),
            "migrations を外さないこと: {roots:?}"
        );

        let separator = std::path::MAIN_SEPARATOR;
        let scanned = vote_scan_targets();
        for module in ["server", "prompt", "llm", "domain", "parse"] {
            let expected = format!("src{separator}{module}{separator}");
            assert!(
                scanned.iter().any(|(path, _)| path.contains(&expected)),
                "{module} が走査対象に入っていない: {:?}",
                scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
            );
        }
        assert!(
            scanned.iter().any(|(path, _)| path.ends_with(".sql")),
            "migrations の .sql を拾えていない: {:?}",
            scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );
    }

    /// 走査対象を「パス, ソース」で返す。**`.sql` は行コメントを落としてから。**
    fn vote_scan_targets() -> Vec<(String, String)> {
        let mut targets = Vec::new();
        for (root, extension) in VOTE_SCAN_ROOTS {
            for (path, source) in vote_source_files(root, extension) {
                let text = if extension == "sql" {
                    strip_sql_line_comments(&source)
                } else {
                    source
                };
                targets.push((path, text));
            }
        }
        targets
    }

    /// `root` 配下の拡張子 `extension` のファイルを「パス, 生ソース」で返す。
    fn vote_source_files(root: &str, extension: &str) -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        let mut stack = vec![manifest.join(root)];

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|ext| ext == extension) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                files.push((path.display().to_string(), source));
            }
        }
        files
    }

    /// SQL の行コメント以降を落とす。**`.sql` 専用。**
    fn strip_sql_line_comments(source: &str) -> String {
        let marker = ["-", "-"].concat();
        source
            .lines()
            .map(|line| line.split(&marker).next().unwrap_or(""))
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// ソースをトークン列に均す（`store::memory::sql_tokens` と同じ規則）。
    fn vote_tokens(source: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut word = String::new();

        for ch in source.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_uppercase());
                continue;
            }
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
        }
        if !word.is_empty() {
            tokens.push(word);
        }
        tokens
    }

    /// ソース中のテーブル名の言及数。
    fn vote_mentions_of(source: &str, table: &str) -> usize {
        vote_tokens(source)
            .iter()
            .filter(|token| token.as_str() == table)
            .count()
    }

    /// **走査本体。** 見張り番も対照も**この関数を通す**こと。
    ///
    /// 判定の前に `;` で文へ割る。割らないと**前の文の末尾**が現在の文の文脈として
    /// 読まれる（記憶の側が `PRAGMA foreign_keys = ON;` で実際に踏んだ）。
    fn vote_violations(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| vote_statement_violations(statement, table))
            .collect()
    }

    /// 違反メッセージに載せる抜粋の幅（トークン数）。**判定には使わない。**
    const VOTE_LOOK_BEHIND: usize = 4;
    const VOTE_LOOK_AHEAD: usize = 8;

    /// 1文の中の違反を返す。**破壊的な語は文全体を見る**（窓で見ない）。
    fn vote_statement_violations(statement: &str, table: &str) -> Vec<String> {
        let tokens = vote_tokens(statement);
        let mut found = Vec::new();

        for (index, token) in tokens.iter().enumerate() {
            if token.as_str() != table {
                continue;
            }
            let from = index.saturating_sub(VOTE_LOOK_BEHIND);
            let to = (index + VOTE_LOOK_AHEAD + 1).min(tokens.len());
            let excerpt = tokens[from..to].join(" ");

            if let Some(keyword) = vote_destructive_in(&tokens) {
                found.push(format!("同じ文に {keyword} があります: {excerpt}"));
            } else if !vote_form_is_allowed(&tokens, index) {
                found.push(format!("許可されていない形です: {excerpt}"));
            }
        }
        found
    }

    /// 文中に破壊的な語があるか。
    ///
    /// **外部キーの参照アクション（`ON DELETE CASCADE` 等）だけは除く。**
    /// 票の表自身が持っているので、除かないと常に赤になる。
    /// 除外は「直前が `ON`」だけでは足りない——参照アクションの次に `FROM` は来ない。
    fn vote_destructive_in(tokens: &[String]) -> Option<&'static str> {
        for (index, token) in tokens.iter().enumerate() {
            let reference_action = token.as_str() == "DELETE"
                && index > 0
                && tokens[index - 1] == "ON"
                && tokens.get(index + 1).map_or("", String::as_str) != "FROM";
            if reference_action {
                continue;
            }
            if let Some(found) = VOTE_DESTRUCTIVE
                .iter()
                .find(|word| **word == token.as_str())
            {
                return Some(found);
            }
        }
        None
    }

    /// テーブル名の言及が**許可された形**か。判定は直前の語で行う。
    ///
    /// 許すのは**追記・読み出し・DDL** だけ。記憶の側と違い、
    /// **`UPDATE` を通す形が1つも無い**（票に更新の用途が無い / ADR-0026 §決定3）。
    fn vote_form_is_allowed(tokens: &[String], index: usize) -> bool {
        let before = |back: usize| -> &str {
            index
                .checked_sub(back)
                .and_then(|at| tokens.get(at))
                .map_or("", String::as_str)
        };

        match before(1) {
            // 追記。
            "INTO" => before(2) == "INSERT",
            // 読み出し。`DELETE FROM` は `vote_destructive_in` が先に落とす。
            "FROM" => true,
            // 以下は migrations の DDL。
            "TABLE" => before(2) == "CREATE",
            "ON" => before(3) == "INDEX",
            "REFERENCES" => true,
            _ => false,
        }
    }

    // ---------------------------------------------------------------------
    // 人が読むための読み（作業単位 D-4a）
    // ---------------------------------------------------------------------

    /// **履歴を引く文が、絞り込みと並びと上限を全部持っている。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `WHERE character_id = ?` | **別のキャラクターの票が一覧に混ざる** |
    /// | `ORDER BY created_at DESC` | 「直近」でなくなる（古い票から N 件） |
    /// | `, id DESC` | 同着の順が挿入順まかせになる |
    /// | `LIMIT ?` | 会話が長くなるほど応答が伸び続ける |
    /// | `created_at` | **いつ動いたのかが読めない**（この入口の存在理由が消える） |
    ///
    /// 実挙動は `tests/server_admin_drift.rs` が持つ。ここは速報。
    #[test]
    fn the_history_query_keeps_its_filter_order_limit_and_time() {
        for fragment in [
            "created_at",
            "WHERE character_id = ?",
            "ORDER BY created_at DESC, id DESC",
            "LIMIT ?",
        ] {
            assert!(
                SELECT_VOTE_HISTORY.contains(fragment),
                "{fragment} が消えています: {SELECT_VOTE_HISTORY}"
            );
        }
    }

    /// **2つの「直近 N 件」が同じ並びを名乗る。**
    ///
    /// 蓄積が読む側（[`SELECT_RECENT_VOTES`]）と人が読む側（[`SELECT_VOTE_HISTORY`]）は
    /// **別の文**である（返す列が違う）。片方だけ並びが変わると、
    /// **同じ言葉が2つの意味を持ち、画面とサブLLM が違う票を「直近」と呼ぶ。**
    ///
    /// 断片は**両方の文の外にある直書き**なので、片方に合わせて緩む経路が無い。
    #[test]
    fn the_two_recent_reads_ask_for_the_same_order() {
        for statement in [SELECT_RECENT_VOTES, SELECT_VOTE_HISTORY] {
            assert!(
                statement.contains("ORDER BY created_at DESC, id DESC"),
                "並びが揃っていません: {statement}"
            );
            assert!(
                statement.contains("WHERE character_id = ?"),
                "絞り込みが揃っていません: {statement}"
            );
        }
    }

    /// **数える2つの文は、上限で切らない。**
    ///
    /// 総数に `LIMIT` が付くと、**上限に達した瞬間に総数が上限で止まる**
    /// ——画面は「全部で 50 件のうち 50 件」と表示し、**切られたことが読めなくなる。**
    /// 集計に付けば、軸ごとの件数が静かに減る。
    ///
    /// **絞り込みは両方が持つ**（別のキャラクターの票を数えない）。
    #[test]
    fn the_counting_queries_are_never_limited() {
        for statement in [COUNT_VOTES, TALLY_VOTES] {
            assert!(
                !statement.contains("LIMIT"),
                "数える文に上限を付けないこと: {statement}"
            );
            assert!(
                statement.contains("WHERE character_id = ?"),
                "絞り込みが消えています: {statement}"
            );
        }
        assert!(
            TALLY_VOTES.contains("GROUP BY axis, delta"),
            "軸と向きの組ごとに数えること: {TALLY_VOTES}"
        );
    }

    /// **人が読むための3つの文も、プレースホルダで値を渡す**（絶対制約）。
    ///
    /// 向きの綴りを `CASE WHEN delta = 'up'` と書く形は、
    /// ここで**リテラルとして落ちる**（[`delta_to_db`] の写しも同時に防がれる）。
    #[test]
    fn the_history_statements_bind_every_value_through_a_placeholder() {
        for statement in [SELECT_VOTE_HISTORY, COUNT_VOTES, TALLY_VOTES] {
            assert!(
                !statement.contains(quote_char()),
                "リテラルを埋め込まないこと: {statement}"
            );
            assert!(
                statement.contains(placeholder_char()),
                "プレースホルダが1つも無い: {statement}"
            );
        }
    }

    /// テストで使う軸名。
    fn axis(raw: &str) -> StatAxis {
        StatAxis::parse(raw).expect("テストで使う軸名は正当であること")
    }

    /// **畳んだ結果が、軸ごと・向きごとに分かれたまま。**
    ///
    /// # 期待値を入力から作らない
    ///
    /// 件数は**全部違う数**（4 / 1 / 7）にしてあり、直書きで押さえる。
    /// 上げと下げを入れ替える変異も、軸を取り違える変異も、ここで赤になる。
    #[test]
    fn folding_keeps_each_axis_and_each_direction_apart() {
        let counted = [
            (axis("パッション"), DriftDelta::Up, 4),
            (axis("ロジック"), DriftDelta::Down, 1),
            (axis("パッション"), DriftDelta::Down, 7),
        ];

        let tallies = fold_tallies(&counted);

        assert_eq!(tallies.len(), 2, "軸ごとに1件へ畳まれること: {tallies:?}");
        assert_eq!(*tallies[0].axis(), axis("パッション"));
        assert_eq!(tallies[0].ups(), 4, "上げの数");
        assert_eq!(tallies[0].downs(), 7, "下げの数");
        assert_eq!(*tallies[1].axis(), axis("ロジック"));
        assert_eq!(tallies[1].ups(), 0, "票の無い向きは 0");
        assert_eq!(tallies[1].downs(), 1);
    }

    /// **1件も無ければ空。** 「まだ何も起きていない」はエラーではない。
    #[test]
    fn folding_nothing_yields_nothing() {
        assert!(fold_tallies(&[]).is_empty());
    }

    // ---------------------------------------------------------------------
    // 累積の読み（作業単位 D-5a）
    // ---------------------------------------------------------------------

    /// **累積の読みが、絞り込みと3つの列を全部持っている。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `axis` | どの軸の累積か分からない |
    /// | `charge` | 累積そのものが取れない |
    /// | **`updated_at`** | **いつまで変容が走っていたのかが表から読めない**（D-5a の是正点） |
    /// | `WHERE character_id = ?` | **別のキャラクターの累積が混ざる** |
    ///
    /// **`LIMIT` は持たない**——軸の数だけしか行が返らない
    /// （`stat_alloc` の主キーに軸名が含まれるので1軸1行）。
    ///
    /// 実挙動は `tests/store_drift_charge.rs` が持つ。**ここは速報。**
    #[test]
    fn the_charge_query_keeps_its_filter_and_every_column() {
        for fragment in ["axis", "charge", "updated_at", "WHERE character_id = ?"] {
            assert!(
                SELECT_CHARGES.contains(fragment),
                "{fragment} が消えています: {SELECT_CHARGES}"
            );
        }
        assert!(
            !SELECT_CHARGES.contains("LIMIT"),
            "累積の読みに上限を付けないこと: {SELECT_CHARGES}"
        );
    }

    /// **累積の読みも、文字列連結ではなくプレースホルダで値を渡す**（絶対制約）。
    ///
    /// **[`UPSERT_CHARGE`] はここに入れない**——`strftime('%Y-...', 'now')` の
    /// リテラルを含むが、それは**値の埋め込みではなく SQL の関数呼び出し**である。
    #[test]
    fn the_charge_query_binds_every_value_through_a_placeholder() {
        assert!(
            !SELECT_CHARGES.contains(quote_char()),
            "リテラルを埋め込まないこと: {SELECT_CHARGES}"
        );
        assert!(
            SELECT_CHARGES.contains(placeholder_char()),
            "プレースホルダが1つも無い: {SELECT_CHARGES}"
        );
    }

    /// **書き戻す文が、累積と時刻の両方を進める。**
    ///
    /// 時刻だけを落とす変異は `tests/store_drift_charge.rs::
    /// writing_again_moves_the_timestamp_forward` が赤にするが、
    /// **その1本が消えたときに気付けるよう**、文面の側でも押さえておく。
    #[test]
    fn the_upsert_moves_both_the_charge_and_the_time() {
        for fragment in ["charge = excluded.charge", "updated_at = strftime"] {
            assert!(
                UPSERT_CHARGE.contains(fragment),
                "{fragment} が消えています: {UPSERT_CHARGE}"
            );
        }
    }
}
