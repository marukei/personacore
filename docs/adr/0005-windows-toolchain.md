# ADR-0005: Windows のツールチェーン競合への対処

- 状態: **解決済み**（2026-08-13 に GNU 版をアンインストールし、恒久対処が完了）
- 日付: 2026-08-13（提起・暫定対処） / 2026-08-13（恒久対処）

> **本 ADR は歴史的記録。** 現在このプロジェクトは rustup の
> `stable-x86_64-pc-windows-msvc` 一本で、素の `cargo` が使える。
> 暫定対処として置いていた `scripts/rust-env.ps1` は削除済み。
> 同種の症状が再発したときの切り分け材料として残す。

## 背景

主機（Windows）でプロジェクトを初期化し依存スタックをビルドしたところ、次で落ちた。

```
error: error calling dlltool 'dlltool.exe': program not found
error: could not compile `windows-sys` (lib) due to 1 previous error
```

調査の結果、Rust が**2つ**インストールされていた。

| 実体 | PATH スコープ | ホスト |
|---|---|---|
| `C:\Program Files\Rust stable GNU 1.97\bin` | **Machine** | `x86_64-pc-windows-gnu` |
| `%USERPROFILE%\.cargo\bin`（rustup） | User | `x86_64-pc-windows-msvc` |

Windows は Machine PATH を User PATH より先に評価するため、素の `cargo` / `rustc` は
**常に GNU 版**に解決される。GNU 版は raw-dylib の処理に mingw の `dlltool.exe` を
必要とするが、mingw binutils が未導入のため落ちる。

紛らわしい点として、`rustup show` は `stable-x86_64-pc-windows-msvc (active, default)` と
表示する。**rustup の設定は正しく、PATH だけが壊れている。**

## 決定

### 暫定（実施済み）

**GNU 版を PATH から外してから** cargo を呼ぶ。3行のスクリプト `scripts/rust-env.ps1` を用意し、
**同一コマンド内でドットソース**して使う。

```powershell
. .\scripts\rust-env.ps1; cargo build
```

この方式で依存スタック（axum / sqlx+SQLite / reqwest+rustls / tokio）の
`cargo clean` からの**クリーンビルド・clippy・test がすべて通ることを確認済み**
（build 約37秒 / clippy 約30秒）。MSVC ビルドツールは導入済みで、
SQLite のネイティブビルドも問題ない。

加えて `.claude/settings.local.json`（git 管理外）に GNU 版を除いた `PATH` を設定した。
**Claude Code の次回セッションからは**素の `cargo` が通る。現セッションには反映されない。

`CLAUDE.md`・`.claude/agents/rust-implementer.md` にこのコマンド形式を明記した。

### 却下した暫定案 — `rustup run stable cargo ...`

**効かない。** 調査中に一度これを採用しかけたが誤りだった。記録として残す。

`rustup run` は toolchain の bin を PATH の**末尾**に足すため、
Machine PATH の GNU 版が先に解決されたままになる。`rustup run` 内での実際の解決順:

```
C:\Program Files\Rust stable GNU 1.97\bin\rustc.exe      ← これが選ばれる
%USERPROFILE%\.cargo\bin\rustc.exe
%USERPROFILE%\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\rustc.exe
```

`rustup run stable rustc -vV` は `host: x86_64-pc-windows-msvc` を返すため紛らわしいが、
これは rustup が rustc を**直接 exec している**ためで、
`cargo` から `rustc` への**入れ子の解決には効かない**。

**この誤りに気づくのが遅れた原因**: `rustup run stable cargo build` が「Finished in 0.43s」と
返したのを成功と判断したが、実際には直前の手動ビルドの成果物が残っていて
**何もコンパイルしていなかった**。`cargo clean` 後に検証すべきだった。
以後、ツールチェーン周りの検証は必ず `cargo clean` から行うこと。

### 恒久（**実施済み** 2026-08-13）

GNU 版 Rust をアンインストールした。

```powershell
winget uninstall Rustlang.Rust.GNU
```

**`cargo` は消えない。** rustup 側（`%USERPROFILE%\.cargo\bin`）が
`cargo` / `rustc` / `rustfmt` / `clippy-driver` / `rust-analyzer` を同名で提供しており、
消えるのは重複していた GNU 版のみ。

実施後の確認結果:

- `rustc -vV` → `host: x86_64-pc-windows-msvc`
- Machine PATH の `C:\Program Files\Rust stable GNU 1.97\bin` は MSI が自動除去
- `cargo clean` からの**素の** `cargo test` で 31 テスト全緑、
  `clippy --all-targets -- -D warnings` 警告0、`fmt --check` 通過

これに伴い `scripts/rust-env.ps1` と `.claude/settings.local.json` の PATH 上書きを削除し、
`CLAUDE.md` / [04-environment.md](../04-environment.md) を更新した。

### 却下した恒久案

- **Machine PATH から GNU 版のディレクトリだけを除去する** — インストールが残って休眠するため、
  将来 PATH を触ったときに再発しうる。アンインストールの方が状態が単純。
- **mingw binutils を導入して GNU 版を直す** — GNU ターゲットで動かす積極的な理由がない。

## 理由

- **暫定対処を先に入れたのは、恒久対処が管理者権限を要する環境変更であり、
  勝手に実施すべきでないため。** ユーザーの判断を待つ間も作業を止めない。
- `rust-toolchain.toml` による解決は**できない**。あれは rustup シム経由でしか効かず、
  GNU 版の cargo が起動した場合は**黙って無視される**。対策として成立しない。

### 却下した選択肢

- **mingw binutils を導入して GNU 版を直す** — GNU ターゲットで動かす積極的な理由がない。
  Windows では MSVC ターゲットが標準であり、そちらは既に動く。
- **`.cargo/config.toml` に `[build] rustc = "絶対パス"` を書く** —
  マシン固有の絶対パスを git 管理下のファイルに書くことになり、macOS 側で壊れる。
  加えて `cargo clippy` は `clippy-driver` を PATH から別途引くため、
  `build.rustc` を差し替えても clippy は GNU 版のまま壊れる。

## 結果

### 得られたもの

- **素の `cargo` が使える。** 回避策のスクリプトも PATH 上書きも不要になった
- 症状（`dlltool.exe not found`）と原因の対応が記録され、再発時に即座に切り分けられる

### 再発時の切り分け

```bash
rustc -vV
```

`host: x86_64-pc-windows-msvc` 以外が出たら、rustup 以外の Rust が PATH にいる。
**`rustup show` は rustup 自身の設定を表示するだけなので、この判別には使えない。**

- **ツールチェーンを切り替えた後は `cargo clean` してから再ビルドする。**
  古いターゲットの成果物が残っていると `E0461`（crate メタデータ不整合）が出る。
  実際に本件の調査中に遭遇した。
- **ツールチェーン周りの検証は必ず `cargo clean` から行う。**
  「Finished in 0.4s」は「直っている」ではなく「何もしていない」ことが多い。
  本件で実際にこれを誤読し、誤った暫定対処を一度採用してしまった。
