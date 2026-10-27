#!/usr/bin/env bash
# The packaged Linux app in a clean Ubuntu 22.04 (tests/linux/desktop.Dockerfile;
# scripts/verify-linux-desktop.ps1; D225, item 3). Under Xvfb with Mesa's
# OpenGL (llvmpipe) it starts the executable of the tar.gz as the user
# `tester` and drives it with xdotool, in three sessions:
#
#   keyring     a D-Bus session with GNOME Keyring as the Secret Service:
#               first run (a new vault, "Remember on this device" on), the
#               device key in the Secret Service, lock and unlock, a restart
#               that opens without the password, a quick connect to OpenSSH
#               on 127.0.0.1 (the host key, the password, a command);
#   no-keyring  a D-Bus session without a Secret Service: the app starts and
#               offers no "Remember on this device";
#   no-dbus     no D-Bus session at all: the app starts.
#
# It starts as root: OpenSSH for `sshuser` with a password made here. The
# screenshots of each state, the app's output and its log go to <out-dir>;
# results.txt says which checks passed. Usage (as root):
#
#   desktop-check.sh <shellpp-<version>-linux-x64.tar.gz> <out-dir>
set -uo pipefail

if [ "${1:-}" = "--session" ]; then
    session="$2"
    shift 2
else
    session=""
fi
pkg="${1:?usage: desktop-check.sh <package.tar.gz> <out-dir>}"
out="${2:?usage: desktop-check.sh <package.tar.gz> <out-dir>}"
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

