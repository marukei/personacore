## 何を変えたか / What changed

<!-- 閉じる Issue があれば `Closes #N` と書く（PR の本文ではこの形でよい）。/ If this closes an issue, write `Closes #N` (this short form is fine in a PR description). -->

## なぜ / Why

## 確かめたこと / How it was checked

- [ ] `cargo build`
- [ ] `cargo test`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `web/` に触れたとき / if `web/` changed: `pnpm check` と / and `pnpm test`

## 確かめ忘れやすいこと / Easy to miss

- [ ] [CLAUDE.md](https://github.com/marukei/personacore/blob/main/CLAUDE.md) の「絶対制約」に反していない / does not break the absolute constraints in CLAUDE.md
- [ ] 適用済みのマイグレーションの SQL を編集していない / does not edit an applied migration
- [ ] コミットの本文の閉じるキーワードは完全な形（`Closes marukei/personacore#N`）/ closing keywords in commit messages use the full form
- [ ] AI が関与したコミットに `Co-Authored-By:` を付けた / commits AI took part in carry a `Co-Authored-By:` trailer
