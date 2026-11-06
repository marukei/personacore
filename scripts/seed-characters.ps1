#Requires -Version 7.0
<#
.SYNOPSIS
    JSON に書いたキャラクターと性格ステータスを personacore の管理 API へ投入する。

.DESCRIPTION
    ADR-0008 のとおり、書き込み経路は `/admin/` の HTTP API だけ。このスクリプトは
    その API のクライアントに過ぎず、DB を直接触らない。
    **キャラ管理 FE ができたら捨てる。** 凝った機能を足さないこと。

    1キャラクターにつき 2 回叩く:
      * POST /admin/characters                  … 作成
      * PUT  /admin/characters/{handle}/stats   … 性格ステータスの丸ごと置き換え

    既に同じ識別子のキャラクターが居る場合は作成をスキップし、**性格ステータスの設定だけ**行う
    （更新 API は無い。ペルソナを変えたい場合は DB を作り直すか、更新 API の追加を検討する）。

.PARAMETER Path
    投入する JSON のパス。既定は同ディレクトリの characters.sample.json。

.PARAMETER BaseUrl
    personacore の待ち受け先。既定は http://127.0.0.1:8080
    （PERSONACORE_PORT を変えている場合はここも合わせる）。

.EXAMPLE
    PS> ./scripts/seed-characters.ps1
    PS> ./scripts/seed-characters.ps1 -Path ./scripts/characters.sample.json -BaseUrl http://127.0.0.1:9001

.NOTES
    JSON の形式（characters.sample.json を参照）:

        {
          "characters": [
            {
              "handle": "aoi",
              "display_name": "葵",                // 省略可（省略時は handle を使う）
              "base_persona": "…",
              "output_format_instruction": "…",   // 省略可
              "stat_scale": 5,                    // 省略可（1〜100 / 省略時はサーバの既定）
              "stats": [ { "axis": "パッション", "value": 1 } ]   // 省略可
            }
          ]
        }

    "handle" は **識別子**（`model` フィールドに書く値 / ADR-0004）で、
    使える文字は `[a-z0-9_-]` だけ（ADR-0017）。
    "display_name" は **プロンプトに載る呼び名**（ADR-0011）。分かれているので、
    `aoi` と `aoi-v2` が**どちらもプロンプト上は「葵」**として共存できる。

    "stat_scale" は**そのキャラクターの性格ステータスの上端**（ADR-0015）。
    キャラクターごとに持つので、5 段階の子と 100 段階の子が同じ DB に並んでよい。
    **省略時の既定はサーバが決める。** ここで既定値を書き写さないこと
    （書き写すと既定が2箇所に分かれ、サーバ側を変えても投入経路だけ古びる）。

    "stats" は API の作成本文には含めない（サーバは未知のフィールドを 400 で弾く）。
    値 0 の軸も書いてよい。プロンプトから落とすのはサーバ側の責務。
#>

[CmdletBinding()]
param(
    [string]$Path = (Join-Path $PSScriptRoot 'characters.sample.json'),
    [string]$BaseUrl = 'http://127.0.0.1:8080'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:BaseUrl = $BaseUrl.TrimEnd('/')
$script:Failures = 0

# 管理 API を1回叩く。**HTTP エラーでも例外にせず**、本文（サーバの定数メッセージ）を読む。
function Invoke-Admin {
    param(
        [Parameter(Mandatory)][string]$Method,
        [Parameter(Mandatory)][string]$Route,
        [Parameter(Mandatory)]$Body
    )

    $json = $Body | ConvertTo-Json -Depth 10 -Compress
    # 日本語をそのまま載せるので UTF-8 のバイト列で送る。
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)

    return Invoke-WebRequest -Method $Method -Uri "$script:BaseUrl$Route" `
        -Body $bytes -ContentType 'application/json; charset=utf-8' `
        -SkipHttpErrorCheck
}

# エラー応答からサーバの文言と code を取り出す（取れなければ生の本文）。
function Get-ApiError {
    param([Parameter(Mandatory)]$Response)

    try {
        $body = $Response.Content | ConvertFrom-Json
        return "$($body.error.message) [$($body.error.code)]"
    }
    catch {
        return $Response.Content
    }
}

