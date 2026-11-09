# 起動する

```powershell
pwsh ./scripts/start.ps1
```

これ1本で立ち上がる（作業単位 F-1）。
**リポジトリ根の README は別に書く**（F-2）。ここは `scripts/` の中の案内。

順に、こうなる:

1. Rust 側のソースから**既定のポートと既定の上流エンドポイントを読む**（写していない）
2. **番号を明示したときだけ**（`-Port` か、引き継いだ `PERSONACORE_PORT`）、そのポートが空いているか見る。
   埋まっていたら、そう言って**何もせず止まる**。明示していなければ見ない（下の「ポートの決まり方」）
3. FE をビルドする（`web/node_modules` が無ければ `pnpm install` から）
4. 上流に繋がるか見る。**繋がらなくても止まらない**（下記）
5. DB の置き場所を決めて `cargo run`
6. 待ち受けが開いたら、既定のブラウザで画面を開く（**開くのは personacore 自身**。
   `-NoBrowser` は `--no-browser` を渡すだけ / 作業単位 F-4c）

## よく使う形

| やりたいこと | 書き方 |
|---|---|
| とりあえず立ち上げる | `pwsh ./scripts/start.ps1` |
| 別のポートで | `pwsh ./scripts/start.ps1 -Port 9000` |
| FE を作り直さずに再起動 | `pwsh ./scripts/start.ps1 -SkipWebBuild` |
| ブラウザは自分で開く | `pwsh ./scripts/start.ps1 -NoBrowser` |
| 別の DB で | `pwsh ./scripts/start.ps1 -DbPath D:/tmp/試し.sqlite` |
| 上流の組を切り替える | `pwsh ./scripts/start.ps1 -Upstream qwen-official`（下記） |

**⚠ `pwsh` を頭に付けること**（ドット起動だと、スクリプトが立てた `PERSONACORE_PORT` /
`PERSONACORE_DB_PATH` が手元のシェルに残り、次に `-Port` 無しで起動したとき
前回のポートを黙って引き継ぎます）。

### ポートの決まり方

待ち受けの番号は、起動するときに**この順で**決まる（[ADR-0057](../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md) §決定5）:

1. 環境変数 `PERSONACORE_PORT`（`-Port` はここへ入る）
2. 画面の「設定」で保存した番号（次の起動から効く。動いている待ち受けは移らない）
3. 既定（`src/server/config.rs` の `DEFAULT_PORT`）

- **番号を明示したとき**（`-Port` か、引き継いだ `PERSONACORE_PORT`）**だけ**、スクリプトは起動の前に
  その番号が空いているかを見て、使われていれば止まる。`[画面]` の行にも、その番号の URL を出す。
- **明示していなければ、スクリプトは事前に見ず、URL も組み立てない。** 番号を決めるのは personacore で、
  スクリプトは保存した番号を読まない（読む実装を2つ持たない）。**画面の URL は、personacore のログの
  `url=` を見る**（ブラウザは personacore が開く）。
- **`-Upstream` を付けると、保存した番号は読まれない。** スクリプトが `PERSONACORE_UPSTREAMS` を
  `scripts/` の下へ向けるので、personacore は「設定の置き場所を変えた起動」として、保存した番号を読みも書きもしない
  （覚えた上流の名前と同じ扱い）。番号を明示していなければ、既定の番号で待ち受け、画面の「設定」は保存できないと言う。
  別の番号で立てたいなら `-Port` を付ける。**`PERSONACORE_UPSTREAMS` を立てたシェルから起動したときも同じ**
  （その場合は、スクリプトの `[画面]` の行も、保存した番号を読まない起動向けの案内を出す）。

**⚠ `-Port` を変えたら、`seed-characters.ps1` にも `-BaseUrl` を渡すこと。**
あちらの既定は**サーバの既定ポート**を向いています（`-Port 9000` で立てて
既定のまま投入すると、何も居ない先を叩きます）。
2つの既定が揃っていることは `tests/start_script.rs` が見ていますが、
**渡し忘れは見ていません。** **画面の「設定」で番号を保存したときも同じ**です（あちらは保存した番号を読みません）。

