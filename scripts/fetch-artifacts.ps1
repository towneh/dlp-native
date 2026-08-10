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

# These doubled paths must not exist: Unity reads one as a second native plugin
# sharing a name with the real one and fails the build on the duplicate. Clear
# them if present — and only them; anything else in unity_package/ is left alone.
foreach ($doubled in @("Plugins/Plugins", "StreamingAssets/StreamingAssets")) {
    $stale = Join-Path $repoRoot (Join-Path "unity_package" $doubled)
    if (Test-Path -LiteralPath $stale) {
        Write-Host "==> Removing nested path: unity_package/$doubled"
        Remove-Item -LiteralPath $stale -Recurse -Force
    }
}

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
        #
        # Merge file by file with each relative path rebuilt: `Copy-Item -Recurse`
        # on a directory whose destination already exists copies the source *inside*
        # it instead of merging, and every destination here already exists.
        $dstPkg = Join-Path $repoRoot "unity_package"
        $prefix = (Resolve-Path $platTmp).Path.TrimEnd('\', '/')

        $planned = Get-ChildItem $platTmp -Recurse -File | ForEach-Object {
            $rel = $_.FullName.Substring($prefix.Length).TrimStart('\', '/')
            [pscustomobject]@{ Source = $_.FullName; Dest = Join-Path $dstPkg $rel }
        }

        # Check every destination is writable before touching any of them. The
        # common failure is the Unity Editor holding unity_dlp.dll open, which
        # would otherwise leave StreamingAssets updated against a stale plugin —
        # a mismatched pair that is hard to spot and easy to misdiagnose.
        $locked = $planned | Where-Object { Test-Path -LiteralPath $_.Dest } | Where-Object {
            try {
                $fs = [System.IO.File]::Open($_.Dest, 'Open', 'Write', 'None')
                $fs.Close()
                $false
            } catch { $true }
        }
        if ($locked) {
            Write-Warning "Cannot write these files — close the Unity Editor and re-run:"
            $locked | ForEach-Object { Write-Warning "    $($_.Dest)" }
            throw "Refusing to part-update unity_package/ for $name"
        }

        # Each file is written to a sibling temp name and renamed, so an interrupted
        # write cannot leave a half-written binary in place of a good one. Every
        # replaced file is kept until the whole set lands: a failure part way through
        # would otherwise leave the same mismatched pair the check above prevents, so
        # the originals go back.
        $backupRoot = Join-Path $tmpDir "restore-$plat"
        $replaced = New-Object System.Collections.ArrayList
        $created  = New-Object System.Collections.ArrayList
        $stagedFiles = New-Object System.Collections.ArrayList
        try {
            foreach ($item in $planned) {
                New-Item -ItemType Directory -Force (Split-Path $item.Dest -Parent) | Out-Null

                if (Test-Path -LiteralPath $item.Dest) {
                    $backup = Join-Path $backupRoot ([System.IO.Path]::GetRandomFileName())
                    New-Item -ItemType Directory -Force $backupRoot | Out-Null
                    Copy-Item -LiteralPath $item.Dest -Destination $backup -Force
                    [void]$replaced.Add([pscustomobject]@{ Dest = $item.Dest; Backup = $backup })
                } else {
                    [void]$created.Add($item.Dest)
                }

                # Unique per file, so this cannot collide with anything already on
                # disk and the exact paths written are known for the rollback below.
                $staged = "$($item.Dest).$([System.IO.Path]::GetRandomFileName()).incoming"
                [void]$stagedFiles.Add($staged)
                Copy-Item -LiteralPath $item.Source -Destination $staged -Force
                Move-Item -LiteralPath $staged -Destination $item.Dest -Force
            }
        } catch {
            Write-Warning "Staging $name failed — restoring unity_package/ to its previous state"
            foreach ($r in $replaced) {
                Copy-Item -LiteralPath $r.Backup -Destination $r.Dest -Force -ErrorAction SilentlyContinue
            }
            foreach ($c in $created) {
                Remove-Item -LiteralPath $c -Force -ErrorAction SilentlyContinue
            }
            # Only the paths this merge wrote; anything else is not ours to delete.
            foreach ($staged in $stagedFiles) {
                Remove-Item -LiteralPath $staged -Force -ErrorAction SilentlyContinue
            }
            throw
        }
        Write-Host "    Merged into unity_package/ ($($planned.Count) files)"
    }
}
finally {
    Remove-Item $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Done."
