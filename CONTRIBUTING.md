# 手を入れる人へ / Contributing

日本語の後に英語の版があります。 / English follows the Japanese.

personacore は個人が趣味で開発しています。Issue と PR は日本語でも英語でもかまいません。

## 始める前に

- **先に [CLAUDE.md](CLAUDE.md) の「絶対制約」を読んでください。** データモデル・セキュリティ・振る舞いについて、破ってはならない条件が並んでいます。
- 小さな直し（誤字・壊れたリンクなど）でなければ、**PR の前に Issue で相談してください。** 設計の理由は [docs/adr/](docs/adr/) にあり、その理由とぶつかる変更は受け入れられないことがあります。
- 脆弱性は Issue に書かず、[SECURITY.md](SECURITY.md) の方法で知らせてください。

## PR の流れ

1. このリポジトリを fork し、ブランチを作って変更します。
2. `main` へ PR を開きます。`main` は PR を通してしか変えられず、**マージには保守者（`marukei`）の承認が要ります。**
3. 新しい commit を push すると、それまでの承認は取り消されます。レビューの会話は、マージの前に解決します。

## テスト

開発は主に Windows 11 で行っています。PR を開く前に、次がすべて通ることを確かめてください。

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

**`cargo build` を省かないでください。** `cargo test` と `cargo clippy --all-targets` だけでは通ってしまうものがあります（理由は [CLAUDE.md](CLAUDE.md) の「コマンド」節）。

画面（`web/`）に触れたときは、次も回してください（詳しくは [web/README.md](web/README.md)）。

```bash
cd web
pnpm install
pnpm check
pnpm test
```

テストとセットでないコードの変更は受け入れません。ロジックは I/O を持たない純粋な関数に寄せてテストしてください。

## コミットの書き方

- 題名は `<type>(<scope>): <subject>` の形です（type は `feat` / `fix` / `docs` / `style` / `refactor` / `test` / `chore`）。
- **Issue の参照は、`#` と番号だけの形で書かないでください。**
  - 追跡するファイル（コード・コメント・文書）では、このリポジトリの Issue は `marukei/personacore#N`、非公開の開発リポジトリ（origin）の Issue は `origin#N` と書きます。`#` と番号だけの形は、`tests/issue_references.rs` が止めます。
  - コミットの本文で Issue を閉じるときは、`Closes marukei/personacore#N` と完全な形で書きます（このリポジトリのコミットは origin にも取り込むので、短い形では origin の同じ番号の Issue を閉じてしまいます）。PR の本文では `Closes #N` と書いてかまいません。
  - `origin#N` やコミットの番号には、このリポジトリからは辿れないものがあります（[README.md](README.md#untraceable-references)）。

## AI に関与させたとき

AI（コード生成のエージェントなど）が生成や改変に関与したときは、**その事実をコミットに記録してください。** このリポジトリでは、`Co-Authored-By:` のトレーラを使います（形は `Co-Authored-By: <モデルやツールの名前> <そのアドレス>`）。

これは、このリポジトリが採用している ACCRL v0.3 の協働の透明性に沿うためです（[LICENSE](LICENSE)・[NOTICE-AI-COLLABORATION.md](NOTICE-AI-COLLABORATION.md)）。ACCRL の誓約を行うのは人間で、AI は誓約者として署名しません。

## ライセンス

送ってもらった変更は、このリポジトリと同じ `MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3` の下で提供されるものとして扱います。PR を開くことで、これに同意したものとみなします。ACCRL の誓約を求めるものではありません。

---

## Before you start

personacore is a hobby project maintained by one person. Issues and pull requests are welcome in Japanese or English.

- **Read the "絶対制約" (absolute constraints) section of [CLAUDE.md](CLAUDE.md) first.** It lists the rules for the data model, security and behaviour that must not be broken. It is in Japanese.
- Unless the change is small (a typo, a broken link and so on), **please open an issue to discuss it before sending a pull request.** The reasons behind the design are in [docs/adr/](docs/adr/) (in Japanese), and a change that conflicts with them may not be accepted.
- Do not report vulnerabilities in issues; follow [SECURITY.md](SECURITY.md).

## Pull requests

1. Fork this repository, create a branch and make your change.
2. Open a pull request against `main`. `main` can only be changed through pull requests, and **merging needs the maintainer's (`marukei`'s) approval.**
3. Pushing new commits dismisses earlier approvals. Review conversations must be resolved before merging.

## Tests

Development happens mainly on Windows 11. Before opening a pull request, make sure all of these pass:

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

**Do not skip `cargo build`.** Some problems pass `cargo test` and `cargo clippy --all-targets` and are only caught by a plain build (the reason is in the "コマンド" section of [CLAUDE.md](CLAUDE.md)).

If you touch the screens (`web/`), also run the following (details in [web/README.md](web/README.md), in Japanese):

```bash
cd web
pnpm install
pnpm check
pnpm test
```

Code changes without tests will not be accepted. Put logic in pure functions without I/O, and test them.

## Commit messages

- The subject line has the form `<type>(<scope>): <subject>` (type is one of `feat` / `fix` / `docs` / `style` / `refactor` / `test` / `chore`).
- **Do not refer to issues with `#` and a number alone.**
  - In tracked files (code, comments, documents), write `marukei/personacore#N` for issues in this repository and `origin#N` for issues in the private development repository (origin). `tests/issue_references.rs` rejects the short `#` form.
  - To close an issue from a commit message, write the full form, `Closes marukei/personacore#N` (commits in this repository are also merged into origin, where the short form would close origin's issue with the same number). In a pull request description, `Closes #N` is fine.
  - Some `origin#N` references and commit ids cannot be followed from this repository ([README.en.md](README.en.md#untraceable-references)).

## When AI was involved

If AI (such as a coding agent) took part in generating or changing your contribution, **record that fact in the commit.** This repository uses the `Co-Authored-By:` trailer (in the form `Co-Authored-By: <name of the model or tool> <its address>`).

This follows the collaboration transparency of ACCRL v0.3, which this repository has adopted ([LICENSE](LICENSE), [NOTICE-AI-COLLABORATION.md](NOTICE-AI-COLLABORATION.md)). The ACCRL pledge is made by humans; AI does not sign as a pledger.

## License

Contributions are provided under the same license as this repository, `MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3`. By opening a pull request, you agree to this. You are not asked to make the ACCRL pledge.
