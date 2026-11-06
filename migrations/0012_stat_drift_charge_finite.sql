-- 累積の列に「有限であること」の CHECK を足す
-- （ADR-0027 §決定7 / 作業単位 16e-b のレビュー指摘3）。
--
-- ============================================================================
-- 設計はここに書かない
-- ============================================================================
--
-- stat_drift_charge が何のための表かは docs/03-data-model.md が正。
-- ここに残すのは「この SQL を読むときにだけ要る注意」だけにする。
--
-- ============================================================================
-- なぜ表を作り直すのか
-- ============================================================================
--
-- SQLite は ALTER TABLE で CHECK を足せない。列の制約を変える唯一の方法が
-- 「新しい表を作って写して入れ替える」である。
--
-- 入れ替えが安全なのは、この表を参照している表が1つも無いから
-- （参照は character への片道だけ）。索引も明示的には持っていない
-- （主キー (character_id, axis) の自動索引だけ）。
--
-- ============================================================================
-- ±Inf の行は写さない（**捨てる**）
-- ============================================================================
--
-- typeof(charge) = 'real' は ±Inf を通してしまう。sqlite3 から 9e999 を書けば
-- その行が作れ、入った瞬間にそのキャラクターの値の適用が永久に止まる
-- （Store::drift_charges が丸ごと InvalidStoredValue になり、
--  累積には削除の導線が無い）。
--
-- 写さないと、その軸の累積は 0 から始め直しになる。溜まっていた端数は失われるが、
-- **有限でない累積に意味のある端数は入っていない**（そこへ至る正規の経路が無い）。
-- 止まったままにするより、0 から続けられる方を選ぶ。
--
-- NaN は入りようがない。SQLite に NaN の表現は無く、束縛すると NULL になり、
-- NOT NULL で弾かれる。
--
-- ============================================================================
-- 長さの CHECK は今回も置かない（0011 の判断を引き継ぐ）
-- ============================================================================
--
-- ここへ来る軸名は stat_alloc から読み出して StatAxis::parse を通った値なので、
-- stat_alloc が許す形より狭い制約を置くと**正当な行だけが静かに落ちる。**
--
-- ============================================================================
-- 走査に引っかかる名前を綴らない
-- ============================================================================
--
-- 記憶の SQL 走査と票の SQL 走査は migrations/** をテーブル名の文字列で走る。
-- どちらの名前もこのファイルには現れない。0009 / 0010 / 0011 と同じ注意。
CREATE TABLE stat_drift_charge_next (
    character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    axis TEXT NOT NULL CHECK (
        typeof(axis) = 'text'
        AND instr(axis, char(0)) = 0
        AND length(trim(
            axis,
            ' ' || char(9) || char(10) || char(11) || char(12) || char(13) || char(12288) || char(160)
        )) > 0
    ),
    charge REAL NOT NULL DEFAULT 0.0 CHECK (
        typeof(charge) = 'real'
        AND charge BETWEEN -1e308 AND 1e308
    ),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (character_id, axis)
);

INSERT INTO stat_drift_charge_next (character_id, axis, charge, updated_at)
SELECT character_id, axis, charge, updated_at
  FROM stat_drift_charge
 WHERE charge BETWEEN -1e308 AND 1e308;

DROP TABLE stat_drift_charge;

ALTER TABLE stat_drift_charge_next RENAME TO stat_drift_charge;
