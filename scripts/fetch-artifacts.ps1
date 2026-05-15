# Fetch the latest successful CI build artifacts and merge them into unity_package/.
#
# Requires: gh CLI authenticated to the repo  (gh auth login)
#
# Usage:
#   pwsh scripts/fetch-artifacts.ps1                    # all platforms
#   pwsh scripts/fetch-artifacts.ps1 windows linux      # specific platforms
#   pwsh scripts/fetch-artifacts.ps1 -Run 12345678      # specific run ID

param(
    [Parameter(Position = 0, ValueFromRemainingArguments)]
    [string[]] $Platforms = @("windows", "macos", "linux", "android", "ios"),

    [string] $Run = ""
)

$ErrorActionPreference = "Stop"

$artifactNames = @{
    "windows" = "unity_dlp-windows-x64"
    "macos"   = "unity_dlp-macos-universal"
    "linux"   = "unity_dlp-linux-x64"
    "android" = "unity_dlp-android-arm64"
    "ios"     = "unity_dlp-ios-arm64"
}

$repoRoot = Split-Path $PSScriptRoot -Parent
$tmpDir   = Join-Path ([System.IO.Path]::GetTempPath()) "dlp-artifacts-$(Get-Random)"

try {
    New-Item -ItemType Directory -Force $tmpDir | Out-Null

    if (-not $Run) {
        Write-Host "==> Finding latest successful run on main..."
        $Run = (gh run list `
            --workflow build.yml `
            --branch main `
            --status success `
            --limit 1 `
            --json databaseId `
            --jq '.[0].databaseId').Trim()
        if (-not $Run) {
            Write-Error "No successful runs found on main branch. Check: gh run list --workflow build.yml --branch main"
            exit 1
        }
        Write-Host "    Run ID: $Run"
    }

    foreach ($plat in $Platforms) {
        $name = $artifactNames[$plat]
        if (-not $name) {
            Write-Warning "Unknown platform '$plat'. Valid: $($artifactNames.Keys -join ', ')"
            continue
        }

        Write-Host "==> Downloading $name..."
        # Use a per-platform subdir: gh run download --dir puts files directly into
        # the target dir (no artifact-name subdirectory is created).
        $platTmp = Join-Path $tmpDir $plat
        New-Item -ItemType Directory -Force $platTmp | Out-Null
        gh run download $Run --name $name --dir $platTmp
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "gh run download failed for $name — skipping"
            continue
        }

        # upload-artifact@v4 strips the common path prefix, so unity_package/ is
        # stripped and Plugins/ + StreamingAssets/ land at the root of the download dir.
        $dstPkg = Join-Path $repoRoot "unity_package"
        Get-ChildItem $platTmp | ForEach-Object {
            Copy-Item $_.FullName (Join-Path $dstPkg $_.Name) -Recurse -Force
        }
        Write-Host "    Merged into unity_package/"
    }
}
finally {
    Remove-Item $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Done."
