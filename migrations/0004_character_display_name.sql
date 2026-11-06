-- character に display_name（プロンプトに載る呼び名）を足す（ADR-0011 / 作業単位 0004a）。
--
-- name は3つの役割を兼ねていた: プロンプトの呼び名 / `model` で指す識別子 / 一意性のキー。
-- 3つ目のせいで**同じ呼び名で別ペルソナの個体を持てない**。回避しようと `葵-実験A` と
-- 名付けると1つ目が汚れ、比べたいのはペルソナなのに名前という別の変数が混ざる。
-- そこで呼び名だけを別の列へ切り出す。**name の改名（→ handle）は 0004b。**
--
-- ============================================================================
-- テーブルを作り直さない
-- ============================================================================
--
-- character は stat_alloc / session / memory / character_appearance の4テーブルから
-- 参照されており（うち session は message → summary / generated_image を連れてくる）、
-- 0003 で分かった「外部キーを切れないので親を落とすには子を全部落とすしかない」を
-- 適用すると**スキーマ全体の再構築**になる。列を足すだけなら ALTER TABLE で足りる。
--
-- ============================================================================
-- なぜ DEFAULT が '' ではないのか（実測 2026-08-14）
-- ============================================================================
--
-- SQLite の ALTER TABLE ADD COLUMN は CHECK 制約を**付けられる**が、
-- **DEFAULT 値がその CHECK を満たしていなければならない。**
-- 満たさないと ADD COLUMN 自体が `CHECK constraint failed` で失敗する。
--
-- 実測（sqlx 0.9 / SQLite 3.4x）:
--
--   * 既存行あり + `NOT NULL DEFAULT '' CHECK(非空)` → **失敗**
--   * 既存行なし + `NOT NULL DEFAULT '' CHECK(非空)` → **成功**
--   * 既存行あり + `NOT NULL DEFAULT '(表示名未設定)' CHECK(非空)` → 成功
--
-- 2番目が最も危険で、**空の開発用 DB では通り、行のある本番 DB でだけ落ちる**
-- （しかも落ちた DB は Store::connect ごと起動不能になる）。
-- したがって DEFAULT には CHECK を満たす値を置くしかない。
--
-- NOT NULL には DEFAULT が要る（SQLite の ADD COLUMN の制限）ので、
-- 「DEFAULT を持たない NOT NULL」は選べない。
--
-- **この既定値は下の UPDATE によって既存行から即座に消える。** 残るのは
-- 「display_name を指定しない INSERT」だけで、これはアプリ経由では起こらない
-- （store::create_character は必ずバインドする）。DB を直接編集したときに
-- この値が現れたら、それは「表示名を入れ忘れた行」の目印として読むこと。
-- 空の呼び名がプロンプトに載るより、目に見える既定値が載る方が調査できる。
--
-- 除去対象の8種は character.name の CHECK / stat_alloc.axis の CHECK /
-- 0002 の character_name_key_idx / src/domain/text.rs の TRIMMABLE と**同じ集合**にする
-- （ドリフト検出は tests/store_schema.rs が実際の INSERT で確認している）。
--
-- **UNIQUE は付けない。** 一意なのは識別子（name）だけで、
-- `aoi` と `aoi-v2` が**どちらもプロンプト上は「葵」**になれることがこの列の存在理由。
--
-- 長さの上限はスキーマでは縛らない（name と同じ。上限は domain::DisplayName が持つ）。
ALTER TABLE character ADD COLUMN display_name TEXT NOT NULL DEFAULT '(表示名未設定)'
    CHECK (length(trim(
        display_name,
        ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
    )) > 0);

-- 既存行を移行前の呼び名で埋める。**この1文が移行の本体。**
-- 落とすと、既存のキャラクターは表示名が上の既定値のまま残り、
-- **プロンプトに載る呼び名が黙って別物になる**（更新 API が無いので直せない）。
-- name は NOT NULL かつ非空の CHECK 付きなので、この UPDATE が CHECK に触れることはない。
UPDATE character SET display_name = name;
