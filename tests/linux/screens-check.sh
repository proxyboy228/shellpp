#!/usr/bin/env bash
# Both themes on Linux (scripts/verify-linux-desktop.ps1, the second layer;
# D225, item 3): the build with developer tools (`--features dev-tools`,
# made in the package image) runs on demo data (`--demo`: 300 made-up hosts
# on documentation addresses, a temporary profile) with `--theme light` and
# `--theme dark` under Xvfb with Mesa's OpenGL in
# tests/linux/desktop.Dockerfile, and the screen is captured. The released
# app follows the system's theme, which a container has no portal to set;
# `--theme` exists only with developer tools. (`--screenshots` itself cannot
# run on Linux: GPUI has no `render_to_image` there.) Each capture must show
# a picture, and the dark one must be darker than the light one. Usage, as
# any user:
#
#   screens-check.sh <shellpp built with dev-tools> <out-dir>
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

# The mean and the spread of grey of a capture, "mean spread".
grey() {
    convert "$1" -colorspace Gray -format '%[fx:mean] %[fx:standard_deviation]' info:
}

code=0
declare -A mean
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
        echo "screens-check: FAIL $theme: no window"
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
        read -r m s <<< "$(grey "$out/demo-$theme.png")"
        mean[$theme]="$m"
        if awk -v s="$s" 'BEGIN { exit !(s > 0.02) }'; then
            echo "screens-check: ok $theme: demo-$theme.png (grey mean $m, spread $s)"
        else
            echo "screens-check: FAIL $theme: demo-$theme.png shows no picture (spread $s)"
            code=1
        fi
    fi
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
done
if [ -n "${mean[light]:-}" ] && [ -n "${mean[dark]:-}" ]; then
    if awk -v d="${mean[dark]}" -v l="${mean[light]}" 'BEGIN { exit !(d < l) }'; then
        echo "screens-check: ok the dark theme is darker than the light one"
    else
        echo "screens-check: FAIL the dark capture is not darker than the light one"
        code=1
    fi
fi
exit "$code"
