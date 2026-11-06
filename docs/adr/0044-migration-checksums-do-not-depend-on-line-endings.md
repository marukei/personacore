# ADR-0044: マイグレーションのチェックサムを改行コードから切り離す。既存の DB は起動時に帳簿を付け替える

- 状態: **採用**（2026-09-19 / 利用者が「origin#22 をやりつつ、勝ちの線までの動きを考えよう」と決めた。塞ぎ方の選択は司令塔）
- 日付: 2026-09-19
- 関連: [ADR-0041](0041-one-executable-per-os.md)（**配布物は exe 1本、DB は `%LOCALAPPDATA%`** —— この ADR が無いとその前提が崩れる）/
  [05-development.md](../05-development.md)「マイグレーションを触るときの罠」（**編集しなくても同じことが起きる経路**をここで足す）/
  CLAUDE.md §4「隔離ワークツリーで全緑でも `main` で緑とは限らない —— 改行が違う」（同じ族。今度はテストではなく DB が割れた）/
  Issue origin#22（`B-53`）

---

## 背景 —— **SQL を1バイトも編集していないのに「modified」で止まった**

`sqlx::migrate!` は各 SQL を `include_str!` で埋め込み、**バイト列そのものの SHA-384** をチェックサムにする
（`sqlx-core 0.9.0` の `migrate/migration.rs`: `Sha384::digest(sql)`。改行の正規化は無い —— 司令塔がソースで確認）。

このリポジトリは **`.gitattributes` が無く、`core.autocrlf = true`**（CLAUDE.md §4 の実測）。索引は全部 LF だが、**作業ツリーの改行は checkout の仕方で決まる**:

| checkout | `migrations/*.sql` の改行 |
|---|---|
| `<repo>`（ツールが書き直したファイルだけ LF） | **混在**（`0001`〜`0004` と `0013` が CRLF、`0005`〜`0012` が LF。2026-09-19 に数えた） |
| `git worktree add` / `git clone`（Windows） | 全部 CRLF |
| Mac / Linux の clone | 全部 LF |

**実測（2026-09-19 / 司令塔）**: F-4b-1 のワークツリー（新しい checkout）の exe が作った DB を、`main` の作業ツリーの exe で開いたら

```
migration 5 was previously applied but has been modified
```

**同じリポジトリの同じコミットから組んだ2つの exe が、同じ DB を共有できない。**

**これは配布の前提を崩す。** ADR-0041 は「exe 1本、DB は `%LOCALAPPDATA%`」と決めた。**exe を組み直す checkout が変わった瞬間に、利用者の DB が開けなくなる**（clone し直し、別の機、Mac）。`--fresh` は退避であって救済ではない。

**CLAUDE.md は「適用済みのマイグレーション SQL は編集しない」を守ってきた。編集しなくても同じことが起きる経路が在った。**

---

## 決定

### 1. **`.gitattributes` で `migrations/*.sql` を LF に固定する**

```
migrations/*.sql text eol=lf
```

索引は既に LF なので、**どの checkout でも LF になる**（Windows の autocrlf より属性が勝つ）。
**`main` の作業ツリーは混在のままなので、属性を足したあと `git checkout -- migrations/`（または `git add --renormalize`）で揃える。** 索引の内容は変わらない。

**⚠ 対象は `migrations/*.sql` だけ。** リポジトリ全体を LF にする決定は別（CLAUDE.md §4 の CRLF の知見が全部それに依存しているので、ここで動かさない）。

### 2. **見張り番: `migrations/*.sql` に CR が1バイトも無いこと**

属性が消えると、Windows の新しい checkout で赤になる。**実ファイルを読む本体と、植えた文字列の対照**（`15b` の作法）。

### 3. **既存の DB は、起動時に帳簿を付け替えてから `migrate` を走らせる**

決定1だけでは**いま在る DB が壊れる** —— 利用者の `data/personacore.sqlite` と `%LOCALAPPDATA%` の DB は**混在の checkout のチェックサム**で作られている。LF に揃えると `0001`〜`0004` と `0013` の値が変わり、同じエラーになる。

