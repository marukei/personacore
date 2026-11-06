-- キャラクター名の正規化キーに UNIQUE 索引を張る（ADR-0004）。
--
-- `character.name` 自体の UNIQUE では `"aoi"` と `" aoi "` の併存を防げない。
-- 併存すると、アプリ層が `model` を trim してから照合した瞬間に**両方へ一致**し、
-- `LIMIT 1` の順序次第で**どちらのキャラに書き込むかが不定**になる。
-- アプリ層は `domain::CharacterName` で正規化済みだが、DB を直接編集された場合の担保として
-- スキーマ側でも正規化後の値で一意にする。
--
-- 除去対象の8種は `character.name` の CHECK 制約および `src/domain/text.rs` の
-- `TRIMMABLE` と**同じ集合**にすること（半角 / TAB / LF / VT / FF / CR / 全角空白 / NBSP）。
-- 一致は tests/store_schema.rs のドリフト検出テストが実際の INSERT で確認している。
--
-- **この索引が防ぐのは「衝突」であって「ゴースト行」ではない。**
-- `"aoi"` が既にある状態での `" aoi "` の直接 INSERT は防げるが、`" aoi "` 単独の行は
-- 依然として作れる。単独のゴースト行は `store` 側で一覧から除外し warn ログに残す。
--
-- `lower()` は挟まない。SQLite の `lower` は ASCII しか畳まないため日本語名では効かず、
-- 大文字小文字の扱いが言語によって割れる方が害が大きい。
CREATE UNIQUE INDEX character_name_key_idx ON character (
    trim(name, ' ' || char(9) || char(10) || char(11) || char(12) || char(13)
             || char(12288) || char(160)));
