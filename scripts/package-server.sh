#!/usr/bin/env bash
# The sync server's Linux archive of a release (docs/release.md): the static
# binary taken from the server's image, with LICENSE, README.md,
# THIRD_PARTY_NOTICES.md (D225, item 14) and deploy/, as
#
#   <out-dir>/shellpp-server-<version>-linux-x64.tar.gz
#
# The release workflow and scripts/release-dry-run.ps1 both call it after
# `docker cp`-ing /usr/local/bin/shellpp-server out of the image.
set -euo pipefail

binary="${1:?usage: package-server.sh <shellpp-server binary> <version> <out-dir>}"
version="${2:?usage: package-server.sh <shellpp-server binary> <version> <out-dir>}"
out="${3:?usage: package-server.sh <shellpp-server binary> <version> <out-dir>}"
repo="$(cd "$(dirname "$0")/.." && pwd)"

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
install -m 0755 "$binary" "$staging/shellpp-server"
install -m 0644 "$repo/server/LICENSE" "$repo/server/README.md" "$repo/server/THIRD_PARTY_NOTICES.md" "$staging/"
cp -r "$repo/server/deploy" "$staging/deploy"

mkdir -p "$out"
archive="$out/shellpp-server-${version}-linux-x64.tar.gz"
# Owner, order and times fixed, so the archive depends on its files only.
mtime="@${SOURCE_DATE_EPOCH:-$(stat -c %Y "$repo/server/Cargo.lock")}"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="$mtime" \
    -C "$staging" -cf - . | gzip -n -9 > "$archive"
echo "package-server: $archive"
