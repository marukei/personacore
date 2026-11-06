-- no-transaction
--
-- character.name を「ASCII 小文字英字・数字・`-`・`_` だけ」に絞る
-- （ADR-0017 / 作業単位 0004b-1）。
--
-- 識別子に残った仕事は**安定した鍵であること**だけになった。呼び名は 0004 が
-- display_name へ分け（ADR-0011）、識別子は ADR-0016 で不変になっている。
-- それでも `character.name` はほとんど自由文を受けており、**その自由が機構を要求していた**
-- （0002 の正規化キー索引・SELECT_BY_NORMALIZED_KEY・ConflictingUnnormalizedName）。
--
-- **空白を表現できなくすれば、ゴースト行（`" aoi "`）はそもそも作れない。**
-- 「防ぐ」のではなく「表現できなくする」側に寄せる。
--
-- ============================================================================
-- 1. なぜ `-- no-transaction` なのか（**この1行が移行の形を決めている**）
-- ============================================================================
--
-- SQLite に `ALTER TABLE ... ADD CONSTRAINT` は無いので、CHECK を足すには
-- テーブルを作り直すしかない。作り直しの公式手順（SQLite 公式「12 ステップ」）は
-- **`PRAGMA foreign_keys = OFF` を前提**にしているが、
-- **このプラグマはトランザクションの中では無視される。**
--
-- sqlx は既定でマイグレーションをトランザクションで包むため、0003 はここで詰まり
-- 「外部キーを切れない → 親を落とすには子を全部落とすしかない」と進んだ。
-- **しかし sqlx 0.9 には包みを外す口がある。**
-- ファイルの先頭が `-- no-transaction` なら、sqlx はこの SQL を包まない
-- （`sqlx-core-0.9.0/src/migrate/source.rs` の `sql.starts_with("-- no-transaction")` と
--  `sqlx-sqlite-0.9.0/src/migrate.rs` の `if migration.no_tx`）。
--
-- **したがって上の1行を消さないこと。** 消すと sqlx が包みに戻り、
-- 下の `BEGIN;` が「トランザクションの中のトランザクション」になって
-- **移行そのものが落ちる**（黙って壊れる方向ではないが、直し方は分からなくなる）。
--
-- 包みを外した代償として、**失敗したときの巻き戻しを sqlx に頼れない。**
-- 巻き戻しは下の明示 `BEGIN` / `COMMIT` が担うが、
-- **`_sqlx_migrations` への記録だけは自分のトランザクションの外**に落ちる
-- （sqlx は移行 SQL を流し終えてから別の文で記録する）。
-- 途中で落ちた場合に何が残るかは
-- `tests/store_schema.rs::a_migration_that_stops_leaves_the_database_where_it_was`
-- が実測で固定している——**データは1行も欠けず、版も進まないので、直して再実行できる。**
--
-- ============================================================================
-- 2. 作り直すのは `character` **1表だけ**
-- ============================================================================
--
-- 外部キーを切れるので、0003 の「親を落とすには子を全部落とすしかない」は成立しない。
-- `DROP TABLE character` は暗黙の DELETE FROM を伴うが、
-- **`PRAGMA foreign_keys = OFF` の下では ON DELETE CASCADE が動かない**ので
-- stat_alloc / session / memory / character_appearance の行はそのまま残る
-- （session 経由の message / summary / generated_image も同じ）。
--
-- **`sqlite_sequence`（ADR-0010 の採番）にも触れない。** session / message を
-- 落とさないので、`aoi@42` の 42 が巻き戻る余地が無い。
--
-- **`memory` を落とさないので、`superseded_by` の繋がりも自然に残る。**
-- 0003 が必要とした退避と復元、`PRAGMA defer_foreign_keys`、`sqlite_sequence` の
-- 写し取りは、**この形では1つも要らない。**
-- ついでに `src/store/memory.rs` の走査（絶対制約「時系列保持」の見張り番）を
-- 緩めずに済む。**代替があるなら見張り番は緩めない。**
--
-- ============================================================================
-- 3. 改名するのは「新しく作った表」の側だけ
-- ============================================================================
--
-- 「`character` を `character_old` へ改名してから新しい `character` を作る」書き方は
-- 一見自然だが、**改名すると他テーブルの `REFERENCES` 句が `character_old` へ
-- 書き換わる。** 書き換わってもエラーは出ず行も消えないので、
-- **その場では何も起きない。** 効いてくるのは後日「キャラクターを消したのに
-- 記憶が残っている」を誰かが見つけたときになる。
--
-- **`PRAGMA foreign_keys = OFF` だけでは止まらない。**
-- 実測（2026-08-16 / sqlite 3.53.4。`ALTER TABLE character RENAME TO character_old`
-- の後の `memory` の DDL）:
--
--   | foreign_keys | legacy_alter_table | memory の REFERENCES |
--   |---|---|---|
--   | ON  | ON  | "character_old"(id) へ書き換わる |
--   | ON  | OFF | "character_old"(id) へ書き換わる |
--   | OFF | ON  | character(id) のまま |
--   | OFF | OFF | **"character_old"(id) へ書き換わる** |
--
-- **止めるには両方が要る。** `foreign_keys` だけを振った2点観測は
-- 「OFF なら安全」と読めてしまうが、それは `legacy_alter_table = ON` が
-- 同時に立っていたからで、**この移行はどちらも立てない。**
--
-- そこで旧表の名前を最後まで動かさない（作る → 写す → 旧表を落とす → 新表を改名）。
-- 改名するのは**誰も参照していない新表**なので、書き換わる `REFERENCES` 句が存在しない。
-- **プラグマの組み合わせに寄りかからずに済むのが、この順序を選ぶ理由。**
-- 見張り番は
-- `tests/store_schema.rs::the_rebuild_leaves_the_other_table_definitions_untouched`
-- （旧表を先に改名する形へ書き換えると、実際にここが赤くなる。実測）。
--
-- なお `ALTER TABLE ... RENAME TO` を通った表の DDL は
-- `CREATE TABLE "character"` と**引用符付き**になる（SQLite が書き戻すときの整形）。
-- 表名そのものは変わらないので `sqlite_master.name` も `PRAGMA table_info` も従来どおりで、
-- DDL の字面を突き合わせているテストは無い（上のテストが併せて固定している）。
--
-- ============================================================================
-- 4. 規則を満たさない行があったら、黙って変換せず落ちる
-- ============================================================================
--
-- 識別子は不変（ADR-0016）なので **API では直せない。** 勝手に `Aoi` → `aoi` や
-- `" aoi "` → `"aoi"` と直すと、**FE 側やクライアント設定に残った `model` が
-- 別のものを指す**（あるいは何も指さなくなる）。ADR-0004 が最も避けたい壊れ方なので、
-- ここで止めて運用者に判断させる。
--
-- **CHECK 違反の素のエラーでは「どの行が悪いのか」が分からない**ため、
-- 先に明示的に落とす文を置く。SQLite で任意の文言を出せるのはトリガの RAISE だけなので、
-- 使い捨ての表とトリガを1組作って即座に捨てている。
--
-- **作業用の表とトリガはトランザクションの中で作ること。** 外に置くと、
-- RAISE で止まったときに表とトリガだけが確定して残り、
-- 再実行が `already exists` で詰む。

