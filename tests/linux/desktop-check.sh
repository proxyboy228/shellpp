#!/usr/bin/env bash
# The Linux app in a clean Ubuntu 22.04 (tests/linux/desktop.Dockerfile;
# scripts/verify-linux-desktop.ps1; D225, item 3). Under Xvfb it starts the
# app as the user `tester` and drives it with xdotool. Two modes:
#
#   desktop-check.sh <shellpp-<version>-linux-x64.tar.gz> <out-dir>
#     The package (plan of run 17, 3.4, first layer): desktop-file-validate,
#     ldd, then the executable of the tar.gz in four sessions:
#       lavapipe    on Mesa's software Vulkan driver: the window appears;
#                   whether it draws is recorded (it does not: GPUI's frames
#                   on lavapipe never reach Xvfb), so the other sessions run
#                   on Mesa's OpenGL (llvmpipe) with the Vulkan driver hidden;
#       keyring     a D-Bus session with GNOME Keyring as the Secret Service:
#                   first run ("Remember on this device" on), the device key
#                   in the Secret Service, lock and unlock, a restart that
#                   opens with the device key, a quick connect to OpenSSH (the
#                   host key, the password, a command), the SFTP panel listing
#                   the server's folder;
#       no-keyring  a D-Bus session without a Secret Service: the app starts
#                   and offers no "Remember on this device";
#       no-dbus     no D-Bus session at all: the app starts.
#
#   desktop-check.sh --screens <shellpp built with screenshots> <out-dir>
#     The app's scenario (3.4, second layer): the build with the
#     `screenshots` feature on demo data (`--demo`) in both themes
#     (`--theme light`, `--theme dark`): in each, a quick connect to OpenSSH
#     and the SFTP panel listing the server's folder. (`--screenshots`
#     itself cannot run on Linux: GPUI has no `render_to_image` there, so
#     the screen is captured from Xvfb.)
#
# It starts as root: OpenSSH for `sshuser` with a password made here. The
# screenshots, the app's output and its log go to <out-dir>; results.txt
# says which checks passed, and every expected check must pass.
set -uo pipefail

mode="package"
session=""
if [ "${1:-}" = "--screens" ]; then
    mode="screens"
    shift
elif [ "${1:-}" = "--session" ]; then
    session="$2"
    mode="${3:-package}"
    shift 3
fi
target="${1:?usage: desktop-check.sh [--screens] <package.tar.gz | shellpp> <out-dir>}"
out="${2:?usage: desktop-check.sh [--screens] <package.tar.gz | shellpp> <out-dir>}"
results="$out/results.txt"
self="$(readlink -f "$0")"

note() { echo "desktop-check: $*"; }
record() { echo "$1 $2" >> "$results"; note "$1 $2"; }

# --- the app's side, as `tester` -------------------------------------------

# Waits for the app's window; prints its id. $1: seconds.
window_of_app() {
    local deadline=$((SECONDS + $1)) id
    while [ "$SECONDS" -lt "$deadline" ]; do
        id="$(xdotool search --onlyvisible --class shellpp 2>/dev/null | head -n 1)"
        if [ -n "$id" ]; then
            echo "$id"
            return 0
        fi
        sleep 0.5
    done
    return 1
}

# Starts the app; sets $app_pid and $window. $1: its data folder (empty: the
# app's own, as `--demo` makes it), $2: a name for its output, the rest: its
# arguments.
start_app() {
    local data="$1" name="$2" started=$SECONDS
    shift 2
    if [ -n "$data" ]; then
        SHELLPP_DATA_DIR="$data" "$APP" "$@" > "$out/app-$name.log" 2>&1 &
    else
        "$APP" "$@" > "$out/app-$name.log" 2>&1 &
    fi
    app_pid=$!
    if window="$(window_of_app 120)"; then
        note "$name: window $window after $((SECONDS - started)) s"
        # No window manager under Xvfb: the pointer over the window gives it
        # the keyboard (focus follows the pointer).
        xdotool mousemove --window "$window" 640 400
        sleep 5
        return 0
    fi
    window=""
    return 1
}

