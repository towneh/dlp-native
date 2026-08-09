#Requires -Version 7
# Build the native library for the Windows x64 host.
# Produces unity_dlp.dll and copies it (plus the Python runtime DLLs) into
# unity_package/Plugins/x86_64/.
#
# Usage:
#   pwsh scripts/build-host.ps1           # release-with-debuginfo (default)
#   pwsh scripts/build-host.ps1 -Debug    # debug build
#   pwsh scripts/build-host.ps1 -Release  # plain release (no debug symbols)
param(
    [switch]$Debug,
    [switch]$Release
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path $PSScriptRoot -Parent
Set-Location $RepoRoot

# ── Locate Python via uv ──────────────────────────────────────────────────────
# Keep in step with PYTHON_VERSION in .github/workflows/build.yml.
$PyVersion = '3.14'
Write-Host "==> Locating Python $PyVersion via uv..."
$PyExe = (uv python find $PyVersion 2>&1).Trim()
if (-not (Test-Path $PyExe)) {
    Write-Error "Python $PyVersion not found via uv. Run: uv python install $PyVersion"
}
$PyPrefix = (& $PyExe -c "import sys; print(sys.prefix, end='')").Trim()
Write-Host "    Python : $PyExe"
Write-Host "    Prefix : $PyPrefix"

$env:PYO3_PYTHON = $PyExe

# ── Build ─────────────────────────────────────────────────────────────────────
$CargoArgs = @('-p', 'unity_dlp_core')
if ($Debug) {
    $Profile = 'debug'
} elseif ($Release) {
    $Profile = 'release'
    $CargoArgs += '--release'
} else {
    $Profile = 'release-with-debuginfo'
    $CargoArgs += '--profile', 'release-with-debuginfo'
}

Write-Host "==> cargo build $($CargoArgs -join ' ')..."
cargo build @CargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# ── Stage to Unity Plugins ────────────────────────────────────────────────────
$Dest = Join-Path $RepoRoot 'unity_package\Plugins\x86_64'
New-Item -ItemType Directory -Force $Dest | Out-Null

$DllSrc = Join-Path $RepoRoot "target\$Profile\unity_dlp.dll"
Copy-Item $DllSrc $Dest -Force
Write-Host "==> Copied unity_dlp.dll → $Dest"

# Copy the Python runtime DLLs that unity_dlp.dll links against.
# python3.dll is the stable-ABI forwarder; pythonXY.dll is the full runtime.
$PyDll = "python$($PyVersion -replace '\.', '').dll"
foreach ($dll in @('python3.dll', $PyDll, 'vcruntime140.dll', 'vcruntime140_1.dll')) {
    $src = Join-Path $PyPrefix $dll
    if (Test-Path $src) {
        Copy-Item $src $Dest -Force
        Write-Host "==> Copied $dll → $Dest"
    }
}

Write-Host ''
Write-Host 'Build complete. Open the Unity project and run the smoke test via'
Write-Host 'Tools → YtDlp → 1 – Init only.'