# Starts the app with data in $1; sets $app_pid and $window. $2: a name for
# its output.
start_app() {
    local started=$SECONDS
    SHELLPP_DATA_DIR="$1" "$APP" > "$out/app-$2.log" 2>&1 &
    app_pid=$!
    if window="$(window_of_app 90)"; then
        note "$2: window $window after $((SECONDS - started)) s"
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

# Whether the Secret Service holds an item of the service `shellpp`; its
# attributes (never the secret) go to secret-tool.txt.
device_key_stored() {
    secret-tool search --all service shellpp 2>&1 | grep -v '^secret' > "$out/secret-tool.txt"
    grep -q 'shellpp' "$out/secret-tool.txt"
}

keep_log() { cp "$1/logs/shellpp.log" "$out/shellpp-$2.log" 2>/dev/null; }

# Whether a screenshot shows a picture: a black or empty screen (frames that
# never reach Xvfb, as with Mesa's lavapipe) has almost no spread of grey.
drawn() {
    local spread
    spread="$(convert "$out/$1.png" -colorspace Gray -format '%[fx:standard_deviation]' info: 2>/dev/null)"
    awk -v s="${spread:-0}" 'BEGIN { exit !(s > 0.02) }'
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

    # Quick connect to OpenSSH on this machine (the search field, UI 2.1).
    click 640 78
    xdotool type --delay 40 "sshuser@127.0.0.1:2222"
    xdotool key Return
    sleep 6
    shot keyring-6-host-key
    # The host key of a first connection (UI 10.2): Enter takes the default.
    xdotool key Return
    sleep 4
    shot keyring-7-password
    xdotool type --delay 40 "$(cat "$SSH_PASSWORD_FILE")"
    xdotool key Return
    sleep 6
    xdotool type --delay 40 "ls -la; echo desktop-check-ok"
    xdotool key Return
    sleep 4
    shot keyring-8-terminal
    if grep -q "connected session=" "$data/logs/shellpp.log" 2>/dev/null; then
        record ok "keyring: quick connect reached OpenSSH and logged in (keyring-8-terminal.png)"
    else
        record FAIL "keyring: quick connect did not log in to OpenSSH (keyring-6..8 screenshots)"
    fi

    # SFTP to the same server: the host saved from the search ("Save as
    # host", UI 2.1), then the SFTP tab's remote panel (UI 5). The positions
    # are those of the 1280x800 window.
    click 93 22            # the Vaults tab
    click 640 78           # the search field, which keeps the last address
    xdotool key ctrl+a
    xdotool type --delay 40 "sshuser@127.0.0.1:2222"
    sleep 2
    click 737 499          # "Save as host"
    sleep 3
    shot keyring-9-saved-host
    xdotool key Escape
    sleep 1
    click 177 22           # the SFTP tab
    sleep 2
    click 960 480          # "Choose host..." of the remote panel
    sleep 2
    xdotool type --delay 40 "127.0.0.1"
    sleep 1
    click 865 334          # the host in the picker
    sleep 5
    shot keyring-10-sftp-connect
    # The panel's password prompt (the test password is made per run and
    # lives only in this container).
    xdotool type --delay 40 "$(cat "$SSH_PASSWORD_FILE")"
    xdotool key Return
    sleep 6
    shot keyring-11-sftp-list
    # The SFTP session's own id: `session opened session=N kind="sftp"`,
    # then `connected session=N`.
    local sftp_id
    sftp_id="$(sed -n 's/.*session opened session=\([0-9]*\) kind="sftp".*/\1/p' "$data/logs/shellpp.log" | tail -n 1)"
    if [ -n "$sftp_id" ] && grep -q "connected session=$sftp_id " "$data/logs/shellpp.log"; then
        record ok "keyring: the SFTP panel connected to OpenSSH (keyring-11-sftp-list.png should list backups, projects, notes.txt)"
    else
        record FAIL "keyring: the SFTP panel did not connect (keyring-9..11 screenshots)"
    fi
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
    dbus-run-session -- bash "$self" --session keyring "$pkg" "$out"
    dbus-run-session --config-file="$bare" -- bash "$self" --session no-keyring "$pkg" "$out"
    bash "$self" --session no-dbus "$pkg" "$out"
    exit 0
fi
if [ -n "$session" ]; then
    "session_${session//-/_}"
    exit 0
fi

# --- root: the package, the files, OpenSSH ----------------------------------

mkdir -p "$out"
chmod 0777 "$out"
: > "$results"
chmod 0666 "$results"
work="$(mktemp -d)"
chmod 0755 "$work"
tar -xzf "$pkg" -C "$work"
dir="$(find "$work" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
note "package: $(basename "$pkg"), files: $(find "$dir" -mindepth 1 -printf '%f ')"

if desktop-file-validate "$dir/shellpp.desktop" > "$out/desktop-file-validate.log" 2>&1; then
    record ok "shellpp.desktop passes desktop-file-validate"
else
    record FAIL "shellpp.desktop: desktop-file-validate (desktop-file-validate.log)"
fi
if ldd "$dir/shellpp" > "$out/ldd.log" 2>&1 && ! grep -q 'not found' "$out/ldd.log"; then
    record ok "every library the executable links is in the system (ldd.log)"
else
    record FAIL "a library the executable links is missing (ldd.log)"
fi

# OpenSSH on 127.0.0.1:2222 for `sshuser`, with a password made now.
password_file="$(mktemp)"
chmod 0644 "$password_file"
head -c 18 /dev/urandom | base64 | tr -d '/+=' > "$password_file"
echo "sshuser:$(cat "$password_file")" | chpasswd
mkdir -p /home/sshuser/backups /home/sshuser/projects
echo "made by desktop-check" > /home/sshuser/notes.txt
chown -R sshuser:sshuser /home/sshuser
ssh-keygen -A > /dev/null
/usr/sbin/sshd -p 2222 -o ListenAddress=127.0.0.1 -o PasswordAuthentication=yes -o KbdInteractiveAuthentication=no

runuser -u tester -- env APP="$dir/shellpp" SSH_PASSWORD_FILE="$password_file" HOME=/home/tester \
    bash "$self" --session tester "$pkg" "$out"

echo
cat "$results"
# Every check must have run and passed: a session that never started (a
# failed dbus-run-session or runuser) leaves its lines out instead of FAIL.
expected=10
passed="$(grep -c '^ok ' "$results")"
if [ "$passed" -ne "$expected" ]; then
    echo "desktop-check: $passed of $expected checks passed"
    exit 1
fi
! grep -q '^FAIL' "$results"
