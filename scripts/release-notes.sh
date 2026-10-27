#!/usr/bin/env bash
# Prints the release notes of <version>: the section of CHANGELOG.md under the
# heading "## [<version>]", up to the next "## [" heading (D225, item 18;
# docs/release.md). The release workflow and scripts/release-dry-run.ps1 use
# it; a version without a section, or with an empty one, fails.
set -euo pipefail

version="${1:?usage: release-notes.sh <version>}"
cd "$(dirname "$0")/.."

notes="$(awk -v heading="## [$version]" '
    $0 == heading { inside = 1; next }
    inside && /^## \[/ { exit }
    inside { sub(/\r$/, ""); print }
' CHANGELOG.md | sed -e '/./,$!d')"

if [ -z "$notes" ]; then
    echo "release-notes: CHANGELOG.md has no notes under \"## [$version]\"" >&2
    exit 1
fi
printf '%s\n' "$notes"
