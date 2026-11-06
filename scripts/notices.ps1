#Requires -Version 7.0
<#
.SYNOPSIS
    配布物に同梱する第三者の著作権表示（scripts/THIRD-PARTY-NOTICES.txt）を作り直す。

.DESCRIPTION
    作業単位 O-1 / Issue origin#48 / ADR-0048 §4。

    Rust は依存の crate を実行ファイルへ静的に組み込み、画面（web/dist）は npm のパッケージの
    コードを束ねて実行ファイルへ埋め込む。MIT / Apache-2.0 などは「複製物に著作権表示と
    ライセンス文を含めよ」と求めるので、それを1つのファイルに集めて zip に同梱する。

      1. Rust:  cargo about generate（cargo-about / scripts/notices/about.toml と about.hbs）
      2. 画面:  vite build --sourcemap を target/notices-web へ別に回し、
                **ソースマップの sources に現れた node_modules のパッケージだけ**を拾う
                ＋ 束ねる側の道具のうち、コードが成果物に入るもの（下の $ToolsInTheBundle）
      3. 見出しと 1・2 をつないで scripts/THIRD-PARTY-NOTICES.txt に書く（UTF-8 / LF）

    **生成物はコミットする。** tests/notices.rs が Cargo.lock（cargo tree）と
    web/pnpm-lock.yaml に突き合わせるので、依存が増減して古びたら cargo test が赤くなる。
    package.ps1 は zip を組む前に毎回これを呼ぶ（配る zip の表示は常に作り直したもの）。

    # 画面の側で「成果物に入るもの」の決め方

    **ビルドの道具（Vite・Tailwind・lightningcss・svelte のコンパイラ等）は、
    それ自身のコードが成果物に入らない限り対象外**である。入るかどうかは推測で決めず、
    **本番と同じ設定のビルドのソースマップ**で決める（sourcemap の有無は束ねる中身を変えない）。

    ソースマップに現れないのに成果物に入るものが2種類ある（O-1 で確かめた）:
      - **modulepreload の polyfill**（dist の JS の先頭の即時関数）。Vite の組み込みの
        プラグイン（vite:modulepreload-polyfill）が rolldown の組み込みを通して差し込む。
        → vite（コアのライセンスの部分だけ）と rolldown を載せる。
      - **Tailwind が生成する CSS**（@layer properties の @property --tw-* と、
        utilities の規則の本体）。app.css が tailwindcss/theme.css と utilities.css を取り込む。
        → tailwindcss を載せる。
    これらは名前で $ToolsInTheBundle に書く。**⚠ 帳簿である** —— 別の道具のコードが
    成果物に入るようになっても、ここへ足さないと載らない（見張り番は無い）。

    # 要るもの

    cargo-about（cargo install --locked cargo-about --features cli）・pnpm・web/node_modules。
    **ネットワークは使わない**（cargo about の --frozen）。ライセンスのファイルを同梱していない
    crate は、cargo-about が SPDX の標準文で補う（その場合、著作権者の行は `<year> <owner>` の形の
    ひな形になる。代わりに crate の authors を名前の下に載せている）。

.EXAMPLE
    PS> pwsh ./scripts/notices.ps1
#>

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# native コマンドの失敗は $LASTEXITCODE で自分で見る（package.ps1 と同じ）。
$PSNativeCommandUseErrorActionPreference = $false

# ----------------------------------------------------------------------
# 置き場所（リポジトリのルートから）。**すべて絶対パスに直してから使う。**
# ----------------------------------------------------------------------
$RepoRoot = Split-Path -Parent $PSScriptRoot

# 生成物（コミットする。package.ps1 が zip に詰める）。
$NoticesRelative = 'scripts/THIRD-PARTY-NOTICES.txt'

# cargo-about の設定とひな形。
$AboutConfigRelative = 'scripts/notices/about.toml'
$AboutTemplateRelative = 'scripts/notices/about.hbs'

# ライセンスのファイルを「そのライセンスの文」と認める確からしさの下限（cargo-about の既定は 0.8）。
# 0.8 だと aws-lc-sys の LICENSE（BoringSSL / OpenSSL の表示を全部含む合本）が認められず、
# SPDX の標準文に置き換わって、中の著作権表示が落ちる（O-1 で実測）。
$AboutThreshold = '0.5'

# 画面のソース。
$WebRoot = 'web'

# 画面のソースマップを取るための、本番とは別の出力先（target/ の下なので追跡されない）。
$WebProbeRelative = 'target/notices-web'

# Rust の部分の一時の出力。
$RustPartRelative = 'target/notices-rust.txt'

# **束ねる側の道具のうち、そのコードが成果物に入るもの**（上の説明）。
# CutBefore: ライセンスのファイルのうち、この行より前だけを載せる（vite の LICENSE.md は
# 後ろに「vite 自身が束ねた依存」のライセンスを 100 KB 以上抱えるが、成果物に入るのは vite 自身のコード）。
$ToolsInTheBundle = @(
    [ordered]@{ Name = 'vite'; CutBefore = '# Licenses of bundled dependencies' }
    [ordered]@{ Name = 'rolldown'; CutBefore = $null }
    [ordered]@{ Name = 'tailwindcss'; CutBefore = $null }
)

