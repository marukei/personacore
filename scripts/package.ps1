#Requires -Version 7.0
<#
.SYNOPSIS
    配布物（実行ファイル1本と、それを詰めた zip）を作る。

.DESCRIPTION
    作業単位 F-4a / ADR-0041 §決定5（5 は F-4d / Issue origin#25、4 は O-1 / Issue origin#48）。
    やることは5つだけで、**順番が本質**:

      1. 画面をビルドする（web/dist）
      2. cargo build --release
      3. 出来た実行ファイルを dist/ へ複製する
      4. 第三者の著作権表示（scripts/THIRD-PARTY-NOTICES.txt）を作り直す（scripts/notices.ps1）
      5. 複製した実行ファイルと利用者向け README と 4 の表示と、リポジトリの根のライセンスの文書と
         AI 協働の開示の文書を dist/personacore-<版>-windows-x64.zip に詰める（scripts/package-zip.ps1。
         ライセンスの文書は O-2 / Issue origin#49、開示の文書は O-3 / Issue origin#50）

    ⚠ **順番が本質** —— 画面はコンパイル時に実行ファイルへ埋め込まれる
    （ADR-0041 §決定2）。**FE を先にビルドしないと、古い（または空の）画面が入る。**
    入っても起動は成功するので、**気づく手掛かりは起動ログの ui_files だけ**である。

    ⚠ このスクリプトは**開発機で動かすもの**である。cargo と pnpm と cargo-about を要求する
    （cargo-about は 4 で使う。cargo install --locked cargo-about --features cli）。
    **配布された側には何も要らない** —— 出来た実行ファイル1本で立つ。

    ⚠ 上流（LM Studio 等）は起動しない。ここは何も起動しない。

.PARAMETER SkipWebBuild
    画面のビルドを飛ばす（既に web/dist がある状態での作り直し用）。
    ⚠ cargo は飛ばさない。古い実行ファイルを配るほうが危ないため。
    ⚠ この旗を渡すと、**実行ファイルに入る画面は「いま web/dist に在るもの」**になる。

.PARAMETER OutDir
    配布物の置き場所。省略時はリポジトリ直下の dist/（.gitignore 済み）。
    ⚠ **相対パスは「このコマンドを打ったディレクトリ」を基準に解決する**
    （scripts は別の場所から呼ばれうるので、絶対化しないと基準が2つに割れる）。

.EXAMPLE
    PS> pwsh ./scripts/package.ps1
    PS> pwsh ./scripts/package.ps1 -SkipWebBuild

    ⚠ `pwsh` を頭に付けて**別プロセスで**起動すること（README と同じ形）。

.NOTES
    ⚠ Mac 版は、このスクリプトの写しが要る（ADR-0041 §決定5）。
    OS ごとの分岐をどう持つかは F-4b で決める。
#>

