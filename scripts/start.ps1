#Requires -Version 7.0
<#
.SYNOPSIS
    personacore を1コマンドで立ち上げる（FE のビルド → 上流の疎通確認 → 起動 → ブラウザ）。

.DESCRIPTION
    作業単位 F-1。手で並べていた手順を1本にまとめただけで、**新しい設定項目は1つも作らない。**
    サーバへ渡すのは既にある環境変数だけで、既定値は Rust 側のソースから**読む**
    （写さない。web/vite.config.ts と同じ作法）。

    やること:

      1. Rust 側から既定のポートと既定の上流エンドポイントを読む
      2. **番号を明示したときだけ**（-Port か、引き継いだ PERSONACORE_PORT）、ポートが空いているか見る
         （前回の personacore が残っていたら、そう言って止まる）。明示していなければ見ない（ADR-0057 §決定7）
      3. FE をビルドする（web/node_modules が無ければ pnpm install から）
      4. 上流に繋がるか見る。**繋がらなくても止めない**（下記）
      5. DB の置き場所を決めて（$env:PERSONACORE_DB_PATH）、cargo run で起動する
      6. 既定のブラウザで画面を開く（**開くのは personacore 自身**。作業単位 F-4c）

    ⚠ 止めない理由: personacore は上流へ**要求ごとに**繋ぐ。つまり
    「先にこちらを起動して、あとから上流を起動する」で動く（起動し直さなくてよい）。
    キャラクターを作る画面は上流が無くても使えるので、止めるほうが不便になる。
    ⚠ ただしモデルが載っていない間は、実効コンテキスト長を聞けないので
    本文の予算が落とし所へ落ちる（ADR-0020）。しばらくすると聞き直す。

    ⚠ このスクリプトは上流（LM Studio 等）を起動しない。GPU を占有する行為なので、
    起動するかどうかは利用者が決める（docs/04-environment.md）。
    ⚠ 例外は -Upstream だけ（作業単位 F-9）。**旗を渡すこと自体が利用者の判断**なので、
    そのときだけ、プロファイルに gguf が書いてある面のために推論サーバを起こす。
    旗を渡さない起動は、いままでと1バイトも変わらない。

.PARAMETER Port
    待ち受けポート。指定すると $env:PERSONACORE_PORT に入れて渡す（画面で保存した番号より優先される）。

    ⚠ **番号を明示したとき（-Port か、引き継いだ $env:PERSONACORE_PORT）だけ**、起動の前にその番号が
      空いているかを見て、使われていれば止まる。画面の URL もその番号で表示する。
    ⚠ **明示していなければ、番号を決めるのは personacore**（画面の「設定」で保存した番号、無ければ Rust 側の既定 /
      ADR-0057 §決定5）。このスクリプトは事前に見ず、URL も組み立てない ——
      **personacore のログの url= を見ること。**
    ⚠ -Upstream を付けると $env:PERSONACORE_UPSTREAMS が変わるので、personacore は保存した番号を読まない
      （明示していなければ既定の番号で待ち受ける）。

.PARAMETER DbPath
    SQLite のファイル。省略時は $env:PERSONACORE_DB_PATH、それも無ければ
    リポジトリ直下の data/ の下（下の「DB の置き場所」を参照）。
    ⚠ **相対パスは「このコマンドを打ったディレクトリ」を基準に解決する**
    （サーバはリポジトリのルートで走るので、絶対化しないと基準が2つに割れる）。

.PARAMETER SkipWebBuild
    FE のビルドを飛ばす（既に web/dist がある状態での再起動用）。
    ⚠ cargo は飛ばさない。古い実行ファイルで起動するほうが危ないため。

.PARAMETER NoBrowser
    ブラウザを開かない（URL は表示する）。personacore に --no-browser を渡すだけ。

    ⚠ **ブラウザを開くのは、このスクリプトではない**（作業単位 F-4c）。
      personacore が待ち受けを開いた後に自分で開く。**ここで開くと二重に開く。**

