#Requires -Version 7.0
<#
.SYNOPSIS
    配布用の zip を作る（中身は実行ファイルと利用者向け README と第三者の著作権表示と、ライセンスの文書5つと、
    AI 協働の開示の文書だけ）。

.DESCRIPTION
    作業単位 F-4d / Issue origin#25。**package.ps1 の最後の段**で、単独で呼ぶものではない
    （package.ps1 が「画面 → cargo build --release → 複製」の後に、複製した exe を渡して呼ぶ）。

      <OutDir>/personacore-<版>-windows-x64.zip
        ├─ personacore.exe           （-ExePath のバイト列そのもの）
        ├─ README.md                 （-ReadmePath のバイト列そのもの。元は scripts/README-dist.md）
        ├─ THIRD-PARTY-NOTICES.txt   （-NoticesPath のバイト列そのもの。元は scripts/THIRD-PARTY-NOTICES.txt。
        │                             作業単位 O-1 / Issue origin#48）
        ├─ LICENSE・LICENSE-MIT・LICENSE-APACHE・LICENSE-ACCRL.md・LICENSE-ACCRL.en.md
        │                            （-LicenseDir の中の同じ名前のファイルのバイト列そのもの。
        │                             package.ps1 はリポジトリの根を渡す。作業単位 O-2 / Issue origin#49）
        └─ NOTICE-AI-COLLABORATION.md（AI 協働の開示の文書。LICENSE が名指す。ライセンスの文書と同じく
                                      -LicenseDir の中の同じ名前のファイルのバイト列そのもの。
                                      作業単位 O-3 / Issue origin#50）

    ⚠ **中身はこの9つだけ。** exe の隣や出力先に何が在っても詰めない
    （開発用の DB・upstreams.json・シンボルを配らないため）。
    **詰める項目は下の $ZipEntries で名指ししたもの**で、ディレクトリを丸ごと詰める形を採らない
    （-LicenseDir も、名指した6つの名前だけを読む）。
    見張り番は tests/distribution.rs の the_zip_holds_exactly_the_executable_the_readme_the_notices_and_the_licenses。

    # <版> の決め方

    **exe 自身に `--version` を聞いて作る。** `personacore 0.1.0 (3858184)` → `0.1.0-3858184`
    （Cargo.toml の版と git describe を `-` でつなぐ）。
    **別のところ（Cargo.toml / git）から読まない** —— 読むと、zip の名前と中の exe の名乗りが
    食い違いうる（そうなると、どちらが配布物を正しく名乗っているか分からなくなる）。

    **止まるもの:**
      - exe が `--version` に失敗した／名乗りの形が違う
      - 版の綴りがファイル名に使えない（タグに `/` を含む等）
      - exe が x64 でない（名前が windows-x64 を名乗るため。PE ヘッダの機種で見る）

    **止まらないもの:** 括弧の中が `unknown`（git の無い機で組んだ）。警告だけ出して作る
    —— exe がそう名乗るので、zip の名前もそう名乗るのが食い違わない形である。

    # 置き換え

    同じ名前の zip が在れば置き換える。**途中で失敗したら、途中のファイルを消して止まる**
    （書き上がってから同じディレクトリの中で改名する）。

.PARAMETER ExePath
    詰める実行ファイル。package.ps1 は**複製した後のもの**（dist/personacore.exe）を渡す。

.PARAMETER ReadmePath
    詰める利用者向け README（zip の中では README.md になる）。

.PARAMETER NoticesPath
    詰める第三者の著作権表示（zip の中では THIRD-PARTY-NOTICES.txt になる）。
    package.ps1 は scripts/notices.ps1 で作り直した直後のものを渡す。

.PARAMETER LicenseDir
    詰めるライセンスの文書（LICENSE・LICENSE-MIT・LICENSE-APACHE・LICENSE-ACCRL.md・LICENSE-ACCRL.en.md）と、
    LICENSE が名指す AI 協働の開示の文書（NOTICE-AI-COLLABORATION.md）を持つディレクトリ。
    zip の中では同じ名前になる。package.ps1 はリポジトリの根を渡す。

.PARAMETER OutDir
    zip の置き場所。無ければ作る。
    ⚠ **相対パスは「このコマンドを打ったディレクトリ」を基準に解決する**（package.ps1 と同じ）。
#>

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ExePath,

    [Parameter(Mandatory)][string]$ReadmePath,

    [Parameter(Mandatory)][string]$NoticesPath,

    [Parameter(Mandatory)][string]$LicenseDir,

    [Parameter(Mandatory)][string]$OutDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# native コマンドの失敗は $LASTEXITCODE で自分で見る（package.ps1 と同じ）。
$PSNativeCommandUseErrorActionPreference = $false

# ----------------------------------------------------------------------
# ⚠ 引数の名前（$ExePath / $ReadmePath / $NoticesPath / $LicenseDir / $OutDir）を局所変数に使い回さないこと。
#    PowerShell の変数名は大文字小文字を区別しない（package.ps1 の同じ注記）。
#    見張り番は tests/start_script.rs の no_script_parameter_is_reused_as_a_local_variable。
# ----------------------------------------------------------------------
$here = (Get-Location).Path
$exeFull = [System.IO.Path]::GetFullPath($ExePath, $here)
$readmeFull = [System.IO.Path]::GetFullPath($ReadmePath, $here)
$noticesFull = [System.IO.Path]::GetFullPath($NoticesPath, $here)
$licenseDirFull = [System.IO.Path]::GetFullPath($LicenseDir, $here)
$zipDir = [System.IO.Path]::GetFullPath($OutDir, $here)