[CmdletBinding()]
param(
    [switch]$SkipWebBuild,

    [string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# native コマンドの失敗は $LASTEXITCODE で自分で見る（版によって既定が違うため固定する）。
$PSNativeCommandUseErrorActionPreference = $false

# ----------------------------------------------------------------------
# 置き場所。**すべて絶対パスに直してから使う。**
# ⚠ 引数の名前（$SkipWebBuild / $OutDir）を局所変数に使い回さないこと。
#    PowerShell の変数名は大文字小文字を区別せず、param(...) に書いた型は
#    その名前への代入すべてに効く（F-9 の2周目で実際に事故った）。
#    見張り番は tests/start_script.rs の
#    no_script_parameter_is_reused_as_a_local_variable。
# ----------------------------------------------------------------------
$RepoRoot = Split-Path -Parent $PSScriptRoot

# 画面の置き場所（リポジトリのルートから）。
$WebRoot = 'web'

# 配布物の既定の置き場所（リポジトリのルートから）。
$DefaultOutRelative = 'dist'

# cargo が release の実行ファイルを置く場所（リポジトリのルートから）。
$ReleaseBinRelative = 'target/release/personacore.exe'

# 配布用の zip を組むスクリプトと、zip に入れる利用者向け README（リポジトリのルートから）。
# README は zip の中では README.md になる（作業単位 F-4d / Issue origin#25）。
$ZipScriptRelative = 'scripts/package-zip.ps1'
$DistReadmeRelative = 'scripts/README-dist.md'
$zipScript = Join-Path $RepoRoot $ZipScriptRelative
$distReadme = Join-Path $RepoRoot $DistReadmeRelative

# 第三者の著作権表示を作り直すスクリプトと、その書き出す先（zip の中では THIRD-PARTY-NOTICES.txt）。
# **書き出す先は notices.ps1 の $NoticesRelative と同じ綴り**であること
# （tests/start_script.rs の the_package_script_zips_the_copied_executable_last が突き合わせる）。
$NoticesScriptRelative = 'scripts/notices.ps1'
$DistNoticesRelative = 'scripts/THIRD-PARTY-NOTICES.txt'
$noticesScript = Join-Path $RepoRoot $NoticesScriptRelative
$distNotices = Join-Path $RepoRoot $DistNoticesRelative

$resolvedOut = if (-not [string]::IsNullOrWhiteSpace($OutDir)) {
    [System.IO.Path]::GetFullPath($OutDir, (Get-Location).Path)
}
else {
    Join-Path $RepoRoot $DefaultOutRelative
}

# ----------------------------------------------------------------------
# 道具
# ----------------------------------------------------------------------

# 直前の native コマンドが成功したことを確かめる。**失敗したら、ここで止まる。**
function Assert-LastExitCode {
    param(
        [Parameter(Mandatory)][string]$What
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$What が失敗しました（終了コード $LASTEXITCODE）。"
    }
}

# ----------------------------------------------------------------------
# 0. 道具が揃っていること（**画面とリリースビルドより先に**）
#    cargo-about は 4 で使う。確かめずに進むと、数分のリリースビルドを終えてから止まる。
#    見張り番は tests/start_script.rs の the_package_script_checks_for_cargo_about_before_building。
# ----------------------------------------------------------------------
$aboutVersion = cargo about --version 2>$null
if ($LASTEXITCODE -ne 0) {
    throw 'cargo-about が見つかりません。第三者の著作権表示（手順4）を作れないので、ビルドの前に止めます。cargo install --locked cargo-about --features cli で入れてください（確かめた版は 0.9.2）。'
}
Write-Host "[道具] $aboutVersion"

# ----------------------------------------------------------------------
# 1. 画面をビルドする（**cargo より先**。ここが順番の本質）
# ----------------------------------------------------------------------
$webPath = Join-Path $RepoRoot $WebRoot

if ($SkipWebBuild) {
    Write-Host '[FE ] -SkipWebBuild: 画面のビルドを飛ばします（いまの web/dist が実行ファイルに入ります）'
}
else {
    if (-not (Test-Path -LiteralPath (Join-Path $webPath 'node_modules'))) {
        Write-Host '[FE ] node_modules が無いので pnpm install から始めます'
        Push-Location $webPath
        try {
            pnpm install --frozen-lockfile
            Assert-LastExitCode -What 'pnpm install'
        }
        finally {
            Pop-Location
        }
    }

    Write-Host '[FE ] pnpm run build'
    Push-Location $webPath
    try {
        pnpm run build
        Assert-LastExitCode -What 'pnpm run build'
    }
    finally {
        Pop-Location
    }
}

# ----------------------------------------------------------------------
# 2. cargo build --release（**画面を埋め込むのはここ**）
# ----------------------------------------------------------------------
Write-Host '[EXE] cargo build --release'
Push-Location $RepoRoot
try {
    cargo build --release
    Assert-LastExitCode -What 'cargo build --release'
}
finally {
    Pop-Location
}

# ----------------------------------------------------------------------
# 3. 配布物を置く
# ----------------------------------------------------------------------
$builtBin = Join-Path $RepoRoot $ReleaseBinRelative
if (-not (Test-Path -LiteralPath $builtBin)) {
    throw "$ReleaseBinRelative が見つかりません（cargo build --release は成功したはずなのに）。"
}

if (-not (Test-Path -LiteralPath $resolvedOut)) {
    New-Item -ItemType Directory -Path $resolvedOut | Out-Null
}
$outBin = Join-Path $resolvedOut (Split-Path -Leaf $builtBin)
Copy-Item -LiteralPath $builtBin -Destination $outBin -Force

# ----------------------------------------------------------------------
# 4. 第三者の著作権表示を作り直す（**zip より先に、毎回**）
#    依存が増減していれば中身が変わる（変わったら notices.ps1 が警告する。コミットすること）。
#    **古い表示を詰めないために、飛ばす旗は設けない。**
# ----------------------------------------------------------------------
& $noticesScript

# ----------------------------------------------------------------------
# 5. 配布用の zip（**複製した後の exe から組む**。exe と README と表示と、
#    リポジトリの根のライセンスの文書5つと AI 協働の開示の文書だけを詰める）
#    名前の版は exe 自身の --version から作る（package-zip.ps1 の説明）。
# ----------------------------------------------------------------------
& $zipScript -ExePath $outBin -ReadmePath $distReadme -NoticesPath $distNotices -LicenseDir $RepoRoot -OutDir $resolvedOut

$sizeMb = [math]::Round((Get-Item -LiteralPath $outBin).Length / 1MB, 1)
Write-Host ''
Write-Host "[出来] $outBin （$sizeMb MB）"
Write-Host '[出来] この1本だけを配れます（利用者向けの README と第三者の著作権表示とライセンスの文書と AI 協働の開示の文書と一緒に詰めたのが上の zip）。動かす機に Rust も pnpm も要りません。'
Write-Host '[出来] DB は OS の作法の場所に出来ます（PERSONACORE_DB_PATH で変えられます）。'
Write-Host '[出来] まっさらから遊び直すなら --fresh（消さずに退避します）。'
