# 09. 各環境のセットアップ

**この文書は2つを別々に書く** —— **要求**（どの環境でも満たさねばならないこと）と、
**歩いた道**（実際に確かめた1通り）。

**⚠ 分けているのは、根の README が実際に2件間違えたから。**
**手順の順番が実体と逆**で、**ブラウザが開くと断定していた**（作業単位 F-2 のレビュー）。
どちらも**書いた人がその道を歩いていない**ことから出ている。
**歩いていない道は、手順として書かない。要求として書く。**

**名乗りの作法は [ADR-0033](adr/0033-measured-constants-name-what-they-were-measured-on.md)**
——測った（歩いた）ものは、**何の上で測ったのか**を一緒に名乗る。

---

## この文書の立て方

| 節 | 書くもの | 書かないもの |
|---|---|---|
| [要求](#要求--環境が満たすこと) | 環境が満たすこと。**満たさないと何が起きるか**まで | 手順 |
| [歩いていない道](#歩いていない道) | **手順を持っていない環境**について、分かっていることだけ | **手順**（歩いていないので書けない） |
| [歩いた道](#歩いた道) | **実際に確かめた1通り**。**どの機・どのモデルで歩いたか**を名乗る | 一般化 |

**節の順番には意味がある。** 「歩いていない道」を「歩いた道」より**前**に置いてあるのは、
**手順を探しに来た読者に、手順の無い環境の話を先に読ませるため**である。
**見張り番がこの順番に寄りかかっていて、順番そのものも見張っている**
（[tests/setup_doc.rs](../tests/setup_doc.rs) の
`the_untrodden_section_comes_before_the_walked_section`）。

---

## 要求 —— 環境が満たすこと

### 上流（推論サーバ）

**personacore が要求するのは「特定の製品であること」ではなく、能力の束である。**

一覧は [ADR-0038](adr/0038-what-personacore-requires-of-an-upstream.md) の §「1. **上流に要求する能力を、3層に分けて名前を付ける**」が持つ。
**必須の層と、あれば使う層と、それぞれ満たせないとどうなるかまで、あちらの表にある。**

> **ここには写さない。件数も書かない。**
> 写した表は、片方を直したときに黙って古びる（[CLAUDE.md](../CLAUDE.md)）。
> **見張り番が鳴るのは、あちらの表のセルが**逐語で**こちらに現れたときだけ。**
> **言い換えた写しは鳴らない**（下の「見ていないこと」）。**塞いだのではなく、一番出やすい形を落としただけ。**

**上流の面は2つの形のどちらか** —— 上流プロファイルで gguf を書いた面は personacore が
`llama serve` を起こし（**起こす面**）、書いていない面は既に動いているサーバへ繋ぐだけ（**繋ぐ面**）
（[ADR-0042](adr/0042-the-executable-owns-the-upstream-it-starts.md)）。
**personacore は頼まれない限り起こさない。** GPU を占有し続ける行為なので、起こすかどうかは
利用者が決める（[04-environment.md](04-environment.md) / 頼む形は [scripts/README-start.md](../scripts/README-start.md)）。
**繋ぐ面の上流はあとから起動してよく、personacore を起動し直す必要は無い。**

プロファイルを使わない起動では、宛先の既定は `src/llm/lmstudio.rs` の `DEFAULT_ENDPOINT`。
別の宛先に居るなら `PERSONACORE_LLM_ENDPOINT` を立ててから起動する。

**上流は2つ指せる。** 裏方（記憶の抽出・性格の変容）だけを別の上流へ向けるなら
`PERSONACORE_SUB_LLM_ENDPOINT` を立てる。**未設定なら本文と同じ上流へ行く。**
**なぜ2つ持てるようにしたのか・何を諦めたのかは
[ADR-0039](adr/0039-two-upstreams-for-two-jobs.md) が持つ**（ここに写さない）。

### 手元（personacore を走らせる機）

| 要るもの | 満たさないとどうなるか | 権威 |
|---|---|---|
| **Rust ツールチェーン** | ビルドできない | [rust-toolchain.toml](../rust-toolchain.toml)（版はここが持つ。写さない） |
| **pnpm** | 画面を作れない。**サーバは起動するが UI が出ない**（起動ログが `ui_files=0` と言う） | [web/README.md](../web/README.md) |
| **PowerShell 7 以上** | [scripts/start.ps1](../scripts/start.ps1) が動かない（`#Requires` が弾く）。**手で立てるなら要らない** | [scripts/README-start.md](../scripts/README-start.md) |
| **cargo-about**（**配布物を作るときだけ**） | [scripts/package.ps1](../scripts/package.ps1) が**画面とリリースビルドの前に止まる**（第三者の著作権表示を作れない）。`cargo install --locked cargo-about --features cli`（確かめた版 0.9.2）。**走らせるだけなら要らない** | [scripts/notices.ps1](../scripts/notices.ps1) |
| **ループバックに待ち受けられること** | 起動できない。**サーバは `127.0.0.1` にしか待ち受けない**（外部へ出さない） | [CLAUDE.md](../CLAUDE.md) の絶対制約 |
| **待ち受けポートが空いていること** | [scripts/start.ps1](../scripts/start.ps1) がそう言って**何もせず止まる**。`-Port` で変えられる | [scripts/README-start.md](../scripts/README-start.md) |
| **SQLite のファイルを置ける場所** | DB を開けずに落ちる。**未設定なら OS の作法の場所**、決められない環境は**起動を止める** | [ADR-0041](adr/0041-one-executable-per-os.md) §決定3 |

**GPU は要らない。** 推論は上流が持つ。

既定のポートは写していない —— 値は `src/server/config.rs` の `DEFAULT_PORT` にある。
**待ち受けの URL は、起動ログと [scripts/start.ps1](../scripts/start.ps1) の `[画面]` 行が出す。**

### 速い環境を要求してよい。遅い環境で壊れてはいけない

方針は [ADR-0036](adr/0036-the-range-of-machines-is-a-constraint.md)。
**遅い機で触るつまみと、触るのがどういうときかは
[04-environment.md](04-environment.md) の「別の機で動かすときに触るつまみ」が持つ**
（ここに写さない）。

---

## 歩いていない道

**⚠ この節には手順を書かない。歩いていないから。**
書いてあるのは「**要求の側から言えること**」と「**確かめていないと分かっていること**」だけである。

* **Windows 以外の OS** —— **一度も走らせたことが無い**
  （[ADR-0036](adr/0036-the-range-of-machines-is-a-constraint.md)）。
  [rust-toolchain.toml](../rust-toolchain.toml) はホストの三つ組を書いていないので
  ツールチェーンの側は移せる想定だが、**確かめていない。**
  起動スクリプトが要求するのは PowerShell 7 であって Windows ではない
  ——が、**pwsh の入った別の OS で走らせた者は居ない。**
  スクリプトを使わず手で立てる形（[scripts/README-start.md](../scripts/README-start.md) の最後の節）**も同じ。**
* **上流が LM Studio 以外** —— ~~**通しで動かしたことがまだ無い。**~~
  **半分は歩いた**（2026-09-17 に訂正）。**`llama serve`（llama.cpp）を抽出の上流にした走行は、
  測定 `C-12` / `C-16` / `C-17` で延べ 12 走ある**（[ADR-0037](adr/0037-extraction-quality-is-a-bet-on-the-model.md) の C-12 / C-17 の節）。
  **上の §上流の表は、`llama serve` に対して満たされている** —— 一覧・実効コンテキスト長・`reasoning_effort` の抑止（作業単位 `G-3b` / `G-7` / `G-10`）。
  ~~**⚠ 歩いていないのは本文側である。**~~ **→ 歩いた**（2026-09-17 / `C-18` と `C-19`）。
  **`llama serve` を本文と抽出の両方の上流にして、3つのモデル（ministral / 公式 Qwen / 無検閲 Qwen）で 34 ターンずつ会話を通した。**
  **どれも成立する。差は癖**（地の文の有無・Markdown・引用符）で、数は [06-roadmap.md](06-roadmap.md) の `C-19` の行と
  `<measurements>\c19-2026-09-17\RESULT.md`。**⚠ 上流の画面が「答えた名前」を名乗れるようになったのは `B-45` から**（それ以前は頼んだ名前しか出ない）。
  **⚠ `llama serve` は 1 プロセス 1 モデルで、`model` の欄を見ない**（`G-1`）——
  **載っているモデルがそのまま使われる。** 本文と抽出を別のモデルにするなら `PERSONACORE_SUB_LLM_ENDPOINT`
  （[ADR-0039](adr/0039-two-upstreams-for-two-jobs.md)）。
* **上流が別の機に居る形** —— `PERSONACORE_LLM_ENDPOINT` は URL を受けるので
  **書けはする**が、**そうやって動かしたことは無い。**
  **personacore 自身の待ち受けはループバックに限る**ので、これは「上流だけが遠い」形になる。
* **VRAM に載らない機・上流を他と取り合う機** —— **時間の値が外れる。**
  つまみと理由は [04-environment.md](04-environment.md)。**この機では踏んでいない。**
* ~~**実行ファイル単体で配る形** —— **未着手**~~ **→ 歩いた**（2026-09-18 / 作業単位 F-4a /
  [ADR-0041](adr/0041-one-executable-per-os.md)）。**下の §歩いた道 の最後の節**が持つ。
  **⚠ 歩いたのは Windows だけ。** Mac 版の作り方は決めていない（ADR-0041 §決定5）。
* ~~**上流を personacore 自身に起こさせる形** —— **本物の `llama` では歩いていない。**~~
  **→ 歩いた**（2026-09-25 / 下の §歩いた道 の「6.」。走ったままの差し替えは origin#2 の実機確認で本物の `llama` を使った）。
  **⚠ 歩いていないのは、公式の配布物（winget の `ggml.llamacpp` / Releases の zip / llama.app）で入れた `llama` で起こす形**
  —— この機の `llama` は `serve` を持つ一体型の CLI だが、入手元が分からない。
  **以下の段落は F-4b-1 の時点の記録として残す**（「差し替えはまだ無い」は origin#2 で事実でなくなった）。
  上流を personacore 自身に起こさせる形（`--upstream <名前>` /
  [ADR-0042](adr/0042-the-executable-owns-the-upstream-it-starts.md) / 作業単位 F-4b-1）
  —— 当時は本物の `llama` では歩いていなかった。
  **エージェントは GPU を掴む経路を走らせない**（[CLAUDE.md](../CLAUDE.md) §4）ので、
  確かめてあるのは**「偽の `llama`」を `PATH` の先頭に置いた走行だけ**
  （`tests/upstream_launch.rs` / `tests/upstream_cli.rs`）——
  起こす・載ったと認める・失敗と認める・**自分が控えた子だけ**止める、までは通っている。
  **確かめていないのは、本物へ渡している引数が正しいことと、本物がモデルを載せること。**
  要るものは2つだけ（実行ファイルと、DB と同じディレクトリの `upstreams.json`）で、
  **書き方と読み方は [scripts/README-start.md](../scripts/README-start.md) が持つ。**
  **⚠ 置き場所は `PERSONACORE_UPSTREAMS` で動かせる。**
  **走ったままの差し替えはまだ無い**（ADR-0042 §決定4 / 作業単位 `F-4b-2`）。

---

## 歩いた道

**この1通りだけが、実際に歩いた道である。** 何の上で歩いたかを先に名乗る。

| | |
|---|---|
| **日付** | 2026-09-03（JST） |
| **リビジョン** | `09539db` |
| **OS** | Windows 11 Pro（10.0.26200） |
| **GPU** | GeForce RTX 4070（VRAM 12GB） |
| **rustc** | 1.97.1（host `x86_64-pc-windows-msvc`） |
| **pnpm** | 9.7.0 |
| **Node** | v26.7.0 |
| **PowerShell** | 7.6.5 |
| **上流** | LM Studio（この回の既定の宛先。起動済み。歩いた時点の記録） |
| **モデル** | `ministral-3-14b-instruct-2512`（本文・サブとも既定のまま） |

**⚠ 何も無い状態から確かめるため、新しく `git clone` したツリーで歩いた。**

### 1. clone してビルドし、テストを通す

```powershell
git clone <このリポジトリ> t
cd t
cargo build
cargo test --no-fail-fast
```

* `cargo build` は 56.44s で `Finished`。**この回、`warning:` の行は 0 だった**
  （`2>&1` で stderr ごと捕まえて数えている）。
  **⚠ これを「警告は出ない」と読まないこと。**
  [04-environment.md](04-environment.md) は
  **「MSVC の linker の案内で、bin をリンクするたびに必ず1件出る」**と書き、
  **同じ機でもサブエージェントからは 0 件と報告され続けている**（**原因は未特定**）とも書いている。
  **F-3 でも割れた** ——歩いた側は**3通りの測り方**
  （clone / `cargo clean -p personacore` からの再ビルド / `src/main.rs` を触っての再リンク）
  **で3回とも 0**、**レビュアーは同じ機の複製で `generated 1 warning` を観測した。**
  **つまりこれは「この機ではこうなる」ではなく「誰が測るとこうなる」の話である。**
  **件数を完了条件にしないこと**——見るのは中身、というあちらの結論をそのまま引き継ぐ。
* `cargo test --no-fail-fast` は **1839 passed / 0 failed / 5 ignored**、**終了コード 0**
  （**上の「リビジョン」の時点の数**。この文書の見張り番はまだ入っていない）。
* **⚠ clone 直後の作業ツリーは全ファイルが CRLF になる**（`core.autocrlf` が立っていて
  `.gitattributes` が無い / [CLAUDE.md](../CLAUDE.md)）。**その CRLF のツリーで全緑だった。**
  隔離ワークツリー（LF）でだけ緑になる形が実際に出ているので、ここを通すこと自体に意味がある。

### 2. 画面をビルドする

```powershell
cd web
pnpm install --frozen-lockfile
pnpm run build
cd ..
```

* `pnpm install --frozen-lockfile` は `Done in 1.8s`。
  **⚠ この機は pnpm の store が既に温まっている。** 初めての機では、ここはもっと掛かる。
* `pnpm run build` は 135 modules を変換して `built in 510ms`。`web/dist` が出来る。

### 3. 起動する

```powershell
pwsh ./scripts/start.ps1 -NoBrowser
```

起動ログが言ったこと（**値そのものはログが出す。ここには写さない**）:

* `[FE ]` が画面を作り直し、`[上流]` が**繋がったと言った**（上流は起動してある）。
* `[DB  ]` が clone したツリーの下の `data/` を指し、そこに新しい DB が出来た。
* `web_dir_exists=true` —— 手順 2 の `web/dist` が見えている。
  **⚠ その欄はいまは無い**（F-4a で `ui_source` / `ui_origin` / `ui_files` へ移った /
  [ADR-0041](adr/0041-one-executable-per-os.md) §決定2）。**観測した事実はそのまま残してある。**
* **モデルがまだ載っていないので、コンテキスト長を聞けず、本文の予算が落とし所へ落ちた**
  （起動ログの `context_tokens`。[archive/04-environment-lm-studio.md](archive/04-environment-lm-studio.md) と
  [scripts/README-start.md](../scripts/README-start.md) が予告しているとおり）。
* 待ち受けを開始し、`[画面]` 行が URL を出した。

### 4. 上流へ1本通す

```powershell
pwsh ./scripts/seed-characters.ps1
```

```bash
curl -X POST http://127.0.0.1:<ポート>/v1/chat/completions -H "Content-Type: application/json" --data-binary "@body.json"
```

* 投入は `[作成] aoi` / `[作成] rin` と、それぞれの性格ステータス 4 軸。
  **`GET /v1/models` が2件を返した。**
* **⚠ 本文はファイルに置いて `--data-binary @` で渡した。**
  この機の Git Bash では `curl -d` にインラインの日本語を渡すと壊れる
  （[04-environment.md](04-environment.md)）。
* **返ってきたのは `200` と、台詞と地の文の混じった本文**（`finish_reason` は `stop`）。
  レスポンスの `model` は `aoi@1` ——`<識別子>@<セッション>` の形
  （[ADR-0004](adr/0004-session-identification.md)）。
* サーバのログは `応答を分解しました outcome=structured fell_back=false`、
  続けて非同期の抽出が `記憶を抽出しました extracted=5 saved=5 discarded=0`。
* **投入から応答まで 14 秒ほど。** **モデルの JIT ロードを含む初回**なので、2回目以降とは別物である。

### この回に確かめていないもの

* **ブラウザが実際に開くこと。** `-NoBrowser` で走らせた
  （エージェントはブラウザを起動しない / [CLAUDE.md](../CLAUDE.md)）。**F-1 でも未確認のまま。**
* **Ctrl+C での停止。** この回は別のプロセスから止めた。
  **DB が閉じられることは F-1 が確かめている**（[scripts/README-start.md](../scripts/README-start.md)）。
* **ストリーミング**（`stream: true`）。通したのは非ストリーミングの1本だけ。
* **画面からの操作。** 配られていることは `web_dir_exists=true` までしか見ていない。
  **⚠ その欄はいまは無い**（F-4a で `ui_source` / `ui_origin` / `ui_files` へ移った /
  [ADR-0041](adr/0041-one-executable-per-os.md) §決定2）。**観測した事実はそのまま残してある。**
* **`-Port` / `-DbPath` / `-SkipWebBuild` を付けた形。** 素の起動だけを歩いた
  （引数ごとの確認は F-1 が済ませている）。

### 5. 配布物を作って、別の場所から起動する（**別の回**）

**⚠ ここから下は 2026-09-18 に歩いた別の回である**（作業単位 F-4a /
[ADR-0041](adr/0041-one-executable-per-os.md)）。
**上の名乗りとの違いは3つだけ**——**日付が 2026-09-18**、
**リビジョンは `b1ba6c1` に F-4a の未コミットの差分を重ねた作業ツリー**、
**PowerShell は 7.6.6**（上の表は 7.6.5。どちらも 7 系）。
**機・GPU・rustc・pnpm・Node は上の表のまま。**
**⚠ 上流は起動していない**——**この回に上流は要らない**（画面と API の形だけを見る）。

```powershell
pwsh ./scripts/package.ps1
```

* `[FE ]` が `pnpm run build`、続けて `[EXE]` が `cargo build --release`。
  **順番はスクリプトが持っている**（画面はコンパイル時に埋め込まれるので、
  逆だと古い画面が入る / [ADR-0041](adr/0041-one-executable-per-os.md) §決定5）。
* 出来たのは **`dist/personacore.exe`（12.3 MB）**、**終了コード 0。**
* **⚠ `cargo build --release` は `warning: linker stdout: …` を1件出した**
  （MSVC の linker の案内）。**件数を完了条件にしないこと**は手順 1 と同じ。

**出来た1本を別のディレクトリへ複製して、そこから起動した**
（`%TEMP%` の下。リポジトリの外で、隣に何も置いていない）:

```text
PERSONACORE_DB_PATH=<一時ディレクトリ>\db\personacore.sqlite
PERSONACORE_PORT=<空いているポート>
```

起動ログが言ったこと:

* `ui_source=embedded` / `ui_origin=web/dist` / **`ui_files=3`**
  ——**画面は実行ファイルの中から来ている**（横にフォルダは置いていない）。
* `PERSONACORE_DB_PATH` の指す**まだ無かったディレクトリを作って**、そこに DB が出来た
  （`personacore.sqlite` と `-wal` / `-shm`）。
* 上流へは繋がらず、`context_tokens` が落とし所へ落ちた。**起動は止まらない。**

`curl` で見たのは2つ:

```text
$ curl -s -i http://127.0.0.1:<ポート>/
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
content-length: 405

<!doctype html>
<html lang="ja">
  <head>
    <meta charset="utf-8" />
    <title>personacore</title>
    <script type="module" crossorigin src="/assets/index-LOD3ORCB.js"></script>
    <link rel="stylesheet" crossorigin href="/assets/index-B3isNEIj.css">
  </head>
  <body><div id="app"></div></body>
</html>

$ curl -s -i http://127.0.0.1:<ポート>/v1/nothing
HTTP/1.1 404 Not Found
content-type: application/json
content-length: 112

{"error":{"message":"指定されたエンドポイントは存在しません","type":"invalid_request_error"}}
```

**`/` は埋め込みの `index.html`、`/v1` の未知パスは OpenAI 形式のエラー。**
**配り方の規則は ADR-0031 §4 のまま変わっていない。**

**`PERSONACORE_DB_PATH` を外して起動すると**、
`%LOCALAPPDATA%\personacore\personacore.sqlite` を指した
（[ADR-0041](adr/0041-one-executable-per-os.md) §決定3。値はログが出す）。

**初期化も同じ実行ファイルで歩いた:**

* `--Fresh`（綴り違い）は**起動せずに終了コード 1**。
  `知らない引数です: --Fresh` まで出る。**黙って無視しない。**
* `--fresh` を付けると、DB を開く前に3つとも改名された
  ——`personacore.sqlite.bak-<時刻>` と、同じ接尾辞の `-wal` / `-shm`。
  **元の3つは消えておらず、新しい DB がその隣に出来た。**

### この回に確かめていないもの

* **Windows 以外。** Mac の作り方は決めていない（ADR-0041 §決定5）。
* **画面からの操作。** 見たのは `/` が `index.html` を返すところまでで、
  **ブラウザで開いてはいない**（エージェントはブラウザを起動しない / [CLAUDE.md](../CLAUDE.md)）。
* **上流を繋いだ会話。** この回は上流を起動していない。
* **`PERSONACORE_WEB_DIR` を立てた形**（ディスクから配る腕）。
  **実行ファイルからは歩いていない**（テストは通っている）。
* **場所を決められない環境で止まること。** `%LOCALAPPDATA%` が無い環境を作れていない。
* **`-SkipWebBuild` / `-OutDir` を付けた形。** 素の作り方だけを歩いた。

### 6. zip を展開して、利用者向け README のとおりに会話まで（**別の回**）

**⚠ ここから下は 2026-09-25 に歩いた別の回である**（Issue origin#25 / 作業単位 F-4d）。
**上の名乗りとの違い**——**日付が 2026-09-25**、**リビジョンは `fb865e9`**（zip の名前と中の exe の `--version` がこれを名乗る）、
**PowerShell は 7.6.6**、**上流は personacore が自分で起こした `llama serve`**（この機の `llama` は `0.3.0-dev (build 10679)`。入手元は不明）、
**モデルは `Ministral-3-14B-Instruct-2512-Q4_K_M.gguf`**。**機・GPU・rustc は上の表のまま**（この回は pnpm と Node を使わない —— 配布物だけを使う）。

```powershell
pwsh ./scripts/package.ps1
```

* 終了コード 0。**`dist\personacore-0.1.0-fb865e9-windows-x64.zip`（5.6 MB）**が出来た。
  展開すると中身は `personacore.exe` と `README.md` の**2つだけ**。

**展開したフォルダで、zip の中の README の手順1・3・4を上から歩いた**（手順2の `hf download` は歩いていない。この機には gguf が既にある）。
保存先は `LOCALAPPDATA` を一時ディレクトリへ向け、**初めて起こす利用者と同じ状態**にした:

* **手順1**: `where.exe llama` が場所を出した。
* **手順3**: `.\personacore.exe` —— ログが見本を置いた場所と `url="http://127.0.0.1:<ポート>/"`（引用符付き）を出し、保存先に DB と `upstreams.json` の見本が出来た。
* **手順4**: 画面の「上流モデル」→ 見本の `ministral-llama` の「直す」→ gguf の欄に **Windows の絶対パス**（`C:\Users\…\*.gguf`）を書いて「保存する」→「このプロファイルに切り替える」
  → 上の表が「載せています（…から）」から「準備完了（上流が名乗った名前: Ministral-3-14B-Instruct-2512-Q4_K_M.gguf）」に変わった。
* **会話**: 「キャラクター」→「新規作成」で1人作り、「会話」で話しかけて**返事が返った（1往復）**。

**同じ日に、利用者が自分の端末と本物の保存先（`%LOCALAPPDATA%\personacore\`）で同じ道を歩き、Ctrl+C で personacore と、それが起こした `llama` の両方が止まることを確かめた。**

### この回に確かめていないもの

* **別の機。** 利用者の手元に、ローカル LLM が動く Windows の機は他に無い（2026-09-25）。**この機でしか歩いていない。**
* **公式の配布物で入れた llama.cpp。** 上の §歩いていない道 のとおり。
* **`hf download`**（README の手順2）。
* **署名の無い exe に SmartScreen の警告が出るか。** この機ではビルドした本人なので出ない。
* **Mac。** Mac の zip の作り方は決めていない（ADR-0041 §決定5）。

---

## この文書が見ていないこと

**⚠ 「書いてあることが実体と食い違ったら鳴る」ではない。**
[tests/setup_doc.rs](../tests/setup_doc.rs) が見ているのは、**下に挙げたものの裏返しだけ**である。

- **手順を1つも実行していない。** 綴りの突き合わせしかしていない。
- **要求の表の右列（満たさないとどうなるか）を、実際に外して確かめていない。**
  例外は「モデルが載っていないと予算が落とし所へ落ちる」で、これは歩いた道で見た。
- **上流の3層は、指し先が実在することと、あちらの表が**逐語で**写っていないことしか見ていない。**
  **⚠ 自分の言葉で言い換えた写しは止まらない**（変異検査で実測。全緑を通った）。
  **表の形は §上流（推論サーバ）の中にだけ置かせない。**
  **⚠ 見出し1つ上（§要求 の直下）に置けば、言い換えた表でも通る**（レビュアーが実測）。
  **散文の言い換えは、どこに置いても通る。人が読むしかない。**
  **中身が正しいかは ADR の側の話**で、ここは見ない。
- **名乗りは、10 欄のうち 2 欄しか実体と突き合わせていない。**
  **`pnpm`**（`web/package.json` の `packageManager`）と
  **`PowerShell`**（`scripts/start.ps1` の `#Requires`）**だけ**が突き合わせ済みで、
  **残り8欄は「埋まっている」しか見ていない** ——`rustc` の版を書き換えても鳴らない。
  **⚠ 初版は「突き合わせる相手が無いから直せない」と 10 欄全部について書いていた。誤りだった。**
  **`PowerShell` はメジャーまで**（`#Requires` が言うのは下限なので、`7.0.0` に書き換えても鳴らない）。
- **歩いた道の順番は、コードブロックに書いた命令の並びしか見ていない。**
  **その順で実際に通ることは見ていない**（これが F-2 で間違えた形そのものである）。
- **「歩いていない道」に手順が無いことは、コードブロックの有無と、
  歩いた命令の綴りでしか見ていない。**
  **⚠ 初版はコードブロックしか見ておらず、散文で手順を書く形が素通りしていた**（レビュアーが実測）。
  **いまは歩いた命令の綴りも禁じてある**が、**その一覧に無い綴りで書けば、まだ素通りする。**
- **リンクは行き先が在ることしか見ていない。** 見出しの錨（`#…`）は落として読むので、
  **節が消えても鳴らない。**
- **`cargo` の並びが [CLAUDE.md](../CLAUDE.md) の要求と一致することは見ていない**
  （歩いた道は `--no-fail-fast` 付きで回しており、根の README とは並びが違う）。
- **床と帳簿そのものは無防備。** この一覧の件数の床も、名乗りの欄の帳簿も、
  **下げれば通る**（[tests/readme.rs](../tests/readme.rs) と同じ限界）。
- **散文の主張は1つも見ていない。** 「pnpm が無いと UI が出ない」が本当かは、
  ここでは検査できない。