stop_app() {
    if [ -n "${app_pid:-}" ] && kill -0 "$app_pid" 2>/dev/null; then
        kill "$app_pid"
        for _ in $(seq 1 20); do
            kill -0 "$app_pid" 2>/dev/null || break
            sleep 0.5
        done
        kill -9 "$app_pid" 2>/dev/null
    fi
    app_pid=""
}

shot() { import -window root "$out/$1.png" 2>/dev/null; }
click() { xdotool mousemove --window "$window" "$1" "$2" click 1; sleep 1; }

# The spread of grey of a screenshot: a black or empty screen has almost none.
spread_of() {
    convert "$out/$1.png" -colorspace Gray -format '%[fx:standard_deviation]' info: 2>/dev/null || echo 0
}
drawn() { awk -v s="$(spread_of "$1")" 'BEGIN { exit !(s > 0.02) }'; }
mean_of() {
    convert "$out/$1.png" -colorspace Gray -format '%[fx:mean]' info: 2>/dev/null || echo 0
}

# Whether the Secret Service holds an item of the service `shellpp`; its
# attributes (never the secret) go to secret-tool.txt.
device_key_stored() {
    secret-tool search --all service shellpp 2>&1 | grep -v '^secret' > "$out/secret-tool.txt"
    grep -q 'shellpp' "$out/secret-tool.txt"
}

keep_log() { cp "$1/logs/shellpp.log" "$out/shellpp-$2.log" 2>/dev/null; }

