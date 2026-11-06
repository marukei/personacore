-- session.id / message.id を AUTOINCREMENT で作り直す（ADR-0010）。
--
-- SQLite は AUTOINCREMENT なしの INTEGER PRIMARY KEY で、最大 rowid の行を削除すると
-- id を再利用する。session.id は `model` フィールドに `aoi@42` の形で **DB の外へ出る**
-- ため、再利用されると外に残った `aoi@42` が別のセッションを指す（ADR-0004 の記憶の混線）。
-- AUTOINCREMENT は ALTER TABLE では足せないので、テーブルごと作り直す。
--
-- ============================================================================
-- なぜ SQLite 公式の「テーブル再構築 12 ステップ」どおりに書けないのか
-- ============================================================================
--
-- 公式手順は `PRAGMA foreign_keys = OFF` を前提にしているが、
-- **このプラグマはトランザクションの中では無視される**（0001 の冒頭にも書いてある）。
-- sqlx はマイグレーションをトランザクションで包むため、ここでは外部キーを切れない。
-- 実測: トランザクション外で OFF にすると 0 になるが、トランザクション内では 1 のまま。
--
-- 外部キーが有効なままだと、次の3つが起きる。1 と 2 は**黙って**起きる。
--
--   1. DROP TABLE は暗黙の DELETE FROM を伴うため、親テーブルを落とすと
--      子テーブルの行が ON DELETE CASCADE で**消える**
--      （実測: message を1件入れた状態で DROP TABLE session すると message が 0 件になる）。
--   2. ALTER TABLE ... RENAME は、他テーブルの REFERENCES 句を**書き換える**。
--   3. 親テーブルが存在しない状態で子テーブルに DML を出すと
--      `no such table: main.<親>` になる。DROP TABLE の暗黙の DELETE FROM も同じで、
--      **「消えた親を参照している子テーブル」が残っているとカスケードを組み立てられない。**
--
-- 3 のため、**session と message だけを落とすことはできない。**
-- summary / generated_image は両方を参照しているので、先に落とす必要がある。
-- 「再構築の対象は最小に」と「外部キーを切れない」を両立できないため、
-- 参照している4テーブルすべてを作り直す。**0001 との差は AUTOINCREMENT の2箇所だけ**で、
-- 他の列・CHECK・UNIQUE・複合外部キー・索引は 0001 と同一にしてある。
--
-- 退避先は CREATE TABLE ... AS SELECT で作る。制約も外部キーも持たないので、
-- この間の出し入れが制約に引っかからない。戻すときは外部キーが有効なまま INSERT するので、
-- **複合外部キーを含む参照整合性が実際に検証される。**
--
-- なお PRAGMA defer_foreign_keys はトランザクション内でも効くが、これは違反の
-- **検査を COMMIT まで遅らせる**だけで、ON DELETE CASCADE の**動作は止めない**。
-- 上の 1 には効かないので使っていない。

-- 退避。列は明示する（* に頼ると列順の思い込みが混ざる）。
CREATE TABLE session_backup AS
    SELECT id, character_id, label, created_at, updated_at FROM session;
CREATE TABLE message_backup AS
    SELECT id, session_id, role, dialogue, narration, raw_content, created_at FROM message;
CREATE TABLE summary_backup AS
    SELECT id, session_id, upto_message_id, content, created_at FROM summary;
CREATE TABLE generated_image_backup AS
    SELECT id, session_id, message_id, positive_prompt, negative_prompt, seed,
           checkpoint, file_path, created_at
    FROM generated_image;

-- 落とす順は**子から親へ**。逆順にすると、上の 3 で
-- `no such table` になるか、カスケードで行が消える。
DROP TABLE generated_image;
DROP TABLE summary;
DROP TABLE message;
DROP TABLE session;

