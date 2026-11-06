-- personacore 初回マイグレーション
--
-- 設計方針（docs/03-data-model.md / ADR-0004 / ADR-0006）:
--   * 主キーは全テーブル INTEGER PRIMARY KEY（rowid エイリアス）。UUID は使わない。
--     セッション ID は `model` フィールドに `aoi@42` の形で人が打つ可能性があるため短く保つ。
--   * 日時は TEXT の ISO-8601 UTC。sqlite3 CLI で直接覗いたときに読めることを優先する。
--   * フェーズ2・3で使う列も初回にすべて含める（後付けマイグレーションを増やさないため）。
--   * CLAUDE.md の「絶対制約」は、アプリ層の注意ではなくスキーマの制約として表現する。
--
-- 外部キー制約は接続ごとに `PRAGMA foreign_keys = ON` で有効化する（src/store/mod.rs）。
-- SQLite は既定で無効なので、有効化を忘れると FK が黙って効かなくなる。
-- なお本ファイル内で PRAGMA を発行してもトランザクション内では無視されるため、書かない。

-- キャラクター定義
CREATE TABLE character (
    id INTEGER PRIMARY KEY,
    -- `@` は `model` フィールドの予約語（`aoi@new` / `aoi@<session-id>`）と衝突するため禁止する。
    -- ADR-0004: アプリ層の検証だけに頼らず、スキーマ側でも書けなくする。
    --
    -- 空文字・空白のみも禁止する。ADR-0004 では `model` をキャラクター名の許可リストと
    -- 照合するため、名前が '' のキャラが1件でもあると `model` が空のリクエストが 404 にならず、
    -- そのキャラに書き込み始める。
    --
    -- SQLite の `trim(X)`（1引数）は**半角スペースしか除かない**ため、それだけでは
    -- TAB / LF / VT / FF / CR / 全角空白(U+3000) / NBSP(U+00A0) のみの名前が素通りする。
    -- 日本語環境では全角空白の混入が現実に起こるので、除去対象を明示的に列挙する。
    --
    -- なお **Unicode の空白文字すべてを網羅するわけではない**（列挙した7種＋半角スペースのみ）。
    -- U+2000 系 / U+200B / U+FEFF / U+0085 などは通る。列挙では原理的に届かないので、
    -- 網羅が必要になったらアプリ層の正規化に寄せること。
    name TEXT NOT NULL UNIQUE
        CHECK (name NOT LIKE '%@%')
        CHECK (length(trim(
            name,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0),
    base_persona TEXT NOT NULL,
    -- 台詞 / 地の文の形式指示。未設定ならアプリ側の既定値を使うため NULL 可。
    output_format_instruction TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- 性格ステータス配分。軸は固定6軸にせず任意の文字列（軸自体が検証対象のため）。
CREATE TABLE stat_alloc (
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    -- 軸名が空だと、プロンプトに**軸名のない「: 3/5」**が載る。
    -- 空白の扱いは character.name と同じ（半角スペースだけでは足りない理由もそちらを参照）。
    axis TEXT NOT NULL CHECK (length(trim(
        axis,
        ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
    )) > 0),
    -- 「性格値が 0 の軸はプロンプトに含めない」を `value != 0` で実装したとき、
    -- 負値が残っていると「パッション: -3/5」がそのままプロンプトに載る。0 は許可する
    -- （「軸ごと落とす」対象として有効な値）。
    --
    -- `typeof` を縛るのは SQLite の型アフィニティ対策。INTEGER 宣言の列でも `'abc'` は
    -- 数値化できず TEXT のまま格納され、比較規則上 TEXT > INTEGER なので
    -- `value >= 0` が真になって**素通りする**。小数（REAL）も同様。
    --
    -- 注意: これは「文字列を一律拒否する」制約ではない。`'3'` や `3.0` は
    -- アフィニティ変換で INTEGER に正規化されてから CHECK を通るため**受理される**。
    -- 拒否されるのは正規化できない値（`'abc'` / `'3abc'` / `'３'` / `2.5`）だけ。
    -- **アプリ層の入力検証を省いてよい理由にはならない。**
    value INTEGER NOT NULL CHECK (typeof(value) = 'integer' AND value >= 0),
    PRIMARY KEY (character_id, axis)
);

-- 会話セッション
CREATE TABLE session (
    id INTEGER PRIMARY KEY,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    -- 人間向けの識別名。自動生成しない場合は NULL。
    label TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- `aoi` 指定時の「最新セッションを継続」を引くための索引。
CREATE INDEX session_character_recent_idx ON session (character_id, updated_at DESC);

-- メッセージ
CREATE TABLE message (
    id INTEGER PRIMARY KEY,
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
    UNIQUE (id, session_id)
);

CREATE INDEX message_session_idx ON message (session_id, id);

-- 記憶
CREATE TABLE memory (
    id INTEGER PRIMARY KEY,
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    -- user / character の区別を落とすと一人称が混線するため NOT NULL + CHECK で必須にする。
    subject TEXT NOT NULL CHECK (subject IN ('user', 'character')),
    content TEXT NOT NULL,
    category TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- 矛盾解消は「上書き削除」ではなく「時系列保持」。古い行を DELETE / UPDATE で潰さず、
    -- 新しい行を追加して古い行からここで指す（フェーズ2で使用）。
    --
    -- **この不変条件はスキーマでは守れない。** 下の外部キーが防ぐのは参照先である
    -- **後継行**（新しい行）の消失だけで、保持したい**古い行**（参照元）は素通しで消える。
    -- `DELETE FROM memory WHERE superseded_by IS NOT NULL` は「もう上書き済みだから
    -- 消してよい行」を選ぶ最も自然なクエリでありながら、保持したい行だけを正確に消す。
    -- **古い行の保護はアプリ層の責務**（tests/store_schema.rs で実挙動を固定してある）。
    --
    -- 複合外部キーで character_id を巻き込み、**別キャラの記憶を指せなくする**。
    -- 別キャラを指せると、フェーズ2の遡行がキャラをまたいで記憶を混線させる。
    -- superseded_by が NULL のときは MATCH SIMPLE により制約は満たされる。
    -- 自己参照は遡行を無限ループにするため CHECK で潰す
    -- （`1→2` かつ `2→1` の循環は防げないので、辿る側に深さ制限を持たせること）。
    superseded_by INTEGER CHECK (superseded_by <> id),
    -- `WHERE locked = 1` で保護対象を選ぶ実装から、`locked = 7` のような行が
    -- 静かに漏れるのを防ぐ。
    locked INTEGER NOT NULL DEFAULT 0 CHECK (locked IN (0, 1)),
    -- 複合外部キーの参照先に UNIQUE 索引が要る（message.UNIQUE (id, session_id) と同じ理由）。
    UNIQUE (id, character_id),
    FOREIGN KEY (superseded_by, character_id) REFERENCES memory(id, character_id)
);

CREATE INDEX memory_character_idx ON memory (character_id, created_at);

-- 会話要約（フェーズ2。フェーズ1では書き込まないが列は今作る）
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

-- キャラクターの外見定義（画像生成用・フェーズ3）。キャラクターにつき1行。
CREATE TABLE character_appearance (
    character_id INTEGER PRIMARY KEY REFERENCES character(id) ON DELETE CASCADE,
    -- 固定部のタグ列（髪色・髪型・目の色・基本の服装）。一貫性の土台。
    fixed_tags TEXT NOT NULL,
    -- 将来の LoRA 指定。現時点では NULL。
    lora_name TEXT,
    lora_strength REAL
);

-- 生成した画像の記録（フェーズ3）
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
