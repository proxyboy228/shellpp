#!/usr/bin/env bash
# Server-side faults for tests against the OpenSSH test server (tests/openssh,
# README.md), on the container that start.sh <workdir> started:
#
#   bash tests/openssh/control.sh <workdir> rotate-host-key
#       new host keys of every type; the listening sshd restarts with them
#       (open sessions keep running). <workdir>/server.json gets the new
#       "host_keys"; its path is printed on stdout.
#   bash tests/openssh/control.sh <workdir> drop-connections
#       kills every connection process in the container: clients see their
#       connection closed without an exit status, the server keeps accepting
#       new connections. Prints the number of processes killed.
#   bash tests/openssh/control.sh <workdir> freeze | thaw
#       pauses or resumes the whole container (docker pause / unpause): open
#       connections stop answering without being closed (keepalive timeouts),
#       new connections hang. A frozen container stays frozen until thaw or
#       stop.sh.
#
# Progress and errors go to stderr; nothing secret is printed.
#
# Exit status: 0 done; 1 failed; 2 usage; 3 Docker is not available.

set -euo pipefail

# Git for Windows: keep container paths and names as they are.
export MSYS_NO_PATHCONV=1

usage() {
    printf 'usage: bash %s <workdir> rotate-host-key | drop-connections | freeze | thaw\n' "$0" >&2
    exit 2
}

die() {
    printf 'openssh/control.sh: %s\n' "$*" >&2
    exit 1
}

if [ "$#" -ne 2 ] || [ -z "$1" ]; then
    usage
fi
workdir="$1"
command="$2"
case "$command" in
    rotate-host-key | drop-connections | freeze | thaw) ;;
    *) usage ;;
esac

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    printf 'openssh/control.sh: Docker is not available (docker info failed)\n' >&2
    exit 3
fi

[ -f "$workdir/container" ] || die "$workdir/container not found (start the server with start.sh)"
name="$(head -n 1 "$workdir/container")"
case "$name" in
    shellpp-openssh-*) ;;
    *) die "$workdir/container does not name a test container" ;;
esac

# A path for programs outside the shell: on Git for Windows C:/... form.
native_path() {
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -m "$1"
    else
        printf '%s\n' "$1"
    fi
}

# A JSON string literal (the values here never contain control characters).
json_string() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '"%s"' "$value"
}

case "$command" in
    rotate-host-key)
        lines="$(docker exec "$name" /usr/local/bin/shellpp-openssh-control rotate-host-key)" ||
            die 'rotating the host key failed'
        host_keys=''
        while read -r key_type fingerprint; do
            [ -n "$key_type" ] || continue
            case "$fingerprint" in SHA256:*) ;; *) die "unexpected fingerprint format for $key_type" ;; esac
            host_keys="${host_keys:+$host_keys,}$(json_string "$key_type"):$(json_string "$fingerprint")"
        done <<<"$lines"
        [ -n "$host_keys" ] || die 'the server reported no host keys'
        # server.json keeps its layout (start.sh writes "host_keys" on one
        # line); only that line changes.
        json="$workdir/server.json"
        [ -f "$json" ] || die "$json not found"
        umask 077
        replaced=0
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
                '  "host_keys": '*)
                    printf '  "host_keys": {%s},\n' "$host_keys"
                    replaced=1
                    ;;
                *) printf '%s\n' "$line" ;;
            esac
        done <"$json" >"$json.tmp"
        if [ "$replaced" -ne 1 ]; then
            rm -f "$json.tmp"
            die "no host_keys line in $json"
        fi
        mv "$json.tmp" "$json"
        printf 'openssh/control.sh: new host keys for %s\n' "$name" >&2
        native_path "$json"
        ;;
    drop-connections)
        count="$(docker exec "$name" /usr/local/bin/shellpp-openssh-control drop-connections)" ||
            die 'dropping the connections failed'
        printf '%s\n' "$count"
        ;;
    freeze)
        docker pause "$name" >/dev/null || die 'docker pause failed'
        ;;
    thaw)
        docker unpause "$name" >/dev/null || die 'docker unpause failed'
        ;;
esac