.PARAMETER Upstream
    上流プロファイルの名前（作業単位 F-9 / ADR-0042 §決定6）。

    ⚠ **このスクリプトはもう、プロファイルを読まない。起こしもしない。**
      渡すのは2つだけ ——
        * $env:PERSONACORE_UPSTREAMS に scripts/upstreams.json
          （無ければ scripts/upstreams.sample.json）
        * cargo run へ `--upstream <名前>`
      **読む・確かめる・起こす・止めるは、全部 personacore の中にある**
      （ADR-0042 §決定1〜3）。**同じことをする実装を2つ持たない。**

    ⚠ 名前が無ければ、在る名前を並べて**実行ファイルが**止まる（黙って既定へ落ちない）。
    ⚠ gguf が書いてある面だけ、**実行ファイルが** llama serve を起こし、
      終了時に**自分が控えた子だけ**を止める。
    ⚠ PERSONACORE_LLM_ENDPOINT が既に立っていても、**プロファイルが勝つ**
      （実行ファイルが、そう言ってから上書きする）。
    ⚠ 載るのを待つのは背景で、待ち受けは先に開く。**画面はすぐ使える。**
      いま何が起きているかは GET /admin/upstream の launch 欄に出る。
    ⚠ **-Upstream を付けないと、配布版の exe で覚えた名前が開発中も効く**
      （このときは PERSONACORE_UPSTREAMS を立てないので、既定の置き場所の selected-upstream.txt を読む /
      ADR-0042 §決定9）。**-Upstream を付けた起動は、覚えた名前を読みも書きもしない。**

.EXAMPLE
    PS> pwsh ./scripts/start.ps1
    PS> pwsh ./scripts/start.ps1 -Port 9000 -SkipWebBuild
    PS> pwsh ./scripts/start.ps1 -Upstream qwen-official

    ⚠ `pwsh` を頭に付けて**別プロセスで**起動すること（README と同じ形）。
    ドット起動（`./scripts/start.ps1`）だと、このスクリプトが立てた
    PERSONACORE_PORT / PERSONACORE_DB_PATH が呼び出し元のシェルに残り、
    次に -Port を付けずに起動したとき前回のポートを黙って引き継ぐ。

.NOTES
    ⚠ RUST_LOG は立てない。未設定のときの既定フィルタが一番よく喋る
    （docs/04-environment.md「引き継がれるのは床だけ」——RUST_LOG を1文字でも書くと
    personacore=info が消える）。

    ⚠ PERSONACORE_STAT_DRIFT も立てない。**実験の切り替えスイッチ**であって、
    既定が off なのには理由がある（src/server/config.rs の ENV_STAT_DRIFT の doc）。
    試したいときは、このスクリプトを呼ぶ前に自分で立てること。

    DB の置き場所（F-1 で決めたこと）:
      サーバの既定はリポジトリ直下で、**そこには古い DB が置いてある**
      （0004 までしか適用されていない / docs/06-roadmap.md の F 箱）。
      そのまま起動すると、その古い DB に続きのマイグレーションが当たる。
      なのでこのスクリプトは data/ の下を明示して渡す（.gitignore 済み）。
      ⚠ 実行ファイル単体で配る形（F-4）になったら、この置き場所は決め直しになる。
#>