**`Store::migrate` の前に1段置く:**

1. `_sqlx_migrations` を読む（無ければ何もしない —— 新しい DB）。
2. 埋め込んだ各マイグレーション（LF 版）について、**「同じ SQL の改行違い」のチェックサム**を計算する —— **CRLF 版**（`\n` → `\r\n`）。
3. 帳簿の `checksum` が **CRLF 版に一致し、LF 版に一致しない**行だけ、**LF 版へ `UPDATE`** する。**それ以外の行は触らない**（本当に編集された SQL は、従来どおり `migrate` が止める）。
4. 付け替えた行は `info` で「マイグレーション N の帳簿を改行違いから付け替えました」と言う。

**⚠ `UPDATE` するのは sqlx の帳簿（`_sqlx_migrations.checksum`）だけ。** `memory` の行を DELETE / UPDATE しない絶対制約とは別の表で、**利用者のデータには1バイトも触らない。**
**⚠ 「改行違い」以外の変異を通さない。** 付け替えの条件は「CRLF 版に一致」だけで、**編集された SQL は今までどおり止まる。**

**混在のファイル（1ファイルの中に LF と CRLF が両方ある）は考えない** —— この家の checkout はファイル単位で揃うので、そういう帳簿は作られていない。**当たったら `migrate` が止めるので、黙って通ることは無い。**

### 4. **[05-development.md](../05-development.md) の「罠」に改行の項を足す**

「編集しなくても、checkout の改行が違えば同じエラーになる。属性が固定し、帳簿の付け替えが既存 DB を救う」。

---

## 却下した選択肢

| 案 | なぜ採らない |
|---|---|
| **改行を正規化してから checksum を取る自前の `Migrator`**（`sqlx::migrate!` を捨てる） | `build.rs` の追跡と `migrate!` の型が一体。**付け替え1段のほうが小さく、sqlx の帳簿の意味も変えない** |
| **属性だけ足して、既存 DB は `--fresh` で** | 利用者の会話・記憶が退避先に残るだけで読めなくなる。**「壊れていないのに読めない」を作る側** |
| **リポジトリ全体を `eol=lf`** | CLAUDE.md §4 の CRLF に関する知見が丸ごと前提を失う。**この ADR の範囲ではない。別に決める** |
| **チェックサムの照合を緩める（sqlx の `ignore_missing` の類）** | sqlx にその向きの選択肢は無く、あっても**編集された SQL まで通す**。**通すのは改行違いだけ** |

---

## 結果

- **どの checkout から組んだ exe でも、同じ DB を開ける。** 配布の前提（ADR-0041）が保たれる。
- **いま在る DB は、次に起動した exe が帳簿を付け替えて、そのまま開く。** 利用者の操作は要らない。
- **失うもの: `_sqlx_migrations` を personacore が1度 `UPDATE` する。** sqlx の帳簿を「読むだけ」ではなくなる。
  **付け替えの条件は狭く固定**（CRLF 版に一致する行だけ）。

## 見張り番（`B-53` が置くもの）

- **`migrations/*.sql` に CR が無い**（実ファイル ＋ 植えた対照）。
- **CRLF 版の帳簿を持つ DB を開ける** —— テストが `migrate` 済みの DB を作り、`_sqlx_migrations.checksum` を CRLF 版に書き換えてから `Store::connect` → 開ける／帳簿が LF 版に戻っている／`info` の行が出る。
- **本当に編集された SQL は今までどおり止まる** —— 帳簿を**第3の値**（どちらの版でもない）にした DB は `migrate` が止める（付け替えの門が緩んでいない対照）。**この対照が無いと、「常に付け替える」実装が緑を通る。**
- **新しい DB では帳簿を読まない**（`_sqlx_migrations` が無い状態で `Store::connect` が通る）。
- **帰属**: 付け替えるのは一致した行だけ（3行のうち1行だけ CRLF 版にした DB で、その1行だけが変わる）。
