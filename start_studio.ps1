param(
    [int]$Port = 8765,
    [string]$Root = '',
    [switch]$OpenBrowser
)

$ErrorActionPreference = 'Stop'
$python = if ($env:CHIPTUNE_PYTHON) { $env:CHIPTUNE_PYTHON } else { Join-Path $PSScriptRoot '.venv\Scripts\python.exe' }
if (-not $Root) {
    $Root = if ($env:CHIPTUNE_DATA_DIR) { $env:CHIPTUNE_DATA_DIR } else { Join-Path $PSScriptRoot 'data' }
}
$chipScript = Join-Path $PSScriptRoot 'chip.py'
$url = "http://127.0.0.1:$Port"

function Get-StudioInfo {
    try {
        return Invoke-RestMethod -Uri "$url/api/info" -TimeoutSec 1
    }
    catch {
        return $null
    }
}

function Show-StudioUrl {
    if ($OpenBrowser) {
        Start-Process $url
    }
    Write-Output $url
}

$existing = Get-StudioInfo
if ($null -ne $existing) {
    if ($existing.application -eq 'chiptune-studio') {
        Show-StudioUrl
        exit 0
    }
    throw "Port $Port is occupied by another application. Choose another port with -Port."
}

$listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
if ($null -ne $listener) {
    throw "Port $Port is occupied by another application. Choose another port with -Port."
}

if (-not (Test-Path -LiteralPath $python -PathType Leaf)) {
    throw "Python environment not found: $python"
}
if (-not (Test-Path -LiteralPath $chipScript -PathType Leaf)) {
    throw "Chiptune CLI not found: $chipScript"
}

$logRoot = Join-Path $Root 'logs'
New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
$stdoutLog = Join-Path $logRoot "studio_$Port.stdout.log"
$stderrLog = Join-Path $logRoot "studio_$Port.stderr.log"
$arguments = @(
    ('"{0}"' -f $chipScript),
    'studio',
    '--root',
    ('"{0}"' -f $Root),
    '--port',
    [string]$Port
)

$process = Start-Process -FilePath $python -ArgumentList $arguments -WindowStyle Hidden `
    -RedirectStandardOutput $stdoutLog -RedirectStandardError $stderrLog -PassThru
try {
    $process.PriorityClass = [System.Diagnostics.ProcessPriorityClass]::BelowNormal
}
catch {
    # The process may exit quickly if startup fails; the redirected logs retain its error.
}

for ($attempt = 0; $attempt -lt 20; $attempt++) {
    $info = Get-StudioInfo
    if ($null -ne $info) {
        if ($info.application -eq 'chiptune-studio') {
            Show-StudioUrl
            exit 0
        }
        throw "Port $Port was claimed by another application. Choose another port with -Port."
    }
    if ($process.HasExited) {
        throw "Studio did not start. Check logs: $stdoutLog and $stderrLog"
    }
    Start-Sleep -Milliseconds 250
}

throw "Studio did not respond within 5 seconds. Check logs: $stdoutLog and $stderrLog"