-- 会話セッション。**0001 との違いは AUTOINCREMENT だけ。**
CREATE TABLE session (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    -- 人間向けの識別名。自動生成しない場合は NULL。
    label TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- `aoi` 指定時の「最新セッションを継続」を引くための索引。
CREATE INDEX session_character_recent_idx ON session (character_id, updated_at DESC);

-- メッセージ。**0001 との違いは AUTOINCREMENT だけ。**
CREATE TABLE message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    -- 構造化出力の分解結果。パースに失敗した場合は両方 NULL になりうる。
    dialogue TEXT,
    narration TEXT,
    -- パース前の生出力。パース失敗時の唯一の調査手段なので必ず保持する。
    raw_content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- `summary` / `generated_image` から「同一セッションのメッセージであること」を
    -- 複合外部キーで縛るための親キー。SQLite は複合外部キーの参照先に
    -- UNIQUE 索引を要求するため、id が主キーでもこの UNIQUE が必要。
    -- **落とすと、参照側の INSERT が丸ごと foreign key mismatch で失敗する。**
    UNIQUE (id, session_id)
);

CREATE INDEX message_session_idx ON message (session_id, id);

-- 会話要約。**0001 と同一**（作り直す message を参照しているため巻き込まれただけ）。
CREATE TABLE summary (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    -- どこまでを要約したか。
    upto_message_id INTEGER NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- 複合外部キーで session_id を巻き込み、**別セッションのメッセージを指せなくする**。
    -- 別セッションを指せると、要約が他の会話の内容を取り込む。
    FOREIGN KEY (upto_message_id, session_id) REFERENCES message(id, session_id)
);

CREATE INDEX summary_session_idx ON summary (session_id, upto_message_id);

-- 生成した画像の記録。**0001 と同一**（巻き込まれただけ）。
CREATE TABLE generated_image (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    -- どのメッセージに対する挿絵か。メッセージに紐付かない生成もありうるため NULL 可。
    -- 複合外部キー（下部）で session_id を巻き込み、別セッションのメッセージを指せなくする。
    -- NULL のときは MATCH SIMPLE により制約は満たされる。
    message_id INTEGER,
    -- 再現性のため、組み立て後の生のプロンプトとシードを必ず保持する。
    -- 辞書やテンプレートを更新した後では当時のプロンプトは再現できない。
    positive_prompt TEXT NOT NULL,
    negative_prompt TEXT NOT NULL,
    -- 型アフィニティ対策。NOT NULL だけでは `'abc'` が TEXT のまま格納されてしまい、
    -- 壊れたことに気づくのは**読み出し時＝再現したくなった瞬間**になる。
    seed INTEGER NOT NULL CHECK (typeof(seed) = 'integer'),
    -- チェックポイントが違えば同じ seed でも再現しないため、seed と同格で必須。
    checkpoint TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (message_id, session_id) REFERENCES message(id, session_id)
);

CREATE INDEX generated_image_session_idx ON generated_image (session_id, id);

-- 退避した行を戻す。**親から子へ**。
-- id を明示して入れるので、外に出た `aoi@42` は同じセッションを指し続ける。
-- 明示した id は sqlite_sequence にも反映されるため、**採番は最大値の続きから**始まる。
INSERT INTO session (id, character_id, label, created_at, updated_at)
    SELECT id, character_id, label, created_at, updated_at FROM session_backup;
INSERT INTO message (id, session_id, role, dialogue, narration, raw_content, created_at)
    SELECT id, session_id, role, dialogue, narration, raw_content, created_at
    FROM message_backup;
INSERT INTO summary (id, session_id, upto_message_id, content, created_at)
    SELECT id, session_id, upto_message_id, content, created_at FROM summary_backup;
INSERT INTO generated_image (id, session_id, message_id, positive_prompt, negative_prompt,
                             seed, checkpoint, file_path, created_at)
    SELECT id, session_id, message_id, positive_prompt, negative_prompt, seed,
           checkpoint, file_path, created_at
    FROM generated_image_backup;

DROP TABLE generated_image_backup;
DROP TABLE summary_backup;
DROP TABLE message_backup;
DROP TABLE session_backup;
