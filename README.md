# personacore

English: [README.en.md](README.en.md) · 繁體中文: [README.zh-TW.md](README.zh-TW.md)

ローカルの推論サーバに、**キャラクターと記憶**を持たせるための中間サーバ。

FE と推論サーバの間に立ち、ペルソナ・性格・記憶・直近の履歴からプロンプトを組み立て、
返ってきた本文を台詞と地の文に分け、会話から記憶を抽出して SQLite に貯める。
**両側で OpenAI 互換の API を話す**ので、FE は既存のチャットクライアントでもよい。

```
FE（この repo の画面 / 既存の OpenAI 互換クライアント）
    ↓ POST /v1/chat/completions
personacore（Rust / axum）  ←→  SQLite
    ↓ POST /v1/chat/completions
上流の推論サーバ
```

**推論そのものは作らない。** 作るのはプロンプトと記憶の管理だけ
（1ターンで何が起きるかは [docs/02-architecture.md](docs/02-architecture.md)）。

---

## このファイルの読み手

**リポジトリを開いた人。** 動かしたい人と、手を入れたい人の両方を想定している。
ここに書くのは**どこを読めばよいか**までで、なぜそう決めたかは書かない。

| やりたいこと | 行き先 |
|---|---|
| **動かす** | 下の「動かす」 |
| **API から使う**（English） | [docs/api/README.md](docs/api/README.md) — API guide in English: headless start, the `model` field, chat clients, admin routes, versioning |
| **設計を知る** | [docs/README.md](docs/README.md)（索引）。まず [docs/01-overview.md](docs/01-overview.md) |
| **どこまで出来ているか** | 出した版は GitHub の Releases、開いている作業単位は GitHub の Issue。順番とその理由は [docs/06-roadmap.md](docs/06-roadmap.md)。ここには写さない |
| **手を入れる** | [CLAUDE.md](CLAUDE.md) の**絶対制約**。読まずに書かない |

**開発の過程の記録**（3ロール体制で進めた経緯・運用の教訓・引き継ぎ。AI 協働の記録）は
[docs/process/](docs/process/) に分けて置いてある（案内は [docs/README.md](docs/README.md) の「開発の過程の記録（AI 協働の記録）」の区分）。
誰が何を決め、誰が何を書いたか（人間と AI の分担・使ったモデル）は
[NOTICE-AI-COLLABORATION.md](NOTICE-AI-COLLABORATION.md)（AI 協働の開示）にまとめてある。
使うためには、どちらも読まなくてよい。

<a id="untraceable-references"></a>
**公開側から辿れない参照。** docs・コードのコメント・過程の記録には、`origin#N` の形の Issue の番号、
コミットの番号（7 桁ほどの 16 進の綴り）、v0.1.x のリリースが出てくる。どれも、公開する前の非公開の元のリポジトリ
（origin）を指していて、公開側のこのリポジトリからは辿れない。公開側の Issue の番号は1から振り直され、
履歴は1つのコミットから始まる（[ADR-0048](docs/adr/0048-open-source-policy.md) §2）。

---

## 動かす

```powershell
pwsh ./scripts/start.ps1
```

これ1本で、画面のビルド → 上流の疎通確認 → DB の用意 → 起動、まで進む。
止めるのは **Ctrl+C**。

待ち受けが開くと、**personacore 自身が**画面の URL（`http://127.0.0.1:<ポート>/`）を
既定のブラウザで開こうとする（作業単位 F-4c。スクリプトはもう開かない）。

> **⚠ ブラウザが実際に開くことは、誰も確かめていない**
> （[scripts/README-start.md](scripts/README-start.md) の「見張り番」節）。
> **開き方は OS ごとに違う**（Windows は `cmd /C start`、Mac は `open`、その他は `xdg-open`）。
> 開かせない指定もある（`-NoBrowser` / 実行ファイルなら `--no-browser`）。
> 画面そのものを配らずに API だけで使うなら `--no-ui`（ブラウザも開かない / [ADR-0047](docs/adr/0047-the-api-is-the-product.md)）。
> **URL はどちらにせよ表示される**ので、開かなければ自分で開けばよい。

要るもの:

| | どこを見るか |
|---|---|
| Rust | [rust-toolchain.toml](rust-toolchain.toml) が指すツールチェーン |
| pnpm | 画面のビルドに使う（[web/README.md](web/README.md)） |
| **PowerShell 7** 以上 | 起動スクリプトが `#Requires` で要求する |
| cargo-about | **配布物を作るときだけ**（`scripts/package.ps1` が第三者の著作権表示を作る）。`cargo install --locked cargo-about --features cli`（確かめた版 0.9.2） |
| 上流の推論サーバ | 下の「上流」。**無くても起動はする** |

