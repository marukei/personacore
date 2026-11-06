-- 属性の変更を追記する表（ADR-0028 §決定5 / 作業単位 A-3a）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「何が入るか」「なぜ現在値の表と2枚あるのか」「1行壊れたときに何が止まるか」は
-- docs/03-data-model.md の attribute_revision の節が正。
--
-- **適用済みのマイグレーションは二度と編集できない**（sqlx のチェックサム）ので、
-- ここに設計判断を写すと、判断が変わった日に古い写しが永久に残る。
-- 残すのは「この SQL を読むときにだけ要る注意」だけにする。
--
-- ============================================================================
-- 名前に mem+ory / stat_drift+_vote を含めない
-- ============================================================================
--
-- 記憶の SQL 走査テスト（src/store/memory.rs の no_statement_updates_or_deletes_a_memory）と
-- 票の SQL 走査テスト（src/store/drift.rs の no_statement_updates_or_deletes_a_vote）は
-- **どちらもテーブル名の文字列**で migrations/** を走査する。
-- その名前を含むテーブルを作ると、そこへの UPDATE / DELETE が誤検出される。
-- 0009 / 0010 / 0011 と同じ注意。
--
-- ============================================================================
-- この表を書き換える文を足さないこと
-- ============================================================================
--
-- 追記のみ（票の表と同じ立場）。見張り番は src/store/attribute.rs の
-- no_statement_updates_or_deletes_a_revision で、src/** と migrations/** の両方を走査する
-- （整理 SQL はマイグレーションに書かれるのが最もありそうなので、そちらも対象に入れてある）。
--
-- 現在値の表（0009）への書き込みが1つのファイルに閉じていることも、同じファイルの
-- no_statement_outside_the_one_writer_touches_the_projection が走査する。
--
-- ============================================================================
-- created_at の綴りを 0011 / 0010 と揃える
-- ============================================================================
--
-- 既定を CURRENT_TIMESTAMP にすると綴りが 2 通りになり、**読み出しのデコードが
-- 行ごとに割れる**（CLAUDE.md の D-5a の実測。0011 と 0012 が同じ綴りだったので
-- 既存行が無傷だった、という前例がある）。ここも同じ綴りにする。
--
-- ============================================================================
-- key / before / after の CHECK は 0009 の写し
-- ============================================================================
--
-- 4条件（typeof / NUL / 空白のみ / 長さ）の理由は 0009 の冒頭にある。要点だけ:
--
--   * typeof を縛るのは型アフィニティ対策。BLOB は TEXT へ変換されないので、
--     縛らないと「読み出しでだけ落ちる行」が作れる。
--   * SQLite の length() は**テキストを最初の NUL までしか数えない**ので、
--     NUL そのものを instr で見る必要がある。
--   * 空白のみを弾く条件は stat_alloc.axis の写し（0001）。半角スペースだけでは足りない。
--
-- 長さは 0009 と同じ 12 / 20。**縮める向きは取れない**（保存済みの行が読めなくなる）。
-- 4条件が全部効いていることは tests/store_schema.rs の
-- attribute_revision_checks_reject_blank_blob_nul_and_over_long が実挙動で見張る。
--
-- before / after は NULL を許す。**NULL の意味は「無かった」/「消えた」**で、
-- 空文字ではない（空文字は 0009 と同じく CHECK が弾く）。
-- **両方が NULL の行は CHECK では止められない**（それぞれの CHECK は片方しか見ない）ので、
-- 読み出し側（src/store/attribute.rs）が InvalidStoredValue に落とす。
CREATE TABLE attribute_revision (
    -- **AUTOINCREMENT。** 同じミリ秒に複数行入るので、並びの第2キーになる。
    -- 行を消さない表なので再利用は起こらないが、id の単調性を宣言しておく。
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    -- 綴りは 0009 と揃える（src/store/memory.rs の subject_to_db が唯一の写し元）。
    subject TEXT NOT NULL CHECK (subject IN ('user', 'character')),
    key TEXT NOT NULL CHECK (
        typeof(key) = 'text'
        AND instr(key, char(0)) = 0
        AND length(trim(
            key,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0
        AND length(key) <= 12
    ),
    before TEXT CHECK (
        before IS NULL OR (
            typeof(before) = 'text'
            AND instr(before, char(0)) = 0
            AND length(trim(
                before,
                ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
            )) > 0
            AND length(before) <= 20
        )
    ),
    after TEXT CHECK (
        after IS NULL OR (
            typeof(after) = 'text'
            AND instr(after, char(0)) = 0
            AND length(trim(
                after,
                ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
            )) > 0
            AND length(after) <= 20
        )
    ),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 「そのキャラクターの履歴を新しい順に N 件」を引くための索引。
-- 主キーは id なので、これが無いと履歴の読みが全表走査になる（行は消えないので増え続ける）。
CREATE INDEX attribute_revision_recent_idx ON attribute_revision (character_id, created_at);
