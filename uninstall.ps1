# ytm-player uninstaller for Windows (PowerShell 5.1+). For installs made
# with the setup program it simply runs that program's uninstaller (also in
# Settings > Apps). For install.ps1 / `cargo install` installs it removes
# ytm.exe and its PATH entry. Asks before deleting your library, settings,
# logs and sign-in.
#
#   powershell -ExecutionPolicy Bypass -File .\uninstall.ps1 [-Purge] [-KeepData] [-Yes]

param(
    [switch]$Purge,
    [switch]$KeepData,
    [switch]$Yes
)

$ErrorActionPreference = 'Stop'

function Say($msg) { Write-Host "==> $msg" -ForegroundColor Red }
function Info($msg) { Write-Host "    $msg" }
function Ask($question, $default = 'y') {
    if ($Yes -or -not [Environment]::UserInteractive) { return $default -eq 'y' }
    $hint = if ($default -eq 'y') { '[Y/n]' } else { '[y/N]' }
    $reply = Read-Host "    $question $hint"
    if (-not $reply) { $reply = $default }
    return $reply -match '^[Yy]'
}

Say 'Uninstalling ytm-player'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\ytm-player'
$uninstaller = Join-Path $installDir 'unins000.exe'
if (Test-Path $uninstaller) {
    Info 'installed with the setup program: running its uninstaller'
    Start-Process $uninstaller -Wait
    exit 0
}

Get-Process ytm -ErrorAction SilentlyContinue | Stop-Process -Force

$bins = @(
    (Join-Path $installDir 'ytm.exe'),
    (Join-Path $env:USERPROFILE '.cargo\bin\ytm.exe')
) | Where-Object { Test-Path $_ }

$purgeData = $Purge -or (-not $KeepData -and (Ask 'Also delete your library, settings, logs and Google sign-in?' 'n'))
if ($purgeData) {
    if ($bins) { & $bins[0] logout 2>$null | Out-Null }
    foreach ($dir in @((Join-Path $env:APPDATA 'ytm-player'), (Join-Path $env:LOCALAPPDATA 'ytm-player'))) {
        if (Test-Path $dir) { Remove-Item $dir -Recurse -Force; Info "removed $dir" }
    }
}

foreach ($bin in $bins) {
    if ($bin -like '*\.cargo\bin\*' -and (Get-Command cargo -ErrorAction SilentlyContinue)) {
        cargo uninstall ytm-player 2>$null | Out-Null
    }
    if (Test-Path $bin) { Remove-Item $bin -Force }
    Info "removed $bin"
}
if ((Test-Path $installDir) -and -not (Get-ChildItem $installDir)) { Remove-Item $installDir }

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$kept = ($userPath -split ';') | Where-Object { $_ -and $_ -ne $installDir }
if ($kept.Count -ne ($userPath -split ';' | Where-Object { $_ }).Count) {
    [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
    Info "removed $installDir from PATH"
}

Say 'Done'
Info 'ytm-player is uninstalled.'
