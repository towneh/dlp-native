# Vendor Pins

This file records the pinned versions of vendored submodules.
Update both the submodule commit and this file when bumping.

| Submodule | Tag / Commit | Pinned date |
|-----------|-------------|-------------|
| `vendor/yt-dlp` | _set at submodule init_ | — |
| `vendor/yt-dlp-ejs` | _set at submodule init_ | — |

## Bumping yt-dlp

1. `cd vendor/yt-dlp && git fetch --tags && git checkout <new-tag>`
2. Update the table above.
3. Run `scripts/bump-yt-dlp.sh` (Phase 6) — it will rebuild and run URL tests.
4. If all tests pass, commit `vendor/yt-dlp` and `VENDOR.md` together.
5. Tag the commit `yt-dlp/<new-tag>`.

## Adding submodules (first time)

```sh
git submodule add https://github.com/yt-dlp/yt-dlp vendor/yt-dlp
git -C vendor/yt-dlp checkout 2025.01.15   # substitute pinned tag
git submodule add https://github.com/yt-dlp/yt-dlp-ejs vendor/yt-dlp-ejs
git -C vendor/yt-dlp-ejs checkout <tag matching yt-dlp's pyproject.toml>
```