# ライセンスのファイルの名前（大文字小文字を問わない）。
$LicenseFilePattern = '^(licen[cs]e|copying)([-._].*)?$'

$Separator = '=' * 80

$Header = @'
THIRD-PARTY NOTICES
第三者のソフトウェアの著作権表示とライセンス

personacore.exe は、次の第三者のソフトウェアを含んでいます。
それぞれの著作権表示とライセンスの全文を、この下に載せます。

  [crate]  Rust のライブラリ（実行ファイルに組み込まれているもの）
  [npm]    画面の JavaScript / CSS に入っているもの

personacore.exe contains the third-party software listed below.
Their copyright notices and license texts follow.
'@

$noticesPath = Join-Path $RepoRoot $NoticesRelative
$aboutConfig = Join-Path $RepoRoot $AboutConfigRelative
$aboutTemplate = Join-Path $RepoRoot $AboutTemplateRelative
$webPath = Join-Path $RepoRoot $WebRoot
$webProbe = Join-Path $RepoRoot $WebProbeRelative
$rustPart = Join-Path $RepoRoot $RustPartRelative
$pnpmStore = Join-Path $webPath 'node_modules/.pnpm'

# ----------------------------------------------------------------------
# 道具
# ----------------------------------------------------------------------

function Assert-LastExitCode {
    param(
        [Parameter(Mandatory)][string]$What
    )

    if ($LASTEXITCODE -ne 0) {
        throw "$What が失敗しました（終了コード $LASTEXITCODE）。"
    }
}

# 改行を LF に揃え、末尾の空白を落とす。
function ConvertTo-LfText {
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string]$Text
    )

    return ($Text -replace "`r`n", "`n" -replace "`r", "`n").TrimEnd()
}

# パッケージのディレクトリから、名前・版・ライセンス・ライセンスの文を読む。
# **ライセンスのファイルが無ければ止まる**（黙って載せ落とさない）。
function Read-WebPackage {
    param(
        [Parameter(Mandatory)][string]$Directory,

        [AllowNull()][string]$CutBefore
    )

    $manifest = Get-Content -LiteralPath (Join-Path $Directory 'package.json') -Raw | ConvertFrom-Json
    $licenseProperty = $manifest.PSObject.Properties['license']
    $licenseName = if ($null -ne $licenseProperty -and $licenseProperty.Value -is [string]) {
        $licenseProperty.Value
    }
    else {
        'UNKNOWN'
    }
    $repositoryProperty = $manifest.PSObject.Properties['repository']
    $repository = $null
    if ($null -ne $repositoryProperty) {
        if ($repositoryProperty.Value -is [string]) {
            $repository = $repositoryProperty.Value
        }
        elseif ($null -ne $repositoryProperty.Value.PSObject.Properties['url']) {
            $repository = $repositoryProperty.Value.url
        }
    }

    $files = @(Get-ChildItem -LiteralPath $Directory -File |
            Where-Object { $_.Name -match $LicenseFilePattern } |
            Sort-Object Name)
    if ($files.Count -eq 0) {
        throw "npm のパッケージ $($manifest.name) $($manifest.version) にライセンスのファイルがありません: $Directory"
    }

    $texts = foreach ($file in $files) {
        $body = ConvertTo-LfText -Text (Get-Content -LiteralPath $file.FullName -Raw)
        if (-not [string]::IsNullOrEmpty($CutBefore)) {
            $at = $body.IndexOf("`n$CutBefore")
            if ($at -ge 0) {
                $body = $body.Substring(0, $at).TrimEnd()
            }
        }
        $body
    }

    return [pscustomobject]@{
        Name       = [string]$manifest.name
        Version    = [string]$manifest.version
        License    = $licenseName
        Repository = $repository
        Text       = ($texts -join "`n`n")
    }
}

# pnpm の置き場所から、そのパッケージのディレクトリを1つだけ探す。
# **版が2つ以上あったら止まる**（どちらが成果物に入ったか、ここでは決められない）。
function Find-PnpmPackageDirectory {
    param(
        [Parameter(Mandatory)][string]$Name
    )

    $encoded = $Name -replace '/', '+'
    $candidates = @(Get-ChildItem -LiteralPath $pnpmStore -Directory |
            Where-Object { $_.Name.StartsWith("$encoded@") } |
            ForEach-Object { Join-Path $_.FullName "node_modules/$Name" } |
            Where-Object { Test-Path -LiteralPath (Join-Path $_ 'package.json') })
    $versions = @($candidates | ForEach-Object {
            (Get-Content -LiteralPath (Join-Path $_ 'package.json') -Raw | ConvertFrom-Json).version
        } | Sort-Object -Unique)
    if ($versions.Count -ne 1) {
        throw "npm のパッケージ $Name の版を1つに決められません（見つかった版: $($versions -join ', ')）。"
    }
    return $candidates[0]
}

