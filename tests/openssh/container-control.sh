#!/bin/sh
# Server-side faults of the OpenSSH test server (tests/openssh, README.md),
# installed in the image as /usr/local/bin/shellpp-openssh-control and run by
# control.sh on the host through `docker exec`:
#
#   shellpp-openssh-control rotate-host-key
#       generates new host keys (every type), restarts the listening sshd with
#       them (SIGHUP: sshd re-executes itself and loads the new keys; sessions
#       that are already open keep running) and waits until the server offers
#       the new Ed25519 key. Prints the new "<type> <SHA256 fingerprint>"
#       lines, the same as /run/shellpp/host_keys.
#
#   shellpp-openssh-control drop-connections
#       kills every connection process (sshd-session and sshd-auth, or the
#       per-connection sshd of OpenSSH before 9.8) with SIGKILL, so every
#       client sees its TCP connection closed without an exit status. The
#       listener keeps running: new connections are accepted at once.
#       Prints the number of processes killed.
#
# Exit status: 0 done; 1 failed; 2 usage. Nothing secret is printed.

set -eu

state=/run/shellpp
pid_file=/run/sshd.pid

fail() {
    echo "shellpp-openssh-control: $*" >&2
    exit 1
}

# write_host_keys: the types and fingerprints of the current host keys.
write_host_keys() {
    mkdir -p "$state"
    : >"$state/host_keys.tmp"
    for pub in /etc/ssh/ssh_host_*_key.pub; do
        key_type=$(cut -d ' ' -f 1 "$pub")
        fingerprint=$(ssh-keygen -l -E sha256 -f "$pub" | cut -d ' ' -f 2)
        printf '%s %s\n' "$key_type" "$fingerprint" >>"$state/host_keys.tmp"
    done
    mv "$state/host_keys.tmp" "$state/host_keys"
}

listener_pid() {
    [ -s "$pid_file" ] || fail "no $pid_file (is sshd running?)"
    pid=$(head -n 1 "$pid_file")
    case "$pid" in '' | *[!0-9]*) fail "unexpected content in $pid_file" ;; esac
    printf '%s\n' "$pid"
}

rotate_host_key() {
    pid=$(listener_pid)
    rm -f /etc/ssh/ssh_host_*
    ssh-keygen -A >/dev/null
    /usr/sbin/sshd -t || fail 'the new host keys do not load'
    write_host_keys
    expected=$(cut -d ' ' -f 2 /etc/ssh/ssh_host_ed25519_key.pub)
    kill -HUP "$pid"
    # The restarted listener answers with the new key; until then the port is
    # briefly closed or still served by the old process. ssh-keyscan also
    # prints a "# host:port SSH-2.0-..." comment line on stdout (OpenSSH
    # 9.8 and later); only the key line counts.
    tries=0
    while [ "$tries" -lt 50 ]; do
        offered=$(ssh-keyscan -T 2 -t ed25519 -p 22 127.0.0.1 2>/dev/null | awk '$2 == "ssh-ed25519" { print $3; exit }' || true)
        if [ "$offered" = "$expected" ]; then
            cat "$state/host_keys"
            return 0
        fi
        tries=$((tries + 1))
        sleep 0.2
    done
    fail 'sshd did not come back with the new host key'
}

drop_connections() {
    listener=$(listener_pid)
    count=0
    for name in sshd-session sshd-auth sshd; do
        for pid in $(pgrep -x "$name" || true); do
            [ "$pid" = "$listener" ] && continue
            if kill -KILL "$pid" 2>/dev/null; then
                count=$((count + 1))
            fi
        done
    done
    printf '%s\n' "$count"
}

if [ "$#" -ne 1 ]; then
    echo 'usage: shellpp-openssh-control rotate-host-key | drop-connections' >&2
    exit 2
fi

case "$1" in
    rotate-host-key) rotate_host_key ;;
    drop-connections) drop_connections ;;
    *)
        echo 'usage: shellpp-openssh-control rotate-host-key | drop-connections' >&2
        exit 2
        ;;
esac