**⚠ `-DbPath` に相対パスを書いたら、基準は「このコマンドを打ったディレクトリ」です**
（サーバはリポジトリのルートで走るので、スクリプトが先に絶対パスへ直してから渡します）。

止めるのは **Ctrl+C**（サーバは同じコンソールに居るので停止要求を受け取り、
DB を閉じてから終わる）。

## 上流（推論サーバ）について

**頼まれない限り、上流は誰も起動しない。** GPU を占有する行為なので、
起動するかどうかは利用者が決める（[docs/04-environment.md](../docs/04-environment.md)）。
頼むときは、下の「上流プロファイル」の gguf を書いた面を選ぶ。頼む形は、起動時の旗 `--upstream`・
画面でのプロファイルの切り替え（`POST /admin/upstream/profile`）・起こすのを待っている面（`not_started`）の
「起こす」ボタンのどれかである。頼むこと自体が、その判断である。

> **⚠ 起こすのは personacore 自身であって、このスクリプトではない**
> （[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md) / 作業単位 F-4b-1）。
> **スクリプトは旗と置き場所を渡すだけ。**

**繋がらなくても止めないのは、止めるほうが不便だから**——personacore は上流へ
**要求ごとに**繋ぐので、**先にこちらを立てて、あとから上流を起動してよい**
（personacore を起動し直す必要はない）。キャラクターを作る画面は上流が無くても使える。

> ⚠ **`-Upstream` を渡した起動では、スクリプトは疎通確認をしない。**
> 宛先を決めるのはプロファイルで、スクリプトはその中身を読まないからである
> ——読まずに既定を叩くと、**関係の無い宛先について「繋がりません」**と言うことになり、
> 読み手には「上流が落ちている」という**別の事実**に読める。
>
> ⚠ 疎通確認が見ているのは「そこに何かが居るか」だけ。
> personacore が上流に要求する能力は
> [ADR-0038 §決定1](../docs/adr/0038-what-personacore-requires-of-an-upstream.md) の3層で、
> **それを満たしているかは実際に会話するまで分からない。**
>
> ⚠ モデルが載っていない間は実効コンテキスト長を聞けないので、本文の予算が
> 落とし所へ落ちる（起動ログの `context_tokens` に出る）。しばらくすると聞き直す。

## 上流プロファイル（`-Upstream`）

`C-18` / `C-19` で「どのモデルでも会話は成立し、差は癖」と分かったので、
**1つのモデルに固定しない。切り替えを安くする**（利用者の決定 / 2026-09-18 / 作業単位 F-9）。

```powershell
pwsh ./scripts/start.ps1 -Upstream qwen-official
```

> **⚠ このスクリプトは、もうプロファイルを読まない。起こしもしない。**
> **読む・確かめる・起こす・止めるは、全部 personacore の中に在る**
> （[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md) / 作業単位 F-4b-1）。
> **スクリプトが渡すのは2つだけ** ——
> `PERSONACORE_UPSTREAMS`（置き場所）と `--upstream <名前>`。
>
> **理由は配布物**（[ADR-0041](../docs/adr/0041-one-executable-per-os.md)）——
> **配るのは実行ファイル1本で、このスクリプトは入らない。**
> ここに実装を残すと「配ったものには無い機能」の案内になる。
> **同じことをする実装を2つ持たない。**

読むのは `scripts/upstreams.json`。**無ければ `scripts/upstreams.sample.json`**
（スクリプトがどちらかを `PERSONACORE_UPSTREAMS` に入れて渡す）。

### 見本にある組

| 名前 | 何 |
|---|---|
| `-Upstream ministral-llama` | 既定のモデル（`Ministral-3-14B`）を llama.cpp で立てる。本文も抽出も同じ上流 |
| `-Upstream qwen-official` | 公式 `Qwen3.6-35B-A3B`（`--n-cpu-moe 28`）。`C-18` / `C-19` の腕 |
| `-Upstream split` | **本文は LM Studio、抽出だけ Qwen を llama.cpp で**（`C-16` の配置 / [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md)） |

