-- 性格ステータスの変容の「票」を置くテーブル
-- （ADR-0026 §決定3 / 作業単位 16d）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「何が入るか」「追記のみである理由」「向きを綴りで持つ理由」「索引の理由」
-- 「1行壊れたときに何が止まるか」は docs/03-data-model.md の stat_drift_vote の節が正。
--
-- **適用済みのマイグレーションは二度と編集できない**（sqlx のチェックサム）ので、
-- ここに設計判断を写すと、判断が変わった日に古い写しが永久に残る。
-- 残すのは「この SQL を読むときにだけ要る注意」だけにする。
--
-- ============================================================================
-- 名前に mem+ory を含めない
-- ============================================================================
--
-- 記憶の SQL 走査テスト（src/store/memory.rs の no_statement_updates_or_deletes_a_memory）は
-- **テーブル名の文字列**で走査する。その名前を含むテーブルを作ると、
-- そこへの UPDATE / DELETE が「記憶を潰す文」として誤検出される。0009 と同じ注意。
--
-- ============================================================================
-- この表を書き換える文を足さないこと
-- ============================================================================
--
-- 追記のみ。見張り番は src/store/drift.rs の no_statement_updates_or_deletes_a_vote で、
-- src/** と migrations/** の両方を走査する（整理 SQL はマイグレーションに
-- 書かれるのが最もありそうなので、そちらも対象に入れてある）。
--
-- ============================================================================
-- axis の CHECK が3条件ある理由（0009 が実測した穴）
-- ============================================================================
--
-- * 空白のみを弾く条件は stat_alloc.axis の写し（0001）。半角スペースだけでは足りない。
-- * typeof を縛るのは型アフィニティ対策。BLOB は TEXT へ変換されないので、
--   縛らないと「読み出しでだけ落ちる行」が作れる。
-- * SQLite の length() は**テキストを最初の NUL までしか数えない**ので、
--   NUL そのものを instr で見る必要がある。
--
-- **長さの CHECK は置かない。** ここへ来る軸名は stat_alloc から読み出して
-- StatAxis::parse を通った値なので、stat_alloc が許す形より狭い制約を置くと
-- **正当な票だけが静かに落ちる。**
--
-- 3条件が全部効いていることは tests/store_schema.rs の
-- drift_vote_axis_check_rejects_blank_blob_and_nul が実挙動で見張る。
CREATE TABLE stat_drift_vote (
    id INTEGER PRIMARY KEY,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    axis TEXT NOT NULL CHECK (
        typeof(axis) = 'text'
        AND instr(axis, char(0)) = 0
        AND length(trim(
            axis,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0
    ),
    -- 綴りの正は domain::DriftDelta::label（src/store/drift.rs の delta_to_db が呼ぶだけ）。
    delta TEXT NOT NULL CHECK (delta IN ('up', 'down')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 「そのキャラクターの直近 N 件の票」を引くための索引。
CREATE INDEX stat_drift_vote_recent_idx ON stat_drift_vote (character_id, created_at);