`scripts/start.ps1` の引数でよく使うのは `-Port` と `-SkipWebBuild`
（順に「別のポートで立てる」「画面を作り直さない」）。**残りの引数・DB の置き場所・
上流が落ちているときの振る舞いは [scripts/README-start.md](scripts/README-start.md)**（ここに写さない）。

待ち受けのポートは、**環境変数 `PERSONACORE_PORT` → 画面の「設定」で保存した番号 → 既定**の順で決まる
（保存した番号は次の起動から効く。`-Port` は `PERSONACORE_PORT` に入る）。

> **既定のポートは写していない。** 値は `src/server/config.rs` の `DEFAULT_PORT` にある。
> markdown は Rust のソースを読めないので、**写す代わりに書かない**ことで揃えてある。

> **⚠ 走らせて確かめたのは、この機（Windows 11 / PowerShell 7）だけ。**
> **他の OS・他の機で動かしたことは一度も無い**
> （[docs/adr/0036-the-range-of-machines-is-a-constraint.md](docs/adr/0036-the-range-of-machines-is-a-constraint.md)）。
> **環境が満たすべきことと、実際に歩いた1通りは
> [docs/09-setup.md](docs/09-setup.md)**（要求と歩いた道を分けて書いてある）。

### 配る（実行ファイル1本にする）

```powershell
pwsh ./scripts/package.ps1
```

**`dist/personacore.exe` が出来る。これ1本を配れば動く**
——**動かす機に Rust も pnpm も要らない**（画面は実行ファイルに埋め込まれている）。
**なぜそうしたのか・何を諦めたのかは
[docs/adr/0041-one-executable-per-os.md](docs/adr/0041-one-executable-per-os.md)** が持つ。

* **DB は OS の作法の場所**に出来る（Windows なら `%LOCALAPPDATA%` の下）。
  **`PERSONACORE_DB_PATH` で変えられる。**
* **まっさらから遊び直すなら `--fresh`。** **消さずに退避する**ので、手で戻せる。
* **⚠ 画面を直したら、実行ファイルを組み直すこと。** 埋め込みはコンパイル時に決まる。
  **起動ログの `ui_source` / `ui_origin` / `ui_files` がそれを言う。**

> **⚠ Windows でしか歩いていない。** Mac 版の作り方は決めていない
> （同じ ADR の §決定5）。**歩いた1通りは [docs/09-setup.md](docs/09-setup.md)。**

### 最初の画面

**キャラクターが1件も無い状態から始まる。**
「キャラクター」→「新規作成」で作る。まとめて投入するなら
[scripts/seed-characters.ps1](scripts/seed-characters.ps1)。

---

## 上流（推論サーバ）

