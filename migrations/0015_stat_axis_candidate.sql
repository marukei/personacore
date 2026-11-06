-- 変容が見いだす軸の候補（ADR-0046 §決定1・§決定6 / 作業単位 A-9a）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「何が入るか」「なぜ表が3枚あるのか」「1行壊れたときに何が止まるか」は
-- docs/03-data-model.md の stat_axis_candidate の節が正。
--
-- **適用済みのマイグレーションは二度と編集できない**（sqlx のチェックサム）ので、
-- ここに設計判断を写すと、判断が変わった日に古い写しが永久に残る。
-- 残すのは「この SQL を読むときにだけ要る注意」だけにする。
--
-- ============================================================================
-- 既存の DB の振る舞いを変えない
-- ============================================================================
--
-- character.axis_discovery_slots は DEFAULT 0（見いださない）。既存の行は全部 0 になり、
-- 新しい3枚の表は空で始まる。**0 のキャラクターには候補が1件も作れない**
-- （書き手が空いた枠を数える / src/store/axis_candidate.rs）。
--
-- DEFAULT が CHECK を満たさないと ADD COLUMN そのものが失敗する（0004 / 0008 の実測）。
-- 0 は CHECK を満たす。
--
-- typeof を縛るのは型アフィニティ対策（0008 の stat_scale と同じ）。INTEGER の列は
-- '3' や 3.0 を整数へ直して入れるが、'abc' や 2.5 は直らずに残り、読み出しでだけ落ちる。
-- 上限 4 は domain::AxisDiscoverySlots::MAX の、移行した時点の写し。
-- **縮める向きは取れない**（保存済みの 4 が読めなくなる）。
--
-- ============================================================================
-- 名前に mem+ory / stat_drift+_vote / attribute+_revision を含めない
-- ============================================================================
--
-- 既存の SQL 走査（src/store/memory.rs / drift.rs / attribute.rs）は**テーブル名の語**で
-- migrations/** を走査する。その語を含むテーブルを作ると誤検出される（0009〜0013 と同じ注意）。
-- 下の REFERENCES memory(...) は記憶の走査が許している形（REFERENCES の直後）。
--
-- ============================================================================
-- 3枚とも、書き換える文を足さないこと
-- ============================================================================
--
-- 追記のみ。状態は移り変わりの行を足して進め、いまの状態は導出する。
-- 見張り番は src/store/axis_candidate.rs の走査で、src/** と migrations/** の両方を見る
-- （整理 SQL はマイグレーションに書かれるのが最もありそうなので、そちらも対象）。
--
-- ============================================================================
-- created_at の綴りを 0010 / 0011 / 0013 と揃える
-- ============================================================================
--
-- 既定を CURRENT_TIMESTAMP にすると綴りが2通りになり、読み出しのデコードが
-- 行ごとに割れる（CLAUDE.md の D-5a の実測）。

ALTER TABLE character ADD COLUMN axis_discovery_slots INTEGER NOT NULL DEFAULT 0
    CHECK (typeof(axis_discovery_slots) = 'integer' AND axis_discovery_slots BETWEEN 0 AND 4);

-- 候補1件につき1行（名前と出どころ）。状態はここに持たない（下の移り変わりの表）。
--
-- name の CHECK は 0013 の key と同じ4条件（typeof / NUL / 空白のみ / 長さ）。
-- 長さ 20 は domain::StatAxis::MAX_CHARS の、移行した時点の写し。
-- 名前は StatAxis::parse を通った値だけが来る（候補は、採用されれば stat_alloc の軸になる）。
--
-- UNIQUE (character_id, name): 同じキャラクターの中で同じ名前を二度使えない
-- （却下・不合格の名前を二度出させない / ADR-0046 §決定2・5）。**照合は完全一致。**
--
-- 出どころの記憶は複合外部キーで同じキャラクターの記憶に縛る
-- （memory の UNIQUE (id, character_id) が参照先。0001 の superseded_by と同じ形）。
CREATE TABLE stat_axis_candidate (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (
        typeof(name) = 'text'
        AND instr(name, char(0)) = 0
        AND length(trim(
            name,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0
        AND length(name) <= 20
    ),
    source_memory_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (character_id, name),
    FOREIGN KEY (source_memory_id, character_id) REFERENCES memory(id, character_id)
);

-- 状態の移り変わり（候補1件につき1〜3行）。**いまの状態は id の最も大きい行。**
--
-- 綴りの正は domain::AxisCandidateState::label / AxisTrialFailure::label。
-- ここは移行した時点の写しで、適用済みの移行は編集できないので、label を変えても変わらない。
-- 理由（reason）は不合格のときだけ持ち、不合格なら必ず持つ（最後の CHECK）。
--
-- 遷移の規則（試用 → 合格／不合格、合格 → 採用／却下）は CHECK では書けない
-- （行をまたぐ）。書き手が BEGIN IMMEDIATE の中で検査し、読み手も同じ規則で導出する
-- （domain::current_axis_candidate_state。崩れた列は読めない候補として一覧を止める）。
CREATE TABLE stat_axis_candidate_transition (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id INTEGER NOT NULL REFERENCES stat_axis_candidate(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK (state IN ('trial', 'passed', 'failed', 'adopted', 'rejected')),
    reason TEXT CHECK (reason IS NULL OR reason IN ('pinned', 'silent', 'too_few_samples')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((state = 'failed') = (reason IS NOT NULL))
);

CREATE INDEX stat_axis_candidate_transition_candidate_idx
    ON stat_axis_candidate_transition (candidate_id, id);

-- 試用の票（ADR-0046 §決定3・6）。**stat_drift_vote と混ぜない**
-- （固定の軸の集計・画面・測定の道具を1つも変えないため）。
--
-- 変容の票と違って `0` も記録する（試用の判定は `0` の率を数える / §決定4）。
-- 綴りの正は domain::AxisTrialVote::label（ここは移行した時点の写し）。
CREATE TABLE stat_axis_trial_vote (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id INTEGER NOT NULL REFERENCES stat_axis_candidate(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    vote TEXT NOT NULL CHECK (vote IN ('up', 'down', 'zero')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX stat_axis_trial_vote_candidate_idx ON stat_axis_trial_vote (candidate_id, vote);
