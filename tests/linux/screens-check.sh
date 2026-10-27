#!/usr/bin/env bash
# Both themes on Linux (scripts/verify-linux-desktop.ps1, the second layer;
# D225, item 3): the build with developer tools (`--features screenshots`,
# made in the package image) runs on demo data (`--demo`: 300 made-up hosts
# on documentation addresses, a temporary profile) with `--theme light` and
# `--theme dark` under Xvfb with Mesa's OpenGL in
# tests/linux/desktop.Dockerfile, and the screen is captured. The released
# app follows the system's theme, which a container has no portal to set;
# `--theme` exists only with developer tools. (`--screenshots` itself cannot
# run on Linux: GPUI has no `render_to_image` there.) Usage, as any user:
#
#   screens-check.sh <shellpp built with screenshots> <out-dir>
set -euo pipefail

app="${1:?usage: screens-check.sh <shellpp> <out-dir>}"
out="${2:?usage: screens-check.sh <shellpp> <out-dir>}"

mkdir -p "$out"
export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR="$(mktemp -d)"
chmod 700 "$XDG_RUNTIME_DIR"
export DISPLAY=:99
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp > "$out/xvfb.log" 2>&1 &
for _ in $(seq 1 40); do
    xdpyinfo > /dev/null 2>&1 && break
    sleep 0.25
done

code=0
for theme in light dark; do
    "$app" --demo --theme "$theme" > "$out/app-$theme.log" 2>&1 &
    pid=$!
    window=""
    for _ in $(seq 1 120); do
        window="$(xdotool search --onlyvisible --class shellpp 2>/dev/null | head -n 1 || true)"
        [ -n "$window" ] && break
        sleep 0.5
    done
    if [ -z "$window" ]; then
        echo "screens-check: $theme: no window"
        code=1
    else
        xdotool mousemove --window "$window" 640 400
        # The demo profile is generated and unlocked first (a debug build).
        sleep 20
        # Icons rasterized after the first frames show at the next one: a
        # pointer move asks for it.
        xdotool mousemove --window "$window" 700 30
        sleep 2
        xdotool mousemove --window "$window" 1270 790
        sleep 2
        import -window root "$out/demo-$theme.png"
        echo "screens-check: $theme: demo-$theme.png"
    fi
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
done
exit "$code"