-- **外部キーを切る**（冒頭 1 / 公式 12 ステップの第 1 段）。
-- トランザクションの外でしか効かないので、`BEGIN` より前に置く。
PRAGMA foreign_keys = OFF;

-- 公式 12 ステップの第 2 段。**巻き戻しはここから COMMIT までが担う。**
BEGIN;

-- ---------------------------------------------------------------------------
-- 1. 事前検査。規則外の行が1件でもあれば、ここで移行全体を止める。
-- ---------------------------------------------------------------------------
CREATE TABLE handle_alphabet_gate_0005 (id INTEGER PRIMARY KEY);

CREATE TRIGGER handle_alphabet_gate_0005_check
BEFORE INSERT ON handle_alphabet_gate_0005
BEGIN
    SELECT RAISE(
        ABORT,
        'personacore 0005: character.name に新しい規則を満たさない行があります。'
        || '使えるのは ASCII の小文字英字・数字・ハイフン・アンダースコアだけです（ADR-0017）。'
        || '識別子は不変（ADR-0016）なので API では直せません。sqlite3 で '
        || 'SELECT id, name FROM character WHERE length(name) = 0 OR name GLOB ''*[^a-z0-9_-]*'' '
        || 'OR length(cast(name AS BLOB)) <> length(name); '
        || 'を実行して該当行を確認し、DB を直接編集してから再実行してください。'
    )
    WHERE EXISTS (
        SELECT 1 FROM character
        WHERE length(name) = 0
           OR name GLOB '*[^a-z0-9_-]*'
           -- **埋め込み NUL の逃げ道を塞ぐ。** SQLite の length() / GLOB は
           -- テキストを最初の NUL までしか見ないので、'aoi' || X'00' || 'AOI' は
           -- 上の2条件を素通りする。識別子は ASCII だけなので、
           -- バイト数と文字数が一致することを併せて要求すれば必ず捕まる。
           OR length(cast(name AS BLOB)) <> length(name)
    );