# A quick connect to OpenSSH on this machine (the search field, UI 2.1) and
# the SFTP panel on the same server, from the hosts screen. $1: a name for
# the screenshots and checks, $2: the log to read. The positions are those of
# the 1280x800 window.
connect_and_list() {
    local tag="$1" log="$2" sftp_id opened
    click 640 78           # the search field, which may keep an address
    xdotool key ctrl+a
    xdotool type --delay 40 "sshuser@127.0.0.1:2222"
    xdotool key Return
    sleep 6
    shot "$tag-host-key"
    # The host key of a first connection (UI 10.2): Enter takes the default.
    xdotool key Return
    sleep 4
    shot "$tag-password"
    xdotool type --delay 40 "$(cat "$SSH_PASSWORD_FILE")"
    xdotool key Return
    sleep 6
    # The command leaves a file the check reads: the keyboard reached the
    # terminal and the shell ran it.
    xdotool type --delay 40 "ls -la; echo desktop-check-ok | tee /tmp/desktop-check-$tag"
    xdotool key Return
    sleep 4
    shot "$tag-terminal"
    if grep -q "connected session=" "$log" 2>/dev/null \
        && grep -qx desktop-check-ok "/tmp/desktop-check-$tag" 2>/dev/null; then
        record ok "$tag: quick connect logged in to OpenSSH and ran a command ($tag-terminal.png)"
    else
        record FAIL "$tag: quick connect did not log in to OpenSSH or run the command ($tag-host-key, -password, -terminal)"
    fi

    # SFTP to the same server: the host saved from the search ("Save as
    # host", UI 2.1), then the SFTP tab's remote panel (UI 5).
    click 93 22            # the Vaults tab
    click 640 78
    xdotool key ctrl+a
    xdotool type --delay 40 "sshuser@127.0.0.1:2222"
    sleep 2
    click 737 499          # "Save as host"
    sleep 3
    shot "$tag-saved-host"
    xdotool key Escape
    sleep 1
    click 177 22           # the SFTP tab
    sleep 2
    opened="$(grep -c 'opendir "/home/sshuser' "$SFTP_LOG")"
    click 960 480          # "Choose host..." of the remote panel
    sleep 2
    xdotool type --delay 40 "127.0.0.1"
    sleep 1
    click 865 334          # the host in the picker
    sleep 5
    shot "$tag-sftp-connect"
    # The panel's password prompt (the test password is made per run and
    # lives only in this container).
    xdotool type --delay 40 "$(cat "$SSH_PASSWORD_FILE")"
    xdotool key Return
    sleep 6
    shot "$tag-sftp-list"
    # The SFTP session's own id: `session opened session=N kind="sftp"`,
    # then `connected session=N`; and OpenSSH's SFTP server logged that the
    # folder was opened for its list since the SFTP tab was opened.
    sftp_id="$(sed -n 's/.*session opened session=\([0-9]*\) kind="sftp".*/\1/p' "$log" | tail -n 1)"
    if [ -n "$sftp_id" ] && grep -q "connected session=$sftp_id " "$log" \
        && [ "$(grep -c 'opendir "/home/sshuser' "$SFTP_LOG")" -gt "$opened" ]; then
        record ok "$tag: the SFTP panel connected to OpenSSH and listed the server's folder ($tag-sftp-list.png: backups, projects, notes.txt)"
    else
        record FAIL "$tag: the SFTP panel did not connect or list the server's folder ($tag-saved-host, -sftp-connect, -sftp-list)"
    fi
}

session_lavapipe() {
    local icd
    icd="$(find /usr/share/vulkan/icd.d -name 'lvp_icd.*.json' | head -n 1)"
    if [ -z "$icd" ]; then
        record info "lavapipe: the image has no Mesa software Vulkan driver"
        return
    fi
    export VK_ICD_FILENAMES="$icd"
    # The Vulkan loader says on stderr which driver it loads and whose
    # devices it lists (the app makes itself non-dumpable, so its
    # /proc/<pid>/maps is closed even to its own user).
    export VK_LOADER_DEBUG=all
    if start_app "$HOME/data-lavapipe" lavapipe; then
        sleep 10
        shot lavapipe
        if ! kill -0 "$app_pid" 2>/dev/null; then
            record info "lavapipe: the app exited on Mesa's software Vulkan driver (app-lavapipe.log)"
        elif ! grep -q 'Device Extension: .*libvulkan_lvp' "$out/app-lavapipe.log"; then
            record info "lavapipe: the Vulkan loader did not load Mesa's software Vulkan driver (lavapipe.png, app-lavapipe.log)"
        elif drawn lavapipe; then
            record info "lavapipe: the window draws on Mesa's software Vulkan driver (lavapipe.png)"
        else
            record info "lavapipe: the window appears on Mesa's software Vulkan driver (loaded) but the screen stays black (lavapipe.png, spread of grey $(spread_of lavapipe)): GPUI's frames on lavapipe do not reach Xvfb, so the check runs on Mesa's OpenGL"
        fi
    else
        record info "lavapipe: no window on Mesa's software Vulkan driver"
    fi
    stop_app
    keep_log "$HOME/data-lavapipe" lavapipe
}

session_keyring() {
    # The login keyring as the default collection, unlocked, as PAM leaves
    # it in a desktop session; without a default collection the Secret
    # Service answers "no storage access", and an empty password makes GNOME
    # Keyring ask through a prompt nobody can answer here.
    mkdir -p "$HOME/.local/share/keyrings"
    printf 'login' > "$HOME/.local/share/keyrings/default"
    eval "$(printf 'keyring-%s' "$RANDOM$RANDOM" | gnome-keyring-daemon --unlock --components=secrets)"
    export GNOME_KEYRING_CONTROL
    local data="$HOME/data-keyring" password="desktop-check-$RANDOM-$RANDOM"
    if ! start_app "$data" keyring-first-run; then
        record FAIL "keyring: the window did not appear"
        stop_app
        return
    fi
    shot keyring-1-first-run
    if drawn keyring-1-first-run; then
        record ok "keyring: the packaged app starts and draws its window (keyring-1-first-run.png)"
    else
        record FAIL "keyring: the window shows no picture (keyring-1-first-run.png)"
    fi
    # Focus is in the first password field (UI 13.1); Tab goes to the second.
    xdotool type --delay 40 "$password"
    xdotool key Tab
    xdotool type --delay 40 "$password"
    xdotool key Return
    for _ in $(seq 1 40); do
        device_key_stored && break
        sleep 0.5
    done
    sleep 3
    shot keyring-2-vault
    if device_key_stored; then
        record ok "keyring: first run created a vault and stored the device key in the Secret Service"
    else
        record FAIL "keyring: no device key in the Secret Service after the first run"
    fi

    xdotool key ctrl+shift+l
    sleep 3
    shot keyring-3-locked
    xdotool type --delay 40 "$password"
    xdotool key Return
    sleep 5
    shot keyring-4-unlocked
    # The core logs "locked" and then "unlocked" (lock_ops).
    if awk '/lock_ops: locked/ { locked = 1 } locked && /lock_ops: unlocked/ { found = 1 } END { exit !found }' "$data/logs/shellpp.log"; then
        record ok "keyring: lock (Ctrl+Shift+L) and unlock with the password (keyring-3-locked.png, keyring-4-unlocked.png)"
    else
        record FAIL "keyring: no lock and unlock in the log (keyring-3-locked.png, keyring-4-unlocked.png)"
    fi
    stop_app
    keep_log "$data" keyring-first-run

    if start_app "$data" keyring-restart; then
        shot keyring-5-restart
        if grep -q 'unlocked with the device key' "$data/logs/shellpp.log" 2>/dev/null; then
            record ok "keyring: the restart opened the vault with the device key from the Secret Service (keyring-5-restart.png)"
        else
            record FAIL "keyring: the restart did not open the vault with the device key (keyring-5-restart.png)"
        fi
    else
        record FAIL "keyring: the window did not appear after a restart"
        stop_app
        return
    fi
    connect_and_list keyring "$data/logs/shellpp.log"
    stop_app
    keep_log "$data" keyring-restart
}

session_no_keyring() {
    if start_app "$HOME/data-no-keyring" no-keyring; then
        shot no-keyring-first-run
        record ok "no-keyring: the app starts without a Secret Service (no-keyring-first-run.png: no \"Remember on this device\")"
    else
        record FAIL "no-keyring: the window did not appear without a Secret Service"
    fi
    stop_app
    keep_log "$HOME/data-no-keyring" no-keyring
}

session_no_dbus() {
    unset DBUS_SESSION_BUS_ADDRESS
    if start_app "$HOME/data-no-dbus" no-dbus; then
        shot no-dbus-first-run
        record ok "no-dbus: the app starts without a D-Bus session"
    else
        record FAIL "no-dbus: the window did not appear without a D-Bus session"
    fi
    stop_app
    keep_log "$HOME/data-no-dbus" no-dbus
}

# The build with the `screenshots` feature on demo data, in both themes.
session_screens() {
    local theme
    for theme in light dark; do
        if ! start_app "" "demo-$theme" --demo --theme "$theme"; then
            record FAIL "demo-$theme: no window"
            stop_app
            continue
        fi
        # The demo profile is generated and unlocked first (a debug build);
        # icons rasterized after the first frames show at the next one, which
        # a pointer move asks for.
        sleep 20
        xdotool mousemove --window "$window" 700 30
        sleep 2
        xdotool mousemove --window "$window" 1270 790
        sleep 2
        shot "demo-$theme"
        if drawn "demo-$theme"; then
            record ok "demo-$theme: the hosts of the demo data in the $theme theme (demo-$theme.png, grey mean $(mean_of "demo-$theme"))"
        else
            record FAIL "demo-$theme: the screen shows no picture (demo-$theme.png)"
        fi
        connect_and_list "demo-$theme" "$out/app-demo-$theme.log"
        stop_app
    done
    if [ -f "$out/demo-light.png" ] && [ -f "$out/demo-dark.png" ] \
        && awk -v d="$(mean_of demo-dark)" -v l="$(mean_of demo-light)" 'BEGIN { exit !(d < l - 0.2) }'; then
        record ok "the dark theme is darker than the light one (by more than 0.2 of grey)"
    else
        record FAIL "the dark capture is not clearly darker than the light one"
    fi
}

if [ "$session" = "tester" ]; then
    export XDG_RUNTIME_DIR
    XDG_RUNTIME_DIR="$(mktemp -d)"
    chmod 700 "$XDG_RUNTIME_DIR"
    export DISPLAY=:99
    Xvfb :99 -screen 0 1280x800x24 -nolisten tcp > "$out/xvfb.log" 2>&1 &
    for _ in $(seq 1 40); do
        xdpyinfo > /dev/null 2>&1 && break
        sleep 0.25
    done
    # Mesa's OpenGL for every start but the lavapipe session: with a Vulkan
    # driver in sight, wgpu takes it, and under Xvfb lavapipe draws nothing.
    export VK_ICD_FILENAMES=/nonexistent
    if [ "$mode" = "screens" ]; then
        dbus-run-session -- bash "$self" --session screens "$mode" "$target" "$out"
        exit 0
    fi
    bash "$self" --session lavapipe "$mode" "$target" "$out"
    # A session bus that cannot start the keyring on demand: the image's
    # D-Bus would start GNOME Keyring as soon as the app asks for it.
    bare="$(mktemp)"
    cat > "$bare" <<'EOF'
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
    dbus-run-session -- bash "$self" --session keyring "$mode" "$target" "$out"
    dbus-run-session --config-file="$bare" -- bash "$self" --session no-keyring "$mode" "$target" "$out"
    bash "$self" --session no-dbus "$mode" "$target" "$out"
    exit 0
fi
case "$session" in
    "") ;;
    lavapipe) session_lavapipe; exit 0 ;;
    keyring) session_keyring; exit 0 ;;
    no-keyring) session_no_keyring; exit 0 ;;
    no-dbus) session_no_dbus; exit 0 ;;
    screens) session_screens; exit 0 ;;
    *) echo "desktop-check: unknown session $session"; exit 2 ;;