上流の面（本文と裏方）は、それぞれ次の2つの形のどちらかで指す
（[docs/adr/0042-the-executable-owns-the-upstream-it-starts.md](docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）。

- **起こす面** —— 上流プロファイル（`upstreams.json`）で gguf を書いた面。
  personacore が `llama serve` を起こし、**終了時に自分が起こした子だけを止める。**
- **繋ぐ面** —— gguf を書いていない面。**既に動いているサーバ**（LM Studio・中継など）へ繋ぐだけで、
  起こしも止めもしない。繋ぐ先のサーバはあとから起動してよく、personacore を起動し直す必要は無い。

**頼まれない限り、personacore は上流を起こさない。** GPU を占有し続ける行為なので、
起こすかどうかは利用者が決める（[docs/04-environment.md](docs/04-environment.md)）。
頼む形は、起動時の `--upstream <名前>`、画面でのプロファイルの切り替え（`POST /admin/upstream/profile`）、
起こすのを待っている面（`not_started`）の「起こす」ボタンの3つである。
**旗を渡さずに起動すると、前回選んだプロファイルを読むが、gguf を書いた面は起こさない**
（[docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md](docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md)）。

**プロファイルの書き方・置き場所・起こす前に見ることは
[scripts/README-start.md](scripts/README-start.md) が持つ。**
上流が鍵（API キー）を求めるときの画面の手順は
[scripts/README-dist.md](scripts/README-dist.md) の「推論サーバが鍵（API キー）を求めるとき」が、書き方と扱いの決定は
[docs/adr/0056-a-key-for-the-upstream.md](docs/adr/0056-a-key-for-the-upstream.md) が持つ（どれもここに写さない）。
**⚠ `upstreams.json` には鍵が平文で残る。人に渡すときは、各面の `api_key`（と `allow_key_over_plain_http`）を消してから渡すこと。**

要求しているのは「特定の製品であること」ではなく、**能力の束**である
——必須のものと、あれば使うもの。**その一覧は
[docs/adr/0038-what-personacore-requires-of-an-upstream.md](docs/adr/0038-what-personacore-requires-of-an-upstream.md)
の §決定1 が持つ**（ここに写さない。**件数も書かない** ——
写した数は、増えても減っても黙って古びる）。

プロファイルを使わない起動では、宛先の既定は `src/llm/lmstudio.rs` の `DEFAULT_ENDPOINT`（LM Studio の既定の待ち受け）。
別の宛先に居るなら `PERSONACORE_LLM_ENDPOINT` を立ててから起動する（プロファイルとぶつかったら、プロファイルが勝つ）。

裏方（記憶の抽出・性格の変容）だけを別の上流へ向けるなら
`PERSONACORE_SUB_LLM_ENDPOINT`。**未設定なら本文と同じ上流へ行く。**
**理由と、諦めたものは
[docs/adr/0039-two-upstreams-for-two-jobs.md](docs/adr/0039-two-upstreams-for-two-jobs.md)
が持つ**（ここに写さない）。

> **初期の開発と測定は LM Studio に対して行った。** 2026-09-17 からは、
> **`llama serve`（llama.cpp）を本文と抽出の両方の上流にした会話も通している**
> （`C-18` / `C-19`。3つのモデルで 34 ターンずつ。**どれも成立し、差は癖**）。
> **どこまで歩いたかの権威は [docs/09-setup.md](docs/09-setup.md) の §歩いた道と §歩いていない道**
> （ここに写さない）。

---

## 手を入れる

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

**`cargo build` を省かないこと。** 理由は [CLAUDE.md](CLAUDE.md) の「コマンド」節にある
（`cargo test` と `cargo clippy --all-targets` だけでは通ってしまうものがある）。
画面側の型検査とテストは [web/README.md](web/README.md)。

**先に読むもの: [CLAUDE.md](CLAUDE.md) の「絶対制約」。**
データモデル・セキュリティ・振る舞いについて、破ってはならない条件が並んでいる
（背景は [docs/03-data-model.md](docs/03-data-model.md) と
[docs/05-development.md](docs/05-development.md)）。

PR の流れ・コミットの書き方・AI に関与させたときの記録は [CONTRIBUTING.md](CONTRIBUTING.md)、
脆弱性の報告は [SECURITY.md](SECURITY.md)（公開の Issue には書かない）。

### どこに何があるか

| | |
|---|---|
| `src/` | 本体。`domain` / `prompt` / `parse` は I/O を持たない純粋関数 |
| `tests/` | 統合テスト |
| `migrations/` | SQLite のスキーマ。**適用済みの SQL は編集しない**（新しい番号を足す） |
| `web/` | 画面（Svelte）→ [web/README.md](web/README.md) |
| `scripts/` | 起動と投入 → [scripts/README-start.md](scripts/README-start.md) |
| `docs/` | 設計と ADR → [docs/README.md](docs/README.md) |

---

## これは何ではないか

- **推論エンジンではない。** 推論は上流に委ねる。
- **公開サーバではない。** ループバックにのみ待ち受け、認証は持たない。外部へ出さない。
- **完成品ではない。** 何が動くかは GitHub の Releases、何が未着手かは GitHub の Issue（順番の理由は [docs/06-roadmap.md](docs/06-roadmap.md)）。

---

## ライセンス

`MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3` —— 次の3つから、使う人が1つを選べる。

- MIT License（[LICENSE-MIT](LICENSE-MIT)）
- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- AI Collective Creativity Respect License (ACCRL) v0.3-PROTOTYPE（[LICENSE-ACCRL.md](LICENSE-ACCRL.md)、英訳は [LICENSE-ACCRL.en.md](LICENSE-ACCRL.en.md)）

MIT か Apache-2.0 を選べば、OSI 承認のライセンスの下で personacore を自分のソフトウェアに組み込める。
ACCRL の採用の表明と誓約は [LICENSE](LICENSE) にある。

---

## この README の見張り番

**⚠ 「書いてあることが実体と食い違ったら鳴る」ではない。**
[tests/readme.rs](tests/readme.rs) が見ているのは、**下に挙げたものだけ**である。

- リンクの行き先が実在すること
- 名指しした `.ps1` が実在すること
- 名指しした引数を、起動スクリプトが宣言し、しかも使っていること
- 名指しした環境変数を、サーバが実際に読むこと
- `cargo` の並びが [CLAUDE.md](CLAUDE.md) の要求と一致すること
- 既定値を写さず在処を指し、その先が実体と一致すること
- 名乗る PowerShell の版が、起動スクリプトの要求と一致すること
- 名指しする宛先がループバックだけであること
- 起動スクリプトが、実際に画面をビルドしてサーバを起動する形であること

**手順の順番・散文の主張・「書いてあるとおりに動くこと」は、どれも見ていない。**
実際、**初版は手順の順番を実体と取り違えており、鳴ったのは人の目だった**（F-2 のレビュー）。

**⚠ 何を守っていないかは、[tests/readme.rs](tests/readme.rs) の頭に全部書いてある。**