# オブジェクトにそのプロパティがあるか（StrictMode 下では直接触ると落ちる）。
function Test-Property {
    param([Parameter(Mandatory)]$Object, [Parameter(Mandatory)][string]$Name)
    return $null -ne $Object.PSObject.Properties[$Name]
}

if (-not (Test-Path -LiteralPath $Path)) {
    throw "投入する JSON が見つかりません: $Path"
}

$document = Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json
if (-not (Test-Property $document 'characters')) {
    throw "JSON のトップレベルに characters 配列がありません: $Path"
}

Write-Host "投入先: $script:BaseUrl"
Write-Host "定義  : $Path"

foreach ($character in $document.characters) {
    $handle = $character.handle

    # **作成本文には handle / display_name / base_persona / output_format_instruction だけを載せる。**
    # stats を混ぜるとサーバが 400（未知のフィールド）で弾く。
    $create = @{
        handle        = $handle
        base_persona  = $character.base_persona
    }
    # 省略時はサーバが handle で埋める（ADR-0011）。**ここで勝手に $handle を入れない。**
    # 入れてしまうと「省略時の既定」がスクリプト側とサーバ側の二重管理になる。
    if (Test-Property $character 'display_name') {
        $create['display_name'] = $character.display_name
    }
    if (Test-Property $character 'output_format_instruction') {
        $create['output_format_instruction'] = $character.output_format_instruction
    }
    # 性格ステータスの上端（ADR-0015）。**省略時はサーバの既定に落ちる。**
    # display_name と同じく、ここで既定値を埋めない（既定の二重管理になる）。
    if (Test-Property $character 'stat_scale') {
        $create['stat_scale'] = $character.stat_scale
    }

    $response = Invoke-Admin -Method 'POST' -Route '/admin/characters' -Body $create

    # **switch は使わない。** switch の中の continue は外側の foreach ではなく
    # switch 自身に効くため、意図せず後続処理が走る。
    $exists = $false
    if ($response.StatusCode -eq 201) {
        Write-Host "[作成] $handle"
        $exists = $true
    }
    elseif ($response.StatusCode -eq 409) {
        # **409 は「まったく同じ綴りの行が既にある」だけ**（作業単位 0004b-2）。
        # 以前はここで conflicting_unnormalized_name（ADR-0004 のゴースト行との衝突）を
        # 分けていたが、正規化キーの索引ごと落としたのでその応答は返らない。
        $detail = Get-ApiError $response
        Write-Warning "[既存] $handle : $detail（ペルソナは更新されません）"
        $exists = $true
    }
    else {
        Write-Error "[失敗] $handle : HTTP $($response.StatusCode) $(Get-ApiError $response)" -ErrorAction Continue
        $script:Failures++
    }

    if (-not $exists) {
        continue
    }

    if (-not (Test-Property $character 'stats')) {
        continue
    }

    # **丸ごと置き換え。** ここに書かれていない軸は消える。
    $route = "/admin/characters/$([uri]::EscapeDataString($handle))/stats"
    $response = Invoke-Admin -Method 'PUT' -Route $route -Body @{ stats = @($character.stats) }

    if ($response.StatusCode -eq 200) {
        $stats = ($response.Content | ConvertFrom-Json).stats
        $summary = ($stats | ForEach-Object { "$($_.axis)=$($_.value)" }) -join ' '
        Write-Host "[性格] $handle : $summary"
    }
    else {
        Write-Error "[失敗] $handle の性格ステータス : HTTP $($response.StatusCode) $(Get-ApiError $response)" `
            -ErrorAction Continue
        $script:Failures++
    }
}

Write-Host ''
Write-Host '--- 現在のキャラクター一覧（GET /v1/models）---'
$models = Invoke-WebRequest -Method 'GET' -Uri "$script:BaseUrl/v1/models" -SkipHttpErrorCheck
if ($models.StatusCode -eq 200) {
    ($models.Content | ConvertFrom-Json).data | ForEach-Object { Write-Host "  $($_.id)" }
}
else {
    Write-Error "モデル一覧を取得できませんでした: HTTP $($models.StatusCode)" -ErrorAction Continue
    $script:Failures++
}

if ($script:Failures -gt 0) {
    throw "$script:Failures 件の投入に失敗しました。"
}