# ライセンスの文書の名前（zip の中でも同じ名前 / 作業単位 O-2 / Issue origin#49）。
# 中身と式の綴りの見張りは tests/license.rs。
$LicenseNames = @(
    'LICENSE'
    'LICENSE-MIT'
    'LICENSE-APACHE'
    'LICENSE-ACCRL.md'
    'LICENSE-ACCRL.en.md'
)

# LICENSE が名指す AI 協働の開示の文書（zip の中でも同じ名前 / 作業単位 O-3 / Issue origin#50）。
# ライセンスの文書と同じディレクトリ（-LicenseDir）から読む。中身（承認した版の指紋）の見張りは tests/license.rs。
$DisclosureName = 'NOTICE-AI-COLLABORATION.md'

# zip の中の名前 → 元のファイル。**詰めるのはここで名指したものだけ。**
$ZipEntries = [ordered]@{
    'personacore.exe'         = $exeFull
    'README.md'               = $readmeFull
    'THIRD-PARTY-NOTICES.txt' = $noticesFull
}
foreach ($licenseName in $LicenseNames) {
    $ZipEntries[$licenseName] = Join-Path $licenseDirFull $licenseName
}
$ZipEntries[$DisclosureName] = Join-Path $licenseDirFull $DisclosureName

# zip の名前の頭と尻（間に <版> が入る）。
$ZipNamePrefix = 'personacore-'
$ZipNameSuffix = '-windows-x64.zip'

# `--version` の名乗りの形（src/server/fresh.rs の VERSION_LINE）。
$VersionLinePattern = '^personacore (?<package>[^\s()]+) \((?<describe>[^\s()]+)\)$'

# ファイル名に入れてよい版の綴り。
$SafeVersionPattern = '^[0-9A-Za-z._-]+$'

# PE ヘッダの機種（IMAGE_FILE_MACHINE_AMD64）。
$MachineX64 = 0x8664

# ----------------------------------------------------------------------
# 道具
# ----------------------------------------------------------------------

# PE ヘッダの機種を読む。**PE でなければ $null。**
function Get-PeMachine {
    param(
        [Parameter(Mandatory)][string]$Path
    )

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $reader = [System.IO.BinaryReader]::new($stream)
        if ($stream.Length -lt 0x40) {
            return $null
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        if ($peOffset + 6 -gt $stream.Length) {
            return $null
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550) {
            # 'PE\0\0'
            return $null
        }
        return [int]$reader.ReadUInt16()
    }
    finally {
        $stream.Dispose()
    }
}

# ----------------------------------------------------------------------
# 1. 詰めるものが在ること
# ----------------------------------------------------------------------
foreach ($entry in $ZipEntries.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "zip に詰める $($entry.Key) の元が見つかりません: $($entry.Value)"
    }
}

# ----------------------------------------------------------------------
# 2. x64 の実行ファイルであること（**起こす前に見る**）
# ----------------------------------------------------------------------
$machine = Get-PeMachine -Path $exeFull
if ($machine -ne $MachineX64) {
    $shown = if ($null -eq $machine) { 'PE ではない' } else { '0x{0:X4}' -f $machine }
    throw "x64 の実行ファイルではありません（機種: $shown）。zip の名前が windows-x64 を名乗れません: $exeFull"
}

# ----------------------------------------------------------------------
# 3. 版は exe 自身に聞く
# ----------------------------------------------------------------------
$versionOutput = & $exeFull --version
if ($LASTEXITCODE -ne 0) {
    throw "実行ファイルが --version に失敗しました（終了コード $LASTEXITCODE）: $exeFull"
}
$versionLine = (@($versionOutput) -join "`n").Trim()
$parsed = [regex]::Match($versionLine, $VersionLinePattern)
if (-not $parsed.Success) {
    throw "実行ファイルの --version の名乗りを読めません: '$versionLine'"
}
$describe = $parsed.Groups['describe'].Value
$version = "$($parsed.Groups['package'].Value)-$describe"
if ($version -notmatch $SafeVersionPattern) {
    throw "版の綴りがファイル名に使えません: '$version'（--version の名乗り: '$versionLine'）"
}
if ($describe -eq 'unknown') {
    Write-Warning "実行ファイルがどのコミットか名乗っていません（'$versionLine'）。git の無い場所で組んだ exe です。"
}

# ----------------------------------------------------------------------
# 4. 組む（書き上がってから改名する）
# ----------------------------------------------------------------------
if (-not (Test-Path -LiteralPath $zipDir)) {
    New-Item -ItemType Directory -Path $zipDir | Out-Null
}
$zipPath = Join-Path $zipDir "$ZipNamePrefix$version$ZipNameSuffix"
$partialPath = "$zipPath.partial"
if (Test-Path -LiteralPath $partialPath) {
    Remove-Item -LiteralPath $partialPath -Force
}

try {
    $archive = [System.IO.Compression.ZipFile]::Open($partialPath, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($entry in $ZipEntries.GetEnumerator()) {
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $archive,
                $entry.Value,
                $entry.Key,
                [System.IO.Compression.CompressionLevel]::Optimal
            ) | Out-Null
        }
    }
    finally {
        $archive.Dispose()
    }
    Move-Item -LiteralPath $partialPath -Destination $zipPath -Force
}
catch {
    if (Test-Path -LiteralPath $partialPath) {
        Remove-Item -LiteralPath $partialPath -Force
    }
    throw
}

$sizeMb = [math]::Round((Get-Item -LiteralPath $zipPath).Length / 1MB, 1)
Write-Host "[zip ] $zipPath （$sizeMb MB / $($ZipEntries.Keys -join ' + ')）"
