-- character に返答の長さの目安（下限と上限の字数）を足す
-- （ADR-0065 §決定1 / 作業単位 A-22a = marukei/personacore#71）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- 「何のための欄か」「プロンプトのどこに何が載るか」は ADR-0065 と docs/03-data-model.md が正。
-- 適用済みのマイグレーションは二度と編集できない（sqlx のチェックサム）ので、
-- ここには「この SQL を読むときにだけ要る注意」だけを残す。
--
-- ============================================================================
-- DEFAULT を置かない（既存の行は全部「未設定」から始まる）
-- ============================================================================
--
-- 2列とも NULL ＝未設定。この移行を当てても、いまの DB の振る舞いは1つも変わらない
-- （未設定のキャラクターのプロンプトは1バイトも変わらない / ADR-0065 §決定1）。
--
-- ============================================================================
-- CHECK は「両方 NULL」か「両方が整数で 1 ≤ 下限 ≤ 上限 ≤ 9999」
-- ============================================================================
--
-- 1組の条件は2列目（response_length_max）の CHECK に置く。1列目を足す時点では2列目がまだ無いので、
-- 1列目の CHECK は自分の範囲だけを縛る。SQLite の CHECK はどの列に付けても行全体の条件として働き、
-- UPDATE で片方の列だけを書いても、両方の列を参照する CHECK は評価される。
-- 実測（2026-11-15 / sqlite3 3.54.1 / 既存行2件の表に同じ2文を当てた）:
--
--   既存行 → 2件とも NULL（typeof = null）
--   受理: (200, 400) / (1, 1) / (9999, 9999) / ('200', '400') と (3.0, 4.0)（型アフィニティで整数に直って入る）
--   拒否: (0, 5) / (5, 10000) / (400, 200) / (NULL, 5) / (5, NULL) / (2.5, 4) / (X'01', 4) / ('abc', 5)
--   拒否: 両方 NULL の行に下限だけ・上限だけを書く UPDATE
--
-- 9999 の綴りの正は domain::ResponseLength::HIGHEST。**ここは移行した時点の写し**で、
-- 上げるときは新しい移行で CHECK も上げる（この文は編集できない）。
--
-- **CHECK は外せる**（PRAGMA ignore_check_constraints）ので、読み出し側でも
-- domain::ResponseLength::parse を通し、通らない行は一覧で指し示す
-- （domain::CharacterColumn::ResponseLengthMin / ResponseLengthMax。1組が読めなければ両方）。

ALTER TABLE character ADD COLUMN response_length_min INTEGER
    CHECK (response_length_min IS NULL
           OR (typeof(response_length_min) = 'integer' AND response_length_min BETWEEN 1 AND 9999));

ALTER TABLE character ADD COLUMN response_length_max INTEGER
    CHECK ((response_length_min IS NULL AND response_length_max IS NULL)
           OR (typeof(response_length_min) = 'integer'
               AND typeof(response_length_max) = 'integer'
               AND 1 <= response_length_min
               AND response_length_min <= response_length_max
               AND response_length_max <= 9999));