# ----------------------------------------------------------------------
# 1. Rust（cargo-about）
# ----------------------------------------------------------------------
Write-Host '[著作] cargo about generate'
Push-Location $RepoRoot
try {
    cargo about generate --frozen --threshold $AboutThreshold -c $aboutConfig -o $rustPart $aboutTemplate
    Assert-LastExitCode -What 'cargo about generate（cargo install --locked cargo-about --features cli で入ります）'
}
finally {
    Pop-Location
}
$rustText = ConvertTo-LfText -Text (Get-Content -LiteralPath $rustPart -Raw)

# ----------------------------------------------------------------------
# 2. 画面（ソースマップに現れたパッケージ ＋ 成果物に入る道具）
# ----------------------------------------------------------------------
if (-not (Test-Path -LiteralPath $pnpmStore)) {
    throw "web/node_modules がありません。web で pnpm install --frozen-lockfile を先に回してください。"
}

Write-Host '[著作] vite build --sourcemap（本番の web/dist とは別の場所へ）'
Push-Location $webPath
try {
    pnpm exec vite build --sourcemap --outDir $webProbe --emptyOutDir --logLevel warn
    Assert-LastExitCode -What 'vite build --sourcemap'
}
finally {
    Pop-Location
}

$maps = @(Get-ChildItem -LiteralPath $webProbe -Recurse -File -Filter '*.map')
if ($maps.Count -eq 0) {
    throw "画面のソースマップが1つもできませんでした: $webProbe"
}

$packageDirectories = [System.Collections.Generic.SortedSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($map in $maps) {
    $json = Get-Content -LiteralPath $map.FullName -Raw | ConvertFrom-Json
    $rootProperty = $json.PSObject.Properties['sourceRoot']
    $base = if ($null -ne $rootProperty -and -not [string]::IsNullOrEmpty($rootProperty.Value)) {
        [System.IO.Path]::GetFullPath($rootProperty.Value, $map.DirectoryName)
    }
    else {
        $map.DirectoryName
    }
    foreach ($source in $json.sources) {
        $normalized = ([string]$source) -replace '\\', '/'
        $marker = 'node_modules/'
        $at = $normalized.LastIndexOf($marker)
        if ($at -lt 0) {
            continue
        }
        $parts = $normalized.Substring($at + $marker.Length).Split('/')
        $nameLength = if ($parts[0].StartsWith('@')) { 2 } else { 1 }
        $relative = $normalized.Substring(0, $at + $marker.Length) + ($parts[0..($nameLength - 1)] -join '/')
        [void]$packageDirectories.Add([System.IO.Path]::GetFullPath($relative, $base))
    }
}
if ($packageDirectories.Count -eq 0) {
    throw '画面のソースマップに npm のパッケージが1つも現れませんでした（拾い方が壊れている疑い）。'
}

$webPackages = [ordered]@{}
foreach ($directory in $packageDirectories) {
    $package = Read-WebPackage -Directory $directory -CutBefore $null
    $webPackages["$($package.Name)@$($package.Version)"] = $package
}
foreach ($tool in $ToolsInTheBundle) {
    $package = Read-WebPackage -Directory (Find-PnpmPackageDirectory -Name $tool.Name) -CutBefore $tool.CutBefore
    $webPackages["$($package.Name)@$($package.Version)"] = $package
}

$webBlocks = foreach ($key in ($webPackages.Keys | Sort-Object)) {
    $package = $webPackages[$key]
    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add($Separator)
    $lines.Add("[license] $($package.License)")
    $lines.Add("[npm] $($package.Name) $($package.Version)")
    if (-not [string]::IsNullOrEmpty($package.Repository)) {
        $lines.Add("        $($package.Repository)")
    }
    $lines.Add('[text]')
    $lines.Add($package.Text)
    $lines -join "`n"
}

# ----------------------------------------------------------------------
# 3. つないで書く（UTF-8 / BOM 無し / LF）
# ----------------------------------------------------------------------
$body = @(
    (ConvertTo-LfText -Text $Header)
    $rustText
    ($webBlocks -join "`n`n")
) -join "`n`n"
$body = "$body`n"

$before = if (Test-Path -LiteralPath $noticesPath) {
    ConvertTo-LfText -Text (Get-Content -LiteralPath $noticesPath -Raw)
}
else {
    $null
}
[System.IO.File]::WriteAllText($noticesPath, $body, [System.Text.UTF8Encoding]::new($false))

$crateCount = ([regex]::Matches($body, '(?m)^\[crate\] ')).Count
$npmCount = ([regex]::Matches($body, '(?m)^\[npm\] ')).Count
Write-Host "[著作] $NoticesRelative （crate $crateCount 件 / npm $npmCount 件）"
if ($before -ne $body.TrimEnd()) {
    Write-Warning "$NoticesRelative の中身が変わりました。依存が増減したなら、この変更をコミットしてください。"
}