esac

# --- root: the app, the files, OpenSSH --------------------------------------

mkdir -p "$out"
chmod 0777 "$out"
: > "$results"
chmod 0666 "$results"
if [ "$mode" = "screens" ]; then
    app="$target"
    expected=7
else
    work="$(mktemp -d)"
    chmod 0755 "$work"
    tar -xzf "$target" -C "$work"
    dir="$(find "$work" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
    app="$dir/shellpp"
    expected=10
    note "package: $(basename "$target"), files: $(find "$dir" -mindepth 1 -printf '%f ')"
    if desktop-file-validate "$dir/shellpp.desktop" > "$out/desktop-file-validate.log" 2>&1; then
        record ok "shellpp.desktop passes desktop-file-validate"
    else
        record FAIL "shellpp.desktop: desktop-file-validate (desktop-file-validate.log)"
    fi
    if ldd "$app" > "$out/ldd.log" 2>&1 && ! grep -q 'not found' "$out/ldd.log"; then
        record ok "every library the executable links is in the system (ldd.log)"
    else
        record FAIL "a library the executable links is missing (ldd.log)"
    fi
fi

# OpenSSH on 127.0.0.1:2222 for `sshuser`, with a password made now.
password_file="$(mktemp)"
chmod 0644 "$password_file"
head -c 18 /dev/urandom | base64 | tr -d '/+=' > "$password_file"
echo "sshuser:$(cat "$password_file")" | chpasswd
mkdir -p /home/sshuser/backups /home/sshuser/projects
echo "made by desktop-check" > /home/sshuser/notes.txt
chown -R sshuser:sshuser /home/sshuser
# OpenSSH's SFTP server with its log (`-e -l INFO`: what it opens, on its
# stderr) in a file the checks read.
sftp_server_log=/var/log/sftp-check.log
: > "$sftp_server_log"
chmod 0666 "$sftp_server_log"
cat > /usr/local/bin/sftp-check-server <<EOF
#!/bin/sh
exec /usr/lib/openssh/sftp-server -e -l INFO 2>> $sftp_server_log
EOF
chmod 0755 /usr/local/bin/sftp-check-server
sed -i 's|^Subsystem[[:space:]]\+sftp[[:space:]].*|Subsystem sftp /usr/local/bin/sftp-check-server|' /etc/ssh/sshd_config
ssh-keygen -A > /dev/null
/usr/sbin/sshd -p 2222 -o ListenAddress=127.0.0.1 -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=no

runuser -u tester -- env APP="$app" SSH_PASSWORD_FILE="$password_file" SFTP_LOG="$sftp_server_log" HOME=/home/tester \
    bash "$self" --session tester "$mode" "$target" "$out"

echo
cat "$results"
# Every check must have run and passed: a session that never started (a
# failed dbus-run-session or runuser) leaves its lines out instead of FAIL.
passed="$(grep -c '^ok ' "$results")"
if [ "$passed" -ne "$expected" ]; then
    echo "desktop-check: $passed of $expected checks passed"
    exit 1
fi
! grep -q '^FAIL' "$results"