END;

INSERT INTO handle_alphabet_gate_0005 (id) VALUES (1);

DROP TRIGGER handle_alphabet_gate_0005_check;
DROP TABLE handle_alphabet_gate_0005;

-- ---------------------------------------------------------------------------
-- 2. 新しい形の表を作る（公式 12 ステップの第 4 段）。
--
--    **0004 までとの差は `character.name` の CHECK が1つ増えることだけ。**
--    列の並びは「識別子 → 呼び名 → 本文」に整理してある
--    （0004 が ALTER TABLE ADD COLUMN で display_name を末尾に足していた）。
-- ---------------------------------------------------------------------------
CREATE TABLE character_new_0005 (
    id INTEGER PRIMARY KEY,
    -- **新しい規則**（ADR-0017 / 作業単位 0004b-1）。
    -- 許すのは ASCII 小文字英字・数字・`-`・`_` だけ。**大文字も拒否する**
    -- （畳み込みは正規化であり、正規化がある限り「畳み込む前の行」が存在しうる）。
    --
    -- **アプリ側の写しは `CharacterName::ALLOWED_CHARACTERS` の1箇所だけ。**
    -- あちらは 38 文字の列挙、ここは範囲記法（`a-z0-9_-`）と、綴りをわざと変えてある。
    -- ずれの検出は tests/store_schema.rs::handle_alphabet_matches_the_schema_check が
    -- **両方向**（アプリが通す値を DB も通す／アプリが弾く値を DB も弾く）で見ている。
    --
    -- GLOB は LIKE と違い**大文字小文字を区別する**。LIKE では `Aoi` を弾けない。
    -- 否定クラスの中で `-` は末尾に置けば文字そのもの（範囲記号にならない）。
    --
    -- `length(cast(name AS BLOB)) = length(name)` は埋め込み NUL 対策
    -- （SQLite の length() / GLOB は最初の NUL までしか見ない）。
    -- 識別子は ASCII だけなのでバイト数と文字数は必ず一致する。
    --
    -- **下の2つの CHECK は、この規則にすべて含まれる**（`@` も空白も規則の外）。
    -- 冗長だが 0004b-1 では残す。正規化の機構ごと畳むのは 0004b-2 の作業単位で、
    -- そのとき `TRIMMABLE` の写しの台帳（src/domain/text.rs）と一緒に外すこと。
    name TEXT NOT NULL UNIQUE
        CHECK (
            length(name) > 0
            AND name NOT GLOB '*[^a-z0-9_-]*'
            AND length(cast(name AS BLOB)) = length(name)
        )
        CHECK (name NOT LIKE '%@%')
        CHECK (length(trim(
            name,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0),
    -- プロンプトに載る呼び名（ADR-0011）。**UNIQUE は付けない**（0004 と同じ）。
    -- 除去対象の8種は stat_alloc.axis の CHECK / 0002 の索引 /
    -- src/domain/text.rs の TRIMMABLE と同じ集合にする。
    display_name TEXT NOT NULL DEFAULT '(表示名未設定)'
        CHECK (length(trim(
            display_name,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0),
    base_persona TEXT NOT NULL,
    -- 台詞 / 地の文の形式指示。未設定ならアプリ側の既定値を使うため NULL 可。
    output_format_instruction TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- ---------------------------------------------------------------------------
-- 3. 写す（公式 12 ステップの第 5 段）。**列は明示する**（* に頼ると列順の
--    思い込みが混ざるうえ、0004 が末尾に足した display_name の位置に依存する）。
--
--    **`created_at` を落とさないこと。** 落としても DEFAULT が効くのでエラーにならず、
--    **静かに移行時刻へ**書き換わる。id も同じで、落とすと採番し直しになり
--    外に出た参照が別の行を指す。
-- ---------------------------------------------------------------------------
INSERT INTO character_new_0005
    (id, name, display_name, base_persona, output_format_instruction, created_at)
    SELECT id, name, display_name, base_persona, output_format_instruction, created_at
    FROM character;

-- ---------------------------------------------------------------------------
-- 4. 旧表を落とす（公式 12 ステップの第 6 段）。
--    **外部キーが切れているので、子テーブルの行は連鎖して消えない。**
--    切れていないと stat_alloc / session / memory / character_appearance が
--    ON DELETE CASCADE で消え、session 経由で message / summary / generated_image まで
--    連鎖する（**エラーは出ない**。実測 2026-08-16: memory 2 行 → 0 行）。
-- ---------------------------------------------------------------------------
DROP TABLE character;

-- ---------------------------------------------------------------------------
-- 5. 新表を `character` にする（公式 12 ステップの第 7 段）。
--    **改名するのは新しく作った表の側だけ**（冒頭 3）。
-- ---------------------------------------------------------------------------
ALTER TABLE character_new_0005 RENAME TO character;

-- ---------------------------------------------------------------------------
-- 6. 索引を作り直す（公式 12 ステップの第 8 段）。
--
--    正規化キーの UNIQUE 索引（0002 / ADR-0004）。**0004b-2 で役目を終える。**
--    `DROP TABLE` は索引も一緒に落とすので、ここで作り直さないと黙って消える
--    （tests/store_schema.rs::migration_creates_every_declared_index が見張っている）。
-- ---------------------------------------------------------------------------
CREATE UNIQUE INDEX character_name_key_idx ON character (
    trim(name, ' ' || char(9) || char(10) || char(11) || char(12) || char(13)
             || char(12288) || char(160)));

-- ---------------------------------------------------------------------------
-- 7. 外部キーの検査（公式 12 ステップの第 10 段）。
--
--    **ここまで外部キーは一切検査されていない。** 公式手順がこの段を要求しているのは
--    そのためで、落とすと**参照整合性が壊れた DB を、壊れたまま次の版へ運ぶ。**
--    `memory` の孤児は `recall_memories` が character_id で引く以上どの想起にも現れず、
--    **誰も読まないまま残り続ける。**
--
--    `PRAGMA foreign_key_check` は違反を**行として返すだけでエラーにならない**ので、
--    移行を止めるにはやはり RAISE が要る。**`pragma_foreign_key_check` 関数は
--    トリガの中では使えない**（`unsafe use of virtual table` / 実測 2026-08-16
--    sqlite 3.53.4）ため、`character` の子4テーブルを素の副問い合わせで見る。
--    この移行が触るのは `character` だけなので、壊れうるのはこの4本の参照だけ。
-- ---------------------------------------------------------------------------
CREATE TABLE foreign_key_gate_0005 (id INTEGER PRIMARY KEY);

CREATE TRIGGER foreign_key_gate_0005_check
BEFORE INSERT ON foreign_key_gate_0005
BEGIN
    SELECT RAISE(
        ABORT,
        'personacore 0005: character を参照している行のうち、親の居ない行があります。'
        || '移行は外部キーを切って行われるため、この状態は移行前から壊れていたことを意味します。'
        || 'sqlite3 で PRAGMA foreign_key_check; を実行して該当行を確認し、'
        || 'DB を直接編集してから再実行してください。'
    )
    WHERE EXISTS (SELECT 1 FROM stat_alloc
                  WHERE character_id NOT IN (SELECT id FROM character))
       OR EXISTS (SELECT 1 FROM character_appearance
                  WHERE character_id NOT IN (SELECT id FROM character))
       OR EXISTS (SELECT 1 FROM memory
                  WHERE character_id NOT IN (SELECT id FROM character))
       OR EXISTS (SELECT 1 FROM session
                  WHERE character_id NOT IN (SELECT id FROM character));
END;

INSERT INTO foreign_key_gate_0005 (id) VALUES (1);

DROP TRIGGER foreign_key_gate_0005_check;
DROP TABLE foreign_key_gate_0005;

-- 公式 12 ステップの第 11 段。
COMMIT;

-- 公式 12 ステップの第 12 段。**接続はプールへ返されて他の処理が引き当てる**ので、
-- 切ったままにしない（tests/store_schema.rs::foreign_keys_pragma_is_on_for_every_connection）。
PRAGMA foreign_keys = ON;