**自分の組を作るなら、見本を写して直す:**

```powershell
Copy-Item ./scripts/upstreams.sample.json ./scripts/upstreams.json
```

`scripts/upstreams.json` は **`.gitignore` 済み**（gguf の置き場所は機ごとに違う）。
**写しがあると、見本は読まれない。** 見本に組を足しても、写しの側へ足すまで出てこない。

### 形

```json
{
  "組の名前": {
    "main": { "endpoint": "http://127.0.0.1:8081", "model": "頼む名前",
              "gguf": "~/.../model.gguf", "args": ["--ctx-size", "16384", "--jinja"],
              "command": "llama" },
    "sub":  { "endpoint": "…", "model": "…" }
  }
}
```

- `main` は**本文**、`sub` は**抽出と変容**。**`sub` を書かなければ、抽出も本文と同じ上流**
  （[ADR-0039 §決定1](../docs/adr/0039-two-upstreams-for-two-jobs.md)）。
- 流れる先は**既にある環境変数4つと同じ入口** ——
  `PERSONACORE_LLM_ENDPOINT` / `PERSONACORE_MAIN_MODEL` /
  `PERSONACORE_SUB_LLM_ENDPOINT` / `PERSONACORE_SUB_MODEL`。
  **サーバ側のつまみは1つも増えていない。**
- `gguf` を**書いた面だけ**、personacore が `llama serve` を起こす。書かなければ起こさない
  （`split` の `main` は LM Studio なので書いていない）。
