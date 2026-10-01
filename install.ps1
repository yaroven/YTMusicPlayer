# ytm-player installer for Windows 10/11 (PowerShell 5.1+).
# Installs a prebuilt ytm.exe from GitHub Releases when available, otherwise
# builds it with Rust (installing rustup if missing). Then helps with the
# Google OAuth client and sign-in, and starts the player. Safe to re-run.
#
#   powershell -ExecutionPolicy Bypass -File .\install.ps1 [-FromSource] [-NoLaunch] [-Yes]

param(
    [switch]$FromSource,
    [switch]$NoLaunch,
    [switch]$Yes
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Say($msg) { Write-Host "==> $msg" -ForegroundColor Red }
function Info($msg) { Write-Host "    $msg" }
function Warn($msg) { Write-Host "warning: $msg" -ForegroundColor Yellow }
function Ask($question, $default = 'y') {
    if ($Yes -or -not [Environment]::UserInteractive) { return $default -eq 'y' }
    $hint = if ($default -eq 'y') { '[Y/n]' } else { '[y/N]' }
    $reply = Read-Host "    $question $hint"
    if (-not $reply) { $reply = $default }
    return $reply -match '^[Yy]'
}

if (-not (Select-String -Path Cargo.toml -Pattern '^name = "ytm-player"' -Quiet)) {
    throw 'run this script from the ytm-player repository'
}

$repo = (git config --get remote.origin.url 2>$null) -replace '^(https://github.com/|git@github.com:)', '' -replace '\.git$', ''
if ($repo -notmatch '/') { $repo = 'yaroven/YTMusicPlayer' }

$installDir = Join-Path $env:LOCALAPPDATA 'Programs\ytm-player'
$bin = $null

# --- 1. Prebuilt release --------------------------------------------------------

function Install-Prebuilt {
    $asset = 'ytm-x86_64-pc-windows-msvc.zip'   # also runs on ARM64 via emulation
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("ytm-" + [Guid]::NewGuid())
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        $base = "https://github.com/$repo/releases/latest/download"
        try {
            Invoke-WebRequest "$base/$asset" -OutFile "$tmp\$asset" -UseBasicParsing
            Invoke-WebRequest "$base/$asset.sha256" -OutFile "$tmp\$asset.sha256" -UseBasicParsing
        } catch {
            # Private repo: use the GitHub CLI and your login.
            if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { return $null }
            gh release download --repo $repo --pattern $asset --pattern "$asset.sha256" --dir $tmp 2>$null | Out-Null
            if ($LASTEXITCODE -ne 0) { return $null }
        }
        $expected = ((Get-Content "$tmp\$asset.sha256") -split '\s+')[0].ToLower()
        $actual = (Get-FileHash "$tmp\$asset" -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) { throw "checksum mismatch for $asset" }
        Expand-Archive "$tmp\$asset" -DestinationPath $tmp -Force
        New-Item -ItemType Directory $installDir -Force | Out-Null
        Copy-Item "$tmp\ytm.exe" "$installDir\ytm.exe" -Force
        return "$installDir\ytm.exe"
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if (-not $FromSource) {
    Say 'Downloading ytm'
    $bin = Install-Prebuilt
    if ($bin) { Info "installed release to $bin" } else { Info 'no prebuilt release available; building from source' }
}

# --- 2. Build from source (fallback) ------------------------------------------------

if (-not $bin) {
    Say 'Build tools'
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $hasMsvc = (Test-Path $vswhere) -and
        (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    if (-not $hasMsvc) {
        Warn 'MSVC C++ build tools are required (Rust links with them; SQLite is compiled in).'
        if (Ask 'Install Visual Studio Build Tools via winget (~2 GB)?') {
            winget install --id Microsoft.VisualStudio.2022.BuildTools -e --source winget `
                --override '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
        } else {
            throw 'install "Desktop development with C++" from Visual Studio Build Tools, then re-run'
        }
    }

    Say 'Rust toolchain'
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path "$cargoBin\cargo.exe")) {
        $env:Path = "$cargoBin;$env:Path"
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        if (-not (Ask 'Rust is not installed. Install it with rustup?')) { throw 'Rust is required' }
        $rustup = Join-Path ([IO.Path]::GetTempPath()) 'rustup-init.exe'
        Invoke-WebRequest 'https://win.rustup.rs/x86_64' -OutFile $rustup -UseBasicParsing
        & $rustup -y --profile minimal
        $env:Path = "$cargoBin;$env:Path"
    }
    Info (rustc --version)

    Say 'Building ytm (release, takes a few minutes the first time)'
    cargo install --path . --locked
    if ($LASTEXITCODE -ne 0) { throw 'build failed' }
    $bin = "$cargoBin\ytm.exe"
}

# --- 3. PATH ----------------------------------------------------------------------------

$binDir = Split-Path $bin
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $binDir) {
    if (Ask "Add $binDir to your PATH?") {
        [Environment]::SetEnvironmentVariable('Path', "$binDir;$userPath", 'User')
        Info 'added; new terminals will find ytm'
    }
}
$env:Path = "$binDir;$env:Path"

# --- 4. Google OAuth client ----------------------------------------------------------

Say 'Configuration'
Info ("config file: " + (& $bin config))
if ((& $bin status) -match 'OAuth client: missing') {
    Info 'No Google OAuth client yet (one-time setup, README "Set up Google sign-in").'
    $json = Get-ChildItem "$env:USERPROFILE\Downloads\client_secret_*.json" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($json -and (Ask "Import $($json.Name) from Downloads?")) {
        & $bin import-client $json.FullName
    } else {
        Info 'Later: ytm import-client <downloaded client_secret_….json>, then: ytm login'
    }
} else {
    Info 'OAuth client: configured'
}

# --- 5. Sign in -------------------------------------------------------------------------

$status = & $bin status
if ($status -match 'signed in:  yes') {
    Info 'Google account: signed in'
} elseif (($status -match 'OAuth client: set') -and -not $Yes) {
    Say 'Sign in'
    if (Ask 'Sign in with Google now (opens the browser)?') {
        & $bin login
        if ($LASTEXITCODE -ne 0) { Warn 'sign-in failed; retry later with: ytm login' }
    }
}

# --- 6. Done ----------------------------------------------------------------------------

Say 'Done'
& $bin status | ForEach-Object { Info $_ }
Info 'Start:        ytm'
Info 'Test audio:   ytm play dQw4w9WgXcQ'

if (-not $NoLaunch -and -not $Yes -and (Ask 'Start ytm now?')) {
    & $bin
}