[CmdletBinding()]
param(
    [ValidateRange(1, 65535)]
    [int]$Port,

    [string]$DbPath,

    [switch]$SkipWebBuild,

    [switch]$NoBrowser,

    [string]$Upstream
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# native コマンドの失敗は $LASTEXITCODE で自分で見る（版によって既定が違うため固定する）。
$PSNativeCommandUseErrorActionPreference = $false

# ----------------------------------------------------------------------
# Rust 側の既定値の在処。**写さずに読む。**
# ⚠ この4つの綴りは tests/start_script.rs が読んで、実体と突き合わせる。
#    形（$名前 = '値'）を崩すと、あちらが「拾えない」と言って赤くなる。
# ----------------------------------------------------------------------
$DefaultPortSource = 'src/server/config.rs'
$DefaultPortPrefix = 'pub const DEFAULT_PORT: u16 = '
$DefaultEndpointSource = 'src/llm/lmstudio.rs'
$DefaultEndpointPrefix = 'pub const DEFAULT_ENDPOINT: &str = '

# 既定の DB の置き場所（リポジトリのルートから）。**サーバの既定とは別の場所。**
$DefaultDbRelative = 'data/personacore.sqlite'

# ----------------------------------------------------------------------
# 上流プロファイル（作業単位 F-9）。**-Upstream を渡したときだけ読む。**
# ⚠ この2つの綴りも tests/start_script.rs が読んで、実体と突き合わせる。
#    見本はコミットする。利用者の写しは .gitignore 済み（機ごとに gguf の場所が違う）。
# ----------------------------------------------------------------------
$UpstreamProfileFile = 'scripts/upstreams.json'
$UpstreamProfileSample = 'scripts/upstreams.sample.json'

# ⚠ **プロファイルを読むのはここではない**（ADR-0042 §決定6 / 作業単位 F-4b-1）。
# 実行ファイルが読み、確かめ、起こし、止める。**このスクリプトは置き場所を渡すだけ。**

# FE のソースが置いてあるディレクトリ。
$WebRoot = 'web'

$RepoRoot = Split-Path -Parent $PSScriptRoot

# ----------------------------------------------------------------------
# 道具
# ----------------------------------------------------------------------

# Rust のソースから `<接頭辞><値>;` の値を読む（前後の空白と引用符は落とす）。
# **読めなければ投げる。** ここで黙って既定へ落ちると、それは「読んでいる」ではなく
# 「写している」のと同じになる（web/vite.config.ts と同じ判断）。
function Read-RustConst {
    param(
        [Parameter(Mandatory)][string]$File,
        [Parameter(Mandatory)][string]$Prefix
    )

    $path = Join-Path $RepoRoot $File
    if (-not (Test-Path -LiteralPath $path)) {
        throw "$File が見つかりません（リポジトリのルート: $RepoRoot）。"
    }

    $source = Get-Content -LiteralPath $path -Raw -Encoding utf8
    $at = $source.IndexOf($Prefix, [System.StringComparison]::Ordinal)
    if ($at -lt 0) {
        throw "$File に「$Prefix」がありません。綴りが変わったなら start.ps1 も合わせてください。"
    }

    $rest = $source.Substring($at + $Prefix.Length)
    $end = $rest.IndexOf(';', [System.StringComparison]::Ordinal)
    if ($end -lt 0) {
        throw "$File の「$Prefix」の直後がセミコロンで終わっていません。"
    }

    return $rest.Substring(0, $end).Trim().Trim('"')
}

# 宛先に TCP で繋がるか。**繋がるかどうかだけ**を見る。
function Test-TcpReachable {
    param(
        [Parameter(Mandatory)][string]$TargetHost,
        [Parameter(Mandatory)][int]$TargetPort,
        [int]$TimeoutMs = 1500
    )

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync($TargetHost, $TargetPort)
        if (-not $task.Wait($TimeoutMs)) { return $false }
        return $client.Connected
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

# ⚠ **プロファイルを読む道具は、ここから消した**（ADR-0042 §決定6 / 作業単位 F-4b-1）。
#
# `Test-JsonField` / `Resolve-HomePath` / `Read-UpstreamFace` /
# `Start-UpstreamProcess` / `Wait-UpstreamReady` は、**実行ファイルの中へ移った**
# （`src/server/upstreams.rs` と `src/server/launch.rs`）。
#
# **同じことをする実装を2つ持たない**——片方が黙って古びる。
# **そして配布物にはこのスクリプトが入らない**（ADR-0041）ので、
# ここに残しておくと「配ったものには無い機能」の案内になる。

# ----------------------------------------------------------------------
# 1. 既定値を読む
# ----------------------------------------------------------------------
$defaultPort = [int](Read-RustConst -File $DefaultPortSource -Prefix $DefaultPortPrefix)
$defaultEndpoint = Read-RustConst -File $DefaultEndpointSource -Prefix $DefaultEndpointPrefix

# ----------------------------------------------------------------------
# 1b. 上流プロファイル（**-Upstream を渡したときだけ** / ADR-0042 §決定6）
#
# ⚠ **ここでするのは「置き場所を渡すこと」だけ。**
#    読む・確かめる・起こす・止めるは、全部 personacore の中にある
#    （src/server/upstreams.rs と src/server/launch.rs）。
#    **同じことをする実装を2つ持たない。**
#
# ⚠ PERSONACORE_UPSTREAMS を立てるのは、**この分岐の中だけ。**
#    外へ出すと、旗を渡していない起動が黙ってプロファイルを読みに行く。
#    見張り番は tests/start_script.rs の
#    the_upstream_environment_variables_are_set_only_inside_the_upstream_branch。
# ----------------------------------------------------------------------
# cargo run へ足す引数。**旗を渡さない起動では空のまま**（いままでと1バイトも変わらない）。
$serverArgs = @()

if ($PSBoundParameters.ContainsKey('Upstream')) {
    # **利用者の写しが在ればそちら、無ければ見本。**
    # 実行ファイルの既定は OS のデータディレクトリの下だが、
    # **開発中の置き場所はリポジトリの中のまま**にする（ADR-0042 §決定1）。
    $profilePath = Join-Path $RepoRoot $UpstreamProfileFile
    if (-not (Test-Path -LiteralPath $profilePath)) {
        $profilePath = Join-Path $RepoRoot $UpstreamProfileSample
    }
    if (-not (Test-Path -LiteralPath $profilePath)) {
        throw "上流プロファイルが見つかりません（$UpstreamProfileFile も $UpstreamProfileSample も）。"
    }

    $env:PERSONACORE_UPSTREAMS = $profilePath
    $serverArgs = @('--upstream', $Upstream)

    Write-Host "[上流] プロファイル '$Upstream'（$profilePath）"
    Write-Host '[上流] 読むのも起こすのも personacore 自身です（GET /admin/upstream の launch 欄に出ます）'
    Write-Host '[上流] 待ち受けは先に開きます。載るのは背景で待つので、画面はすぐ使えます'
}

# ブラウザを開くのは personacore 自身（作業単位 F-4c）。**このスクリプトは旗を流すだけ。**
# ⚠ **`-Upstream` の分岐より後に置く** —— あちらは `$serverArgs` を代入で作り直すので、
#    前に置くと `--no-browser` が黙って消える。
if ($NoBrowser) {
    $serverArgs += '--no-browser'
}

# 上流の宛先も**ここで決めて、ここで検証する。**
# ⚠ FE のビルドより前に見るのは、書式が壊れているときに
# 「ビルドを全部終えてから、意味の分からない英語の例外で落ちる」のを避けるため
# （[uri] はスキームが無いと相対 URI になり、Host が空文字列になる）。
#
# ⚠ **-Upstream を渡したときは、決めも検証もしない**（ADR-0042 §決定6 / レビュー是正6）。
#    宛先を決めるのはプロファイルで、このスクリプトはその中身を読まない。
#    **読まずに PERSONACORE_LLM_ENDPOINT（や既定）を検証すると、
#    プロファイルが勝つ起動でも、壊れた環境変数が立っている機では throw で止まる。**
#    疎通の確認を黙らせたのと同じ理由 —— **この値は、その起動では使われない。**
#    **書式が壊れていれば、実行ファイルの側が自分で止める**（AppConfig の検証）。
$endpoint = $null
$upstreamUri = $null
if (-not $PSBoundParameters.ContainsKey('Upstream')) {
    $endpoint = if (-not [string]::IsNullOrWhiteSpace($env:PERSONACORE_LLM_ENDPOINT)) {
        $env:PERSONACORE_LLM_ENDPOINT.Trim()
    }
    else {
        $defaultEndpoint
    }

    $upstreamUri = $endpoint -as [uri]
    if ($null -eq $upstreamUri -or -not $upstreamUri.IsAbsoluteUri -or
        [string]::IsNullOrEmpty($upstreamUri.Host)) {
        throw "上流のエンドポイントが URL として読めません: $endpoint" +
        "（PERSONACORE_LLM_ENDPOINT は http:// か https:// から書いてください）"
    }
}

# ----------------------------------------------------------------------
# 番号を明示したか（作業単位 F-12b / ADR-0057 §決定7）
#
# ⚠ **明示したのは -Port か、引き継いだ PERSONACORE_PORT のときだけ。**
#    明示していなければ、番号を決めるのは personacore（画面の「設定」で保存した番号、無ければ既定）で、
#    **このスクリプトはその番号を知らない**（保存のファイルを読まない —— 読む実装を2つ持たない）。
#    既定の番号で事前検査して URL を組み立てると、保存した番号で起きる起動を誤って止め、開かない URL を案内する。
#    見張り番は tests/start_script.rs の the_start_script_checks_the_port_only_when_it_was_named。
# ----------------------------------------------------------------------
$explicitPort = $null
if ($PSBoundParameters.ContainsKey('Port')) {
    $explicitPort = $Port
    $env:PERSONACORE_PORT = "$Port"
}
elseif (-not [string]::IsNullOrWhiteSpace($env:PERSONACORE_PORT)) {
    $raw = $env:PERSONACORE_PORT.Trim()
    $parsed = 0
    if (-not [int]::TryParse($raw, [ref]$parsed) -or $parsed -lt 1 -or $parsed -gt 65535) {
        throw "PERSONACORE_PORT がポート番号として読めません: $raw"
    }
    $explicitPort = $parsed
}

# ----------------------------------------------------------------------
# 2. ポートが空いているか（**番号を明示したときだけ**）
# ----------------------------------------------------------------------
if ($null -ne $explicitPort) {
    if (Test-TcpReachable -TargetHost '127.0.0.1' -TargetPort $explicitPort -TimeoutMs 500) {
        throw "ポート $explicitPort は既に使われています（前回の personacore が残っていませんか）。" +
        " 別のポートで立てるなら -Port を指定してください。"
    }
    $screenHint = "http://127.0.0.1:$explicitPort/"
}
else {
    # ⚠ **事前に見ない。URL も組み立てない**（番号は personacore が決める）。
    #    その番号が使われていれば、personacore が自分でそう言う。画面の URL は、そのログの url= の欄。
    # ⚠ -Upstream を付けた起動は PERSONACORE_UPSTREAMS が変わるので、personacore は保存した番号を読まない。
    $screenHint = if ($PSBoundParameters.ContainsKey('Upstream')) {
        "personacore のログの url= を見てください（-Upstream を付けた起動は保存した番号を読まないので、既定の $defaultPort で待ち受けます）"
    }
    else {
        "personacore のログの url= を見てください（画面の「設定」で保存した番号、無ければ既定の $defaultPort で待ち受けます）"
    }
}

# ----------------------------------------------------------------------
# 3. FE のビルド
# ----------------------------------------------------------------------
if ($SkipWebBuild) {
    Write-Host '[FE ] ビルドを飛ばしました（-SkipWebBuild）'
}
else {
    if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
        throw 'pnpm が見つかりません。FE のビルドに必要です（飛ばすなら -SkipWebBuild）。'
    }

    Push-Location (Join-Path $RepoRoot $WebRoot)
    try {
        if (-not (Test-Path -LiteralPath 'node_modules')) {
            Write-Host '[FE ] 依存を入れています（pnpm install）…'
            pnpm install --frozen-lockfile
            if ($LASTEXITCODE -ne 0) { throw "pnpm install が失敗しました（終了コード $LASTEXITCODE）。" }
        }

        Write-Host '[FE ] ビルドしています（pnpm run build）…'
        pnpm run build
        if ($LASTEXITCODE -ne 0) { throw "pnpm run build が失敗しました（終了コード $LASTEXITCODE）。" }
    }
    finally {
        Pop-Location
    }
}

# ----------------------------------------------------------------------
# 4. 上流の疎通（**止めない**）
# ----------------------------------------------------------------------
# ⚠ 見ているのは「そこに何かが居るか」だけ。
# personacore が要求する能力（OpenAI 互換の /v1/chat/completions 等 / ADR-0038 §決定1）を
# 満たしているかは、実際に会話するまで分からない。
# 宛先の決定と書式の検証は 1. で済ませてある。
#
# ⚠ **-Upstream を渡した起動では、ここで何も言わない**（ADR-0042 §決定6）。
# 宛先を決めるのはプロファイルで、このスクリプトはその中身を読まない。
# ここで PERSONACORE_LLM_ENDPOINT（または既定）を叩くと、
# **プロファイルとは関係の無い宛先について「繋がりません」と言う**ことになり、
# 読み手には「上流が落ちている」という**別の事実**に読める。
# いま何が起きているかは GET /admin/upstream の launch 欄に出る。
if ($PSBoundParameters.ContainsKey('Upstream')) {
    Write-Host "[上流] プロファイル '$Upstream' の宛先は personacore が決めます（ここでは確かめません）"
}
elseif (Test-TcpReachable -TargetHost $upstreamUri.Host -TargetPort $upstreamUri.Port) {
    Write-Host "[上流] $endpoint に繋がりました"
}
else {
    Write-Warning @"
上流 ($endpoint) に繋がりません。
  * 落ちているだけかもしれません（普段は落としてある / docs/04-environment.md）。壊れているとは限りません。
  * このまま起動します。会話はできませんが、キャラクターを作る画面は使えます。
  * 上流はあとから起動してよく、personacore を起動し直す必要はありません。
"@
}

# ----------------------------------------------------------------------
# 5. DB の置き場所を決めて起動
# ----------------------------------------------------------------------
$resolvedDb = if (-not [string]::IsNullOrWhiteSpace($DbPath)) {
    $DbPath
}
elseif (-not [string]::IsNullOrWhiteSpace($env:PERSONACORE_DB_PATH)) {
    $env:PERSONACORE_DB_PATH
}
else {
    Join-Path $RepoRoot $DefaultDbRelative
}

# ⚠ 相対パスは**呼んだときのカレントディレクトリ**を基準に絶対化する。
# サーバはこの後 $RepoRoot へ移ってから走るので、絶対化しないと基準が2つになる ——
# **ディレクトリは手元に作られ、サーバはリポジトリのルート側を探す。**
# 現れ方は「FE とサーバのビルドを全部終えたあとに DB を開けずに落ちる」で、いちばん遅い。
if (-not [System.IO.Path]::IsPathRooted($resolvedDb)) {
    $resolvedDb = [System.IO.Path]::GetFullPath($resolvedDb, $PWD.ProviderPath)
}

# ⚠ ワイルドカードを解釈させない（`[` `]` `*` を含むパス）。
# Test-Path は -LiteralPath なのに New-Item は -Path、と作法が割れていたので
# **親の取り出しも作成も .NET 側へ寄せた**（CreateDirectory は既にあっても失敗しない）。
$dbDir = [System.IO.Path]::GetDirectoryName($resolvedDb)
if ($dbDir) {
    [System.IO.Directory]::CreateDirectory($dbDir) | Out-Null
}
$env:PERSONACORE_DB_PATH = $resolvedDb

Write-Host "[DB  ] $resolvedDb"
Write-Host "[画面] $screenHint"

# ⚠ カレントディレクトリはリポジトリのルートにする。
# PERSONACORE_WEB_DIR を渡していないので、既定の相対パスがここから解決される。
$exitCode = 1
Push-Location $RepoRoot
try {
    # ⚠ **上流を起こすのは、このスクリプトではない**（ADR-0042 §決定6 / 作業単位 F-4b-1）。
    # personacore が --upstream を受け、**待ち受けを開いてから**起こし、
    # **載るのは背景で待ち**、終了時に**自分が控えた子だけ**を止める。
    #
    # **だからここには、起こす行も、待つ行も、止める行も無い。**
    # **同じことをする実装を2つ持たない**——片方が黙って古びる。

    # ⚠ **ブラウザを開くのも、このスクリプトではない**（作業単位 F-4c）。
    # personacore が待ち受けを開いた後に自分で開く（-NoBrowser なら --no-browser を渡してある）。
    # **ここで開くと二重に開く。** 見張り番は tests/start_script.rs の
    # the_start_script_leaves_opening_the_browser_to_the_executable。

    # **旗を渡していなければ `$serverArgs` は空**で、この行は `cargo run` そのものになる。
    cargo run -- @serverArgs
    $exitCode = $LASTEXITCODE
}
finally {
    Pop-Location

    # ⚠ **起こした上流を止めるのも、このスクリプトではない**（ADR-0042 §決定2）。
    # personacore が `serve` を抜けるときに、**自分が控えた子だけ**を止めて終了を待つ。
    # **名前で殺す形は、あちらにも無い**（利用者が別に立てた上流を巻き込まないため）。
    #
    # ⚠ **personacore を強制終了した場合は、あちらの後始末も走らない。**
    # そのときは起こした llama が残るので、自分で止めること（F-9 から変わっていない）。
}

exit $exitCode