- **`command` は任意**（既定 `llama`）。**PATH に無い `llama` を絶対パスで指すためだけに在る**
  （[ADR-0042 §決定1](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）。
  「モデルの置き場所が変わる」のと同じ理由で、**実行ファイルの置き場所も変わる。**
- **`gguf` を書いた面の `endpoint` は、ループバックだけ**
  （`127.0.0.1` / `localhost` / `[::1]`）。そのホストが `llama serve --host` に渡るので、
  **外から繋がる待ち受けを作らないようにしている。**
  **`gguf` の無い面は縛らない** —— あちらの待ち受けを作ったのは personacore ではなく、
  縛ると「別の機に上流を置く」形（[ADR-0039 §結果](../docs/adr/0039-two-upstreams-for-two-jobs.md)）が
  書けなくなる。
- **`endpoint` はホストとポートまで書く**（例 `http://127.0.0.1:8080` / 作業単位 B-68・Issue origin#97）。
  `/v1/chat/completions` などは personacore が足すので、`…/v1` まで書くと `…/v1/v1/…` に送る
  （中継の説明書きが `…/v1` までを宛先として示していても、`/v1` は外す）。
  **画面は末尾が `/v1` の宛先を保存の前に止める。ファイルに直接そう書いた場合は止めず、警告のログを出す**
  （既に書いた設定で起動できなくしない）。警告は起動時にその面ごとに1行、そのプロファイルへ切り替えるたびにも1行出る。パスの前置き（`https://example.com/api`）は書ける。
  **`/v1` 以外の版の綴りを使う上流には、いまは対応していない。**
- **`api_key` は任意**（[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md)）。鍵を求める上流
  （「API キー」を作った後の、鍵で守られた中継など）への鍵で、その面への送信の全部に `Authorization: Bearer <鍵>` が付く。
  **面ごとの値**（`sub` に書かなければ、`main` の鍵を裏方へ貸さない）。**鍵を平文の `http://` で別の機へ送るときは、
  ローカルネットワークなら警告して送り、それ以外が混ざるなら面に `"allow_key_over_plain_http": true` を書いたときだけ送る**
  （書いていなければ送らない）。**ファイルには平文で残る**ので、
  他人に渡すときは消すこと。書き方の詳細は [docs/04-environment.md](../docs/04-environment.md)。
- **知らない面・知らない鍵は、名前を挙げて止まる**
  （面は `main` / `sub`、鍵は `endpoint` / `model` / `gguf` / `args` / `command` / `context_tokens` / `api_key` /
  `allow_key_over_plain_http` だけ）。
  `gguf` を `ggml` と綴っても**黙って無視しない**（無視すると「起こす」が静かに消えて、
  あとで出る「上流に繋がりません」が**別の事実**に読める）。
- `args` は `llama serve` へそのまま渡す。**`--host` と `--port` は書かない** ——
  `endpoint` から作って personacore が足す。
  **1要素に空白を入れても割れない**（`Command` が引数を1つずつ渡す。
  PowerShell の `Start-Process` はそれが出来ず、`F-9` は手で引用符を足していた）。
- `~` は利用者のホームへ展開する（見本を別の機へ写せるように）。

### 起こす前に見ること

**1つでも駄目なら、何も起こさずに止まる:**

- プロファイルのファイルが無い → **見に行った場所を挙げて止まる**
- プロファイル名が無い → **在る名前を並べて止まる**（黙って既定へ落ちない）
- **知らない面**（`main` / `sub` 以外）→ その名前を挙げて止まる
- **知らない鍵**（上の7つ以外）→ その綴りを挙げて止まる
- **`gguf` を書いた面の宛先がループバックでない** → 止まる（`--host` に渡る値だから）
- **`api_key` を書いた面の宛先が `https://` でもループバックの `http://` でもない** → 止まる（鍵が暗号化されずに別の機へ流れるから）
- **`api_key` が空・4096 バイト超・空白や改行や ASCII の外の字を含む** → 止まる（**鍵の綴りは挙げない**）
- **`gguf` のファイルが無い** → そのパスを挙げて止まる（**1つも起こさない**）
- `main` と `sub` が**同じポートに別の gguf** を載せようとしている（1 プロセス 1 モデル）
- **`gguf` を書いた面の宛先が、personacore 自身の待ち受けと同じポート** →
  **両方の値を挙げて止まる**（作業単位 B-52）。**起こしてしまうと、`llama` は
  そのポートを取れずに 2 秒で `exit code: 1` になり、
  画面に出せるのは「失敗（exit code: 1）」だけになる。**
  **`gguf` の無い面は縛らない**（そこに待ち受けを作ったのは personacore ではない）。
  **見本は `8081`** —— サーバの既定（`8080`）を避けてある。

> **⚠ `F-9` に在って、いま無い検査が2つある** ——
> **「`llama` が既に動いている」**と**「その宛先のポートに既に何かが居る」。**
> **止めるのは自分が控えた子だけ**（[ADR-0042 §決定2](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）なので、
> **他人の `llama` を名前で数える経路そのものを作っていない**
> ——数えると、`split`（LM Studio が内側で `llama-server` を立てる）が成り立たなくなる。
> **ポートが埋まっていれば、起こした `llama` が自分で失敗して終わり、
> `GET /admin/upstream` の `launch.state` が `failed` になる。**
>
> **⚠ ただし「埋めているのが personacore 自身」だけは、起こす前に止まる**
> （上の箇条書きの最後 / 作業単位 B-52）——
> **その1つだけは、起こす前に両方の値が分かっている。**

### 起こしたあと

**待ち受けは先に開く。載るのは背景で待つ**（[ADR-0042 §決定3](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）——
**画面はすぐ使える。** `F-9` の順（載るまで待ってから起動）だと、20 GB の gguf で
**1〜2分のあいだ画面が出なかった。**

いま何が起きているかは **`GET /admin/upstream` の `launch` 欄**に出る（面ごと）:

| `launch.state` | 意味 |
|---|---|
| `external` | **起こしていない**（LM Studio・利用者が手で立てたもの・gguf を書いていない面・プロファイルを選んでいない起動） |
| `not_started` | **起こすのを待っている**（旗を渡さずに personacore を起動し、覚えた名前のプロファイルを使ったときの、gguf を書いた面 / [ADR-0055](../docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md)）。**同じ名前の `POST /admin/upstream/profile` で起こす**（画面の「起こす」ボタンが `selected_profile` の名前でこれを送る） |
| `loading` | 載せている最中（`since` に開始時刻） |
| `ready` | 載った（`answered_name` に**上流が名乗った gguf のファイル名**） |
| `failed` | 子が自分で終了した／別のモデルが載った（`reason` と `log`） |
| `stopped` | personacore が止めた |

**「載った」と認めるのは、`/v1/models` が gguf のファイル名を名乗ったとき。**
**頼んだ名前では確かめない** —— **`llama serve` は `model` の欄を見ない**（`B-45` / `C-18`）。

**待ちに固定の打ち切りは無い**（[ADR-0035](../docs/adr/0035-the-watchdog-reports-the-human-decides.md)）。
**15 分を超えたら「まだ載っていません。待ち続けます」と1本言うだけで、止めない。**
**⚠ ただし子が自分で終了したら即「失敗」** —— 死んだ書き手は待たない。

### 起こしたものは止める

personacore が停止するとき（Ctrl+C を含む）、**自分が控えた子だけ**を止めて終了を待つ。
**利用者が別に立てた上流には触らない**（名前で探す経路がそもそも無い）。

**⚠ personacore を強制終了した場合は、この後始末が走らない。**
そのときは起こした `llama` が残るので、自分で止めること（`F-9` から変わっていない）。

**personacore を走らせたままの上流の差し替えは、上流モデルの画面か `POST /admin/upstream/profile` で行う**
（[ADR-0042 §決定4](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md) / 作業単位 `F-4b-2`）。
起動し直さなくてよい。差し替えでも、新しいプロファイルに残らない子を止めてから起こす。

ログは **DB と同じディレクトリの `upstream-logs/`**（既定では
`%LOCALAPPDATA%\personacore\upstream-logs\`、スクリプト経由なら `data/upstream-logs/`）。
**`GET /admin/upstream` に出るのはファイル名だけ**（置き場所は載せない / 絶対制約）。

### 守れないこと

- **`args` に `--host` / `--port` を書いた場合の結末は、「いつまでも載らない」である。**
  personacore は `endpoint` から作った `--host` / `--port` を必ず渡すので、同じ旗が2回並ぶ。
  **どちらが勝つかは `llama serve` 側の話で、ここでは測っていない**（上流を起こす測定は
  司令塔がやる）。**`args` 側が勝てば、personacore が叩く宛先には誰も居ないまま
  `launch.state` が `loading` のままになる。**
- **既に立っている上流には、この旗は届かない。** 旗がするのは「4つの欄を決める」ことと
  「gguf の面を起こす」ことだけ。
- **`--upstream` と `PERSONACORE_LLM_ENDPOINT` がぶつかったら、プロファイルが勝つ**
  （黙って上書きせず、前の値を言ってから上書きする）。
- **どの組を常用するかは、測定では決まらない**（[ADR-0039 §結果](../docs/adr/0039-two-upstreams-for-two-jobs.md)）。
  切り替えが安くなるだけで、選ばない。

## 配った実行ファイルだけで使ってみる

**Rust も pnpm も PowerShell も要らない道**（[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)）。
要るのは2つだけ:

1. `dist/personacore.exe`（作り方は [`package.ps1`](package.ps1) / [ADR-0041](../docs/adr/0041-one-executable-per-os.md)）
2. `%LOCALAPPDATA%\personacore\upstreams.json`（上の「形」のとおり。gguf の場所を直す）
   —— **無ければ、初めて叩いたときに実行ファイルが見本を書く**（作業単位 F-4c）。
   **在るファイルには触らない。** 書いたときは、起動ログと画面（`#/upstream`）がその場所を言う。

```text
personacore.exe --upstream qwen-official
```

起動ログが `待ち受けを開始しました addr=127.0.0.1:<ポート> …` と言ったら、
（**ポートの既定値はここに写さない。** 起動ログが出す値がすべてである）
**実行ファイルがその URL を既定のブラウザで開く**（作業単位 F-4c。`--no-browser` で開かない。
開けなかったときは WARN に URL が出るので、自分で開く）。
**どのコミットの実行ファイルかは `personacore.exe --version`。**
**上流が載る前でも画面は使える** —— キャラクターの作成は上流が無くても通る。
載ったかどうかは `GET /admin/upstream` の `launch` 欄。

- **DB もプロファイルも `%LOCALAPPDATA%\personacore\` の下**（[ADR-0041 §決定3](../docs/adr/0041-one-executable-per-os.md)）。
  **見る場所を2つにしない。**
- **別の場所に置きたいなら `PERSONACORE_UPSTREAMS`**（ファイルのパス）**と
  `PERSONACORE_DB_PATH`。**
- **まっさらから始めるなら `--fresh`**（消さずに退避する / ADR-0041 §決定4）。
- **画面を配らずに API だけで使うなら `--no-ui`**（ブラウザも開かない。`/` は 404 で、`/v1` と `/admin` と
  `GET /openapi.json` には答える / [ADR-0047](../docs/adr/0047-the-api-is-the-product.md) §①）。
  **`start.ps1` にこの旗に当たる引数は無い** —— スクリプトを使わずに立てるなら `cargo run -- --no-ui`（下の「手で立ち上げる」）。

> **⚠ この道は、本物の `llama` では歩いていない**（2026-09-19 / 作業単位 F-4b-1）。
> **エージェントは GPU を掴む経路を走らせない**（[CLAUDE.md](../CLAUDE.md) §4）ので、
> **確かめたのは「偽の `llama`」を `PATH` の先頭に置いた走行だけ**
> （`tests/upstream_launch.rs` / `tests/upstream_cli.rs`）。
> **本物で確かめていないのは、渡している引数が正しいことと、本物がモデルを載せること。**
> **実機での確認は司令塔が行う。**

## つまみを増やしていない

スクリプトがサーバへ渡すのは**既にある環境変数だけ**で、既定値は Rust 側から読む。
`-Port` は `PERSONACORE_PORT`、`-DbPath` は `PERSONACORE_DB_PATH` に入るだけ。

**立てないものが2つある**（意図的）:

- **`RUST_LOG`** —— 未設定のときの既定フィルタが一番よく喋る。
  1文字でも書くと `personacore=info` が消える（[docs/04-environment.md](../docs/04-environment.md)）。
- **`PERSONACORE_STAT_DRIFT`** —— **実験の切り替えスイッチ**で、既定が `off` なのには理由がある
  （`src/server/config.rs` の `ENV_STAT_DRIFT` の doc）。試すなら呼ぶ前に自分で立てる:

  ```powershell
  $env:PERSONACORE_STAT_DRIFT = 'on'; pwsh ./scripts/start.ps1
  ```

## DB の置き場所

既定は **`data/personacore.sqlite`**（`.gitignore` 済み）。

**サーバの既定はリポジトリ直下**で、そこには古い DB が置いてある
（[docs/06-roadmap.md](../docs/06-roadmap.md) の F 箱）。環境変数を指定せずに起動すると
そこへ続きのマイグレーションが当たるので、スクリプトは明示して避ける。

> ⚠ 実行ファイル単体で配る形（F-4）になったら、この置き場所は決め直しになる。

## 見張り番

`tests/start_script.rs` が、スクリプトの綴りとサーバの実体を突き合わせる
（環境変数の綴り／既定のポート／既定の上流／DB の置き場所／宛先がループバックだけであること）。

### スクリプトに残る主張（`tests/start_script.rs`）

**`F-9` は8つ持っていたが、ADR-0042 §決定6 で実装が実行ファイルへ移ったので、
スクリプト側に残るのは5つである**（名前は `tests/start_script.rs`）——

①スクリプトが名指しする見本が実在し、利用者の写しは `.gitignore` に在る
（`the_start_script_points_at_the_sample_profile_that_exists`）
②`PERSONACORE_UPSTREAMS` は**旗の分岐の中にしか立たない**（旗なしの既定が変わらない）。
**そして上流の環境変数4つは、スクリプトが1つも立てない**
（`the_upstream_environment_variables_are_set_only_inside_the_upstream_branch`）
③旗と置き場所を**実行ファイルへ流す3行**が、それぞれ1箇所に在る
（`the_start_script_hands_the_upstream_flag_to_the_executable`）
④**起こす・止める・読む綴りが1つも残っていない**
（`the_start_script_no_longer_starts_or_stops_the_upstream`）
⑤**スクリプト引数と同じ名前の変数へ代入しない**
（`no_script_parameter_is_reused_as_a_local_variable`）。
**PowerShell の変数名は大文字小文字を区別せず、`param(…)` に書いた型は
その名前への代入すべてに効く**ので、値が黙って別の型に化ける。

そのほか、見本そのものを見るものが3つ ——
見本のどの面もループバック以外を向かない
（`the_upstream_profiles_name_no_address_other_than_the_loopback`）／
見本の `main` と `sub` が同じポートに2つの gguf を載せない
（`no_upstream_profile_points_two_gguf_at_the_same_port`）／
この案内が挙げる組の名前と見本の組が一致する
（`the_readme_names_exactly_the_profiles_the_sample_defines`）。

**見本を本番の判定に通すものは `tests/upstream_profiles.rs` にある** ——
読めること（`the_sample_profile_parses_with_the_parser_the_executable_uses`）と、
**どの面もサーバの既定の待ち受けを取りに行かないこと**
（`no_sample_profile_launches_on_the_default_bind_port` / 作業単位 B-52）。

> **⚠ 消えた2つは、主張が移っただけである**（作業単位 F-4b-1 / 司令塔の採否を仰いだ）——
> `the_started_upstreams_are_stopped_by_recorded_pid_when_the_script_ends` と
> `the_started_upstream_is_recorded_before_the_script_waits_for_it`。
> **どちらも、いまは本物のプロセスで見ている**（下の表）。**綴りより強い。**

**⚠ 実行はしていない。** 「書いてあるとおりに動くこと」は `tests/start_script.rs` が見ていない。
**⚠ 「宣言した値を使っているか」を見る節も同じ**——言えるのは
「その行がソースにある」までで、**その行が効いていること**は見ていない。

### 実行ファイル側の見張り番（**本物のプロセスで見る**）

**`PATH` の先頭に「偽の `llama`」**（何もしない同名の実行ファイル）**を置いて、
本当に起こして本当に止める**（[CLAUDE.md](../CLAUDE.md) §4「本物を起こせない経路は偽物で起こす」）。
**GPU も VRAM も1バイトも使わない。**

| 主張 | 見張り番 |
|---|---|
| **`PATH` の `llama` を起こし、gguf の名前を名乗ったら `ready`。止めたら子も消える** | `tests/upstream_launch.rs::the_named_gguf_is_started_reaches_ready_and_is_stopped_with_the_server` |
| **止めるのは控えた子だけ**（他人の `llama` は生きている） | 同 `stopping_the_server_leaves_someone_elses_llama_alone` |
| **子が自分で終了したら即「失敗」**（死んだ書き手を待たない） | 同 `a_child_that_exits_on_its_own_is_reported_as_failed` |
| **別の名前を名乗ったら「失敗」** | 同 `an_upstream_that_names_another_model_is_reported_as_failed` |
| **gguf が無ければ1つも起こさない** | 同 `a_missing_gguf_starts_nothing_and_stops_the_startup` |
| **待ち受けに失敗したら子が1本も居ない**（＝起こすのは待ち受けの後） | 同 `a_failed_bind_leaves_no_child_behind` |
| **待ち受けと同じポートを取りに行く面は、起こす前に止まる**（両方の値を挙げて） | 同 `a_face_that_wants_the_servers_own_port_stops_before_anything_is_started` |
| **`gguf` の無い面は同じポートを名乗ってよい**（対比。外の上流は縛らない） | 同 `an_external_face_may_name_the_servers_own_port` |
| **旗が無ければ1つも起こさない**（対比） | 同 `an_empty_plan_launches_nothing_and_leaves_both_faces_external` |
| **起こすのを待つ起動では何も起こさず、面は `not_started` のままになる。同じ名前の `POST` で起こすと `ready` まで行く**（ADR-0055） | 同 `a_waiting_start_launches_nothing_until_the_same_name_is_asked` |
| **待っていた面を、起こさないプロファイルへ切り替えたら `external`** | 同 `a_waiting_face_switched_to_a_profile_that_does_not_launch_it_becomes_external` |
| **覚えた名前で起きた実行ファイルは起こさずに待ち、`--upstream` で起きた実行ファイルは起こす**（対） | `tests/upstream_cli.rs::an_upstream_flag_is_remembered_for_the_next_bare_start` |
| **旗と置き場所が実行ファイルへ届き、プロファイルが環境変数に勝つ** | `tests/upstream_cli.rs::the_named_profile_reaches_the_executable_and_wins_over_the_environment` |
| **知らない名前・無いファイル・無い gguf で起動が止まる** | 同 `an_unknown_profile_name_stops_the_executable` ほか2本 |
| **プロファイルの検証規則そのもの**（面・鍵・ループバック・`~`・同じポート） | `tests/upstream_profiles.rs`（17 本） |

**⚠ `tests/upstream_cli.rs` は生きた偽物を残さない。**
テストから実行ファイルへ送れるのは `kill` だけで、**それでは `serve` が返らず、
起こした子が孤児として残る**（2026-09-18 に実測 —— `cargo test` の出力を掴んだまま、
読み手が返らなくなった）。**一生を見る腕は `tests/upstream_launch.rs` にある**
（`App::run_with_upstreams` を直に呼ぶので、止める合図を送れる）。

**⚠ 確かめていないもの** ——**本物の `llama serve` へ渡している引数が正しいこと**と
**本物がモデルを載せること**（偽物は引数を見ない）。**そこは司令塔の実機確認である。**
**⚠ ブラウザが実際に開くこと**も確かめていない（エージェントはブラウザを起動しない）。

実際に走らせて確かめたのはここまで（F-1 / 作業者とレビュアー）——
**素の起動 / `-Port` / `-DbPath`（絶対・相対の両方） / `PERSONACORE_PORT` の読み取りと拒否 /
ポート衝突で止まること / 上流に繋がる側と繋がらない側の両方 / `web_dir_exists=true` で
画面が配られること / `netstat` で `127.0.0.1` のみが待ち受けていること /
Ctrl+C で DB が閉じられること**（WAL がチェックポイントされ、孤児プロセスも残らない）。

> **⚠ `web_dir_exists` という欄は、いまは無い。** **F-4a で `ui_source` / `ui_origin` / `ui_files` へ移った**
> （ADR-0041 §決定2）。**上の行は F-1 の時点で観測したものをそのまま残してある。**

## キャラクターを作る

最初の画面は「キャラクターがまだ1件もありません」。
**「キャラクター」→「新規作成」**で作る。まとめて投入したいなら
[`seed-characters.ps1`](seed-characters.ps1)。

## スクリプトを使わずに手で立ち上げる

```powershell
cd web; pnpm install; pnpm run build; cd ..
$env:PERSONACORE_DB_PATH = "$PWD/data/personacore.sqlite"
cargo run
```

**⚠ 画面はコンパイル時に実行ファイルへ埋め込まれる**（ADR-0041 §決定2 / 作業単位 F-4a）。
つまり `pnpm run build` は `cargo run` より**先**に回すこと——後だと、
**実行ファイルの中の画面が古いまま**になる。
起動ログの `ui_source` / `ui_origin` / `ui_files` がそれを言う
（1件も無ければ `ui_files=0` と警告が出る）。

`PERSONACORE_WEB_DIR` を立てると、埋め込みではなく**そのディレクトリから**配る
（`vite build --watch` の成果物を、実行ファイルを組み直さずに見るための旗）。

> **⚠ テスト用の実行ファイルを `[[bin]]` に足すと、素の `cargo run` が曖昧になる。**
> **`Cargo.toml` の `default-run = "personacore"` がそれを守る**
> （2026-09-19 に実機で踏んだ —— 偽の `llama` を足した瞬間、`start.ps1 -Upstream …` が
> `could not determine which binary to run` で exit 101 になった）。
> **`required-features` は候補から外す効果を持たない。**
> 見張り番は `tests/readme.rs::the_manifest_names_a_default_binary_when_more_than_one_exists`。
