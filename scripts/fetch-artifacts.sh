#!/usr/bin/env bash
# Fetch the latest successful CI build artifacts and merge them into unity_package/.
#
# Requires: gh CLI authenticated to the repo  (gh auth login)
#
# Usage:
#   bash scripts/fetch-artifacts.sh                    # all platforms
#   bash scripts/fetch-artifacts.sh windows linux      # specific platforms
#   bash scripts/fetch-artifacts.sh -r 12345678        # specific run ID
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

artifact_name() {
    case "$1" in
        windows) echo "unity_dlp-windows-x64" ;;
        macos)   echo "unity_dlp-macos-universal" ;;
        linux)   echo "unity_dlp-linux-x64" ;;
        android) echo "unity_dlp-android-arm64" ;;
        ios)     echo "unity_dlp-ios-arm64" ;;
        *)       echo "" ;;
    esac
}

ALL_PLATFORMS=(windows macos linux android ios)
PLATFORMS=()
RUN_ID=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -r|--run)
            RUN_ID="$2"
            shift 2
            ;;
        -*)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
        *)
            PLATFORMS+=("$1")
            shift
            ;;
    esac
done

if [[ ${#PLATFORMS[@]} -eq 0 ]]; then
    PLATFORMS=("${ALL_PLATFORMS[@]}")
fi

# Temp dir — cleaned up on exit
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

# Resolve run ID
if [[ -z "$RUN_ID" ]]; then
    echo "==> Finding latest successful run on main..."
    RUN_ID="$(gh run list \
        --workflow build.yml \
        --branch main \
        --status success \
        --limit 1 \
        --json databaseId \
        --jq '.[0].databaseId')"
    if [[ -z "$RUN_ID" ]]; then
        echo "ERROR: No successful runs found on main branch." \
             "Check: gh run list --workflow build.yml --branch main" >&2
        exit 1
    fi
    echo "    Run ID: $RUN_ID"
fi

for PLAT in "${PLATFORMS[@]}"; do
    NAME="$(artifact_name "$PLAT")"
    if [[ -z "$NAME" ]]; then
        echo "WARNING: Unknown platform '$PLAT'. Valid: ${ALL_PLATFORMS[*]}" >&2
        continue
    fi

    echo "==> Downloading $NAME..."
    PLAT_TMP="$TMP_DIR/$PLAT"
    mkdir -p "$PLAT_TMP"
    gh run download "$RUN_ID" --name "$NAME" --dir "$PLAT_TMP"

    # upload-artifact@v4 strips the common path prefix, so unity_package/ is stripped
    # and Plugins/ + StreamingAssets/ land at the root of the download dir.
    cp -r "$PLAT_TMP/." "$REPO_ROOT/unity_package/"
    echo "    Merged into unity_package/"
done

echo ""
echo "Done."
