#!/usr/bin/env bash
# The Linux package of Shell++ (D225, item 8): builds the desktop app with the
# `dist` profile, without the `fido2-hid` feature (D173), and writes
#
#   <out-dir>/shellpp-<version>-linux-x64.tar.gz  with one folder
#     shellpp-<version>-linux-x64/: shellpp, shellpp.desktop, shellpp.png,
#     LICENSE, THIRD_PARTY_NOTICES.md, README-linux.txt
#   <out-dir>/shellpp-<version>-linux-x64.tar.gz.sha256
#
# Runs inside the image of tests/linux/package.Dockerfile (Ubuntu 22.04), the
# same for the release workflow and scripts/release-dry-run.ps1:
#
#   docker run --rm -v "$PWD:/src:ro" -v <out>:/out -v <target>:/target \
#     <image> bash scripts/package-linux.sh /out
#
# It refuses an executable that needs a glibc newer than 2.35 (D225, item 12).
set -euo pipefail

out="${1:?usage: package-linux.sh <out-dir>}"
repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"

# [workspace.package] version of Cargo.toml (docs/release.md).
version="$(awk '/^\[workspace\.package\]/{in_section=1; next} /^\[/{in_section=0} in_section && /^version *=/{gsub(/.*= *"|".*/, ""); print; exit}' Cargo.toml)"
if [ -z "$version" ]; then
    echo "package-linux: no [workspace.package] version in Cargo.toml" >&2
    exit 1
fi
name="shellpp-${version}-linux-x64"
max_glibc="2.35"

cargo build --locked --profile dist -p shellpp-desktop
binary="${CARGO_TARGET_DIR:-$repo/target}/dist/shellpp"

# The newest glibc symbol version the executable asks for.
needed="$(objdump -T "$binary" | sed -n 's/.*GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -uV | tail -n 1)"
if [ -z "$needed" ]; then
    echo "package-linux: no glibc version found in shellpp's dynamic symbols" >&2
    exit 1
fi
# GLIBC_ABI_DT_RELR comes with glibc 2.36.
if objdump -T "$binary" | grep -q 'GLIBC_ABI_DT_RELR'; then
    needed="2.36"
fi
if [ "$(printf '%s\n%s\n' "$needed" "$max_glibc" | sort -V | tail -n 1)" != "$max_glibc" ]; then
    echo "package-linux: shellpp needs glibc $needed, more than $max_glibc" >&2
    exit 1
fi
echo "package-linux: shellpp $version needs glibc $needed or later"

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
dir="$staging/$name"
mkdir -p "$dir"
install -m 0755 "$binary" "$dir/shellpp"
# The package carries the code and its symbol table (function names in
# backtraces); the line tables of the dist profile are dropped, and the
# release workflow keeps no copy of them (its build container is removed).
strip --strip-debug "$dir/shellpp"
install -m 0644 apps/desktop/packaging/linux/shellpp.desktop "$dir/shellpp.desktop"
install -m 0644 apps/desktop/assets/icons/128x128.png "$dir/shellpp.png"
install -m 0644 LICENSE "$dir/LICENSE"
install -m 0644 THIRD_PARTY_NOTICES.md "$dir/THIRD_PARTY_NOTICES.md"
cat > "$dir/README-linux.txt" <<EOF
Shell++ ${version} for Linux (x86-64)

Run ./shellpp from this folder, or install it for your user:

  install -Dm755 shellpp ~/.local/bin/shellpp
  install -Dm644 shellpp.desktop ~/.local/share/applications/shellpp.desktop
  install -Dm644 shellpp.png ~/.local/share/icons/hicolor/128x128/apps/shellpp.png

~/.local/bin has to be on your PATH for the menu entry to start it.

What it needs: glibc ${max_glibc} or later (Ubuntu 22.04 or later, Debian 12 or
later), an X11 or Wayland session, a Vulkan driver (Mesa's is enough) and the
libraries libxkbcommon, libxkbcommon-x11, libxcb, libX11-xcb,
libwayland-client, fontconfig and freetype. "Remember on this device" needs a
Secret Service provider (GNOME Keyring, KWallet, KeePassXC); without one the
app asks for the master password at every start.

The data lives in \${XDG_DATA_HOME:-~/.local/share}/shellpp/, the logs in
\${XDG_STATE_HOME:-~/.local/state}/shellpp/logs/, temporary SFTP files in
\${XDG_CACHE_HOME:-~/.cache}/shellpp/tmp/. Start it with SHELLPP_DATA_DIR set
to a folder to keep the data there instead. To uninstall, delete the three
installed files and those folders.

Linux is checked in containers, not yet on a real desktop: please report
problems at https://github.com/proxyboy228/shellpp/issues
More: https://github.com/proxyboy228/shellpp#linux
EOF

mkdir -p "$out"
archive="$out/$name.tar.gz"
# Owner, order and times fixed, so the archive depends on its files only.
mtime="@${SOURCE_DATE_EPOCH:-$(stat -c %Y Cargo.lock)}"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="$mtime" \
    -C "$staging" -cf - "$name" | gzip -n -9 > "$archive"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "package-linux: $archive"
