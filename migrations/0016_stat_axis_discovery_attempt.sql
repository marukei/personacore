-- 軸の候補を尋ねた「試み」の記録（ADR-0046 §決定2 / 作業単位 A-9b / Issue #42）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「なぜ候補の表とは別に要るのか」「目印をどう読むか」は docs/03-data-model.md の
-- stat_axis_candidate の節が正（0015 と同じ理由 —— 適用済みの移行は編集できない）。
--
-- ============================================================================
-- 既存の DB の振る舞いを変えない
-- ============================================================================
--
-- 新しい表を1枚足すだけ。空で始まり、既存の表には触らない。
--
-- ============================================================================
-- 名前に mem+ory / stat_drift+_vote / attribute+_revision を含めない
-- ============================================================================
--
-- 0015 と同じ注意。下の REFERENCES memory(...) は記憶の走査が許している形。
--
-- ============================================================================
-- 書き換える文を足さないこと
-- ============================================================================
--
-- 追記のみ。見張り番は src/store/axis_candidate.rs の走査（0015 の3枚と同じ走査に入っている）。
--
-- outcome の綴りの正は domain::AxisDiscoveryOutcome::label（ここは移行した時点の写し）。
-- created_at の綴りは 0010 / 0011 / 0013 / 0015 と揃える（D-5a の実測）。

-- 試み1回につき1行。through_memory_id は「その回に数え終えた記憶」（数えた記憶のうち最新）。
-- 次の試みは、そのキャラクター自身の記憶がそれより後に N 件増えてから。
--
-- 複合外部キーで同じキャラクターの記憶に縛る（0015 の source_memory_id と同じ形）。
CREATE TABLE stat_axis_discovery_attempt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    through_memory_id INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK (
        outcome IN ('proposed', 'duplicate', 'malformed', 'truncated', 'upstream_failed', 'not_recorded')
    ),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (through_memory_id, character_id) REFERENCES memory(id, character_id)
);

CREATE INDEX stat_axis_discovery_attempt_character_idx
    ON stat_axis_discovery_attempt (character_id, through_memory_id);
