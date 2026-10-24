#!/usr/bin/env bash
# Starts the OpenSSH test server in Docker (tests/openssh, README.md).
#
#   bash tests/openssh/start.sh <workdir>
#
# 1. Builds the image from this directory (Docker's layer cache makes repeated
#    builds quick), unless SHELLPP_OPENSSH_IMAGE names a prebuilt image.
# 2. Generates the credentials of this run in <workdir>: two random passwords
#    and an Ed25519 client key without passphrase (client_ed25519 and
#    client_ed25519.pub).
# 3. Creates a container that publishes port 22 on 127.0.0.1:<random port>,
#    copies the credentials into it and starts it; the entrypoint creates the
#    users and generates the host keys.
# 4. Waits until the server sends its SSH identification line and writes
#    <workdir>/server.json (see README.md). Its path is printed on stdout.
#
# stop.sh <workdir> removes the container. Progress and errors go to stderr;
# secrets are never printed.
#
# Exit status: 0 started; 1 failed; 2 usage; 3 Docker is not available.
#
# Environment:
#   SHELLPP_OPENSSH_IMAGE     prebuilt image to run instead of building
#                             shellpp-test-openssh:local
#   SHELLPP_OPENSSH_PROGRAMS  1: run the image with the terminal programs of
#                             the compatibility screenshots
#                             (programs.Dockerfile on top of the image above)
#   SHELLPP_OPENSSH_LIFETIME  seconds after which the container stops by
#                             itself (default 3600)
#   SHELLPP_OPENSSH_TIMEOUT   seconds to wait for the server (default 60)
#   SHELLPP_TEST_RUN          the run or agent; the container is labelled
#                             shellpp-run=<it> (default local), so stop.sh
#                             --all removes only this run's containers
#   SHELLPP_OPENSSH_VARIANT   a variant of the server (variants/<name>.sh,
#                             README.md "Variants"; default none)

set -euo pipefail

readonly LABEL='dev.shellpp.test=openssh'
readonly RUN_LABEL="shellpp-run=${SHELLPP_TEST_RUN:-local}"
readonly PASSWORD_USER='pwuser'
readonly KEY_USER='keyuser'
readonly KI_USER='kiuser'

log() {
    printf 'openssh/start.sh: %s\n' "$*" >&2
}

die() {
    log "$*"
    exit 1
}

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    printf 'usage: bash %s <workdir>\n' "$0" >&2
    exit 2
fi

lifetime="${SHELLPP_OPENSSH_LIFETIME:-3600}"
variant="${SHELLPP_OPENSSH_VARIANT:-}"
case "$variant" in *[!a-z0-9-]*) die 'SHELLPP_OPENSSH_VARIANT names no variant' ;; esac
wait_seconds="${SHELLPP_OPENSSH_TIMEOUT:-60}"
case "$lifetime" in '' | *[!0-9]*) die 'SHELLPP_OPENSSH_LIFETIME must be a number of seconds' ;; esac
case "$wait_seconds" in '' | *[!0-9]*) die 'SHELLPP_OPENSSH_TIMEOUT must be a number of seconds' ;; esac

# Git for Windows: keep container paths such as <name>:/run as they are.
export MSYS_NO_PATHCONV=1

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    log 'Docker is not available (docker info failed)'
    exit 3
fi
command -v ssh-keygen >/dev/null 2>&1 || die 'ssh-keygen is required to generate the client key'

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# A path for programs outside the shell: on Git for Windows C:/... form.
native_path() {
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -m "$1"
    else
        printf '%s\n' "$1"
    fi
}

# Starts a container, again when publishing its port failed: Docker picks
# the random host port from the kernel's ephemeral range, so the port can be
# taken by a local socket meanwhile (rootless Docker then fails instead of
# picking another one).
start_container() {
    local tries=0
    until docker start "$1" >/dev/null 2>"$workdir/start-error"; do
        tries=$((tries + 1))
        if [ "$tries" -ge 5 ] || ! grep -q 'address already in use' "$workdir/start-error"; then
            cat "$workdir/start-error" >&2
            rm -f "$workdir/start-error"
            return 1
        fi
        sleep 1
    done
    rm -f "$workdir/start-error"
}

# 128 random bits as 32 hex digits.
random_hex() {
    od -An -N16 -tx1 /dev/urandom | tr -d ' \n'
}

# A JSON string literal (the values here never contain control characters).
json_string() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '"%s"' "$value"
}

# Prints the server's identification line, or nothing. Docker accepts TCP
# connections on the published port before sshd listens, so a successful
# connect alone does not mean the server is ready.
read_banner() {
    (
        exec 3<>"/dev/tcp/127.0.0.1/$1" || exit 1
        IFS= read -r -t 2 line <&3 || exit 1
        printf '%s\n' "$line"
    ) 2>/dev/null || true
}

umask 077
mkdir -p "$1"
workdir="$(cd "$1" && pwd)"
rm -f "$workdir/server.json"

image="${SHELLPP_OPENSSH_IMAGE:-}"
if [ -z "$image" ]; then
    image='shellpp-test-openssh:local'
    log "building $image"
    docker build --quiet --tag "$image" "$(native_path "$here")" >/dev/null || die 'docker build failed'
elif ! docker image inspect "$image" >/dev/null 2>&1; then
    die "image $image not found"
fi
case "${SHELLPP_OPENSSH_PROGRAMS:-}" in
'' | 0) ;;
1)
    base="$image"
    image='shellpp-test-openssh-programs:local'
    log "building $image"
    docker build --quiet --tag "$image" --build-arg "BASE=$base" \
        --file "$(native_path "$here/programs.Dockerfile")" "$(native_path "$here")" >/dev/null ||
        die 'docker build of the programs image failed'
    ;;
*) die 'SHELLPP_OPENSSH_PROGRAMS must be 1 or empty' ;;
esac

# Credentials of this run.
password_user_secret="$(random_hex)"
ki_user_secret="$(random_hex)"
key="$workdir/client_ed25519"
rm -f "$key" "$key.pub"
ssh-keygen -q -t ed25519 -N '' -C 'shellpp-openssh-test' -f "$key" >/dev/null || die 'ssh-keygen failed'

input="$workdir/container-input"
name="shellpp-openssh-$(random_hex | cut -c 1-12)"
ok=0
# On failure the container goes away; the copy of the credentials for the
# container never outlives this script.
trap 'rm -rf "$input"; if [ "$ok" -ne 1 ]; then docker rm -f "$name" >/dev/null 2>&1 || true; rm -f "$workdir/container"; fi' EXIT

rm -rf "$input"
mkdir -p "$input/shellpp-input"
printf '%s\n' "$password_user_secret" >"$input/shellpp-input/$PASSWORD_USER.password"
printf '%s\n' "$ki_user_secret" >"$input/shellpp-input/$KI_USER.password"
cp "$key.pub" "$input/shellpp-input/$KEY_USER.authorized_keys"
# kbd-2fa: the key user answers its keyboard-interactive question with the
# keyboard-interactive user's password.
if [ "$variant" = 'kbd-2fa' ]; then
    printf '%s\n' "$ki_user_secret" >"$input/shellpp-input/$KEY_USER.password"
fi

# --init: a minimal init as PID 1 reaps the processes of ended sessions
# (sshd runs under `timeout`, which does not), so no zombies show up in `ps`
# or htop.
docker create \
    --init \
    --name "$name" \
    --label "$LABEL" --label "$RUN_LABEL" \
    --hostname shellpp-openssh \
    --publish 127.0.0.1::22 \
    --env "SHELLPP_OPENSSH_LIFETIME=$lifetime" \
    --env "SHELLPP_OPENSSH_VARIANT=$variant" \
    "$image" >/dev/null || die 'docker create failed'
printf '%s\n' "$name" >"$workdir/container"

# The credentials enter the container as a tar stream, so no host path has to
# be shared with the Docker daemon.
COPYFILE_DISABLE=1 tar -C "$input" -cf - shellpp-input | docker cp - "$name:/run" >/dev/null ||
    die 'copying the credentials into the container failed'
rm -rf "$input"

start_container "$name" || die 'docker start failed'

mapping="$(docker port "$name" 22/tcp | head -n 1)" || die 'reading the published port failed'
port="${mapping##*:}"
case "$port" in '' | *[!0-9]*) die 'the container has no published SSH port' ;; esac

deadline=$((SECONDS + wait_seconds))
ready=0
while [ "$SECONDS" -lt "$deadline" ]; do
    banner="$(read_banner "$port")"
    case "$banner" in
        SSH-2.0-*)
            ready=1
            break
            ;;
    esac
    if [ "$(docker inspect --format '{{.State.Running}}' "$name" 2>/dev/null)" != 'true' ]; then
        break
    fi
    sleep 1
done
if [ "$ready" -ne 1 ]; then
    log 'the SSH server did not start; container output:'
    docker logs --tail 50 "$name" >&2 || true
    die "no SSH server on 127.0.0.1:$port"
fi

host_key_lines="$(docker exec "$name" cat /run/shellpp/host_keys)" || die 'reading the host keys failed'
host_keys=''
while read -r key_type fingerprint; do
    [ -n "$key_type" ] || continue
    case "$fingerprint" in SHA256:*) ;; *) die "unexpected fingerprint format for $key_type" ;; esac
    host_keys="${host_keys:+$host_keys,}$(json_string "$key_type"):$(json_string "$fingerprint")"
done <<<"$host_key_lines"
[ -n "$host_keys" ] || die 'the server reported no host keys'

{
    printf '{\n'
    printf '  "host": "127.0.0.1",\n'
    printf '  "port": %s,\n' "$port"
    printf '  "host_keys": {%s},\n' "$host_keys"
    printf '  "users": {\n'
    printf '    "password": {"name": %s, "password": %s},\n' \
        "$(json_string "$PASSWORD_USER")" "$(json_string "$password_user_secret")"
    printf '    "publickey": {"name": %s, "private_key_path": %s},\n' \
        "$(json_string "$KEY_USER")" "$(json_string "$(native_path "$key")")"
    printf '    "keyboard_interactive": {"name": %s, "password": %s}\n' \
        "$(json_string "$KI_USER")" "$(json_string "$ki_user_secret")"
    printf '  },\n'
    printf '  "variant": %s,\n' "$(json_string "$variant")"
    printf '  "container": %s\n' "$(json_string "$name")"
    printf '}\n'
} >"$workdir/server.json.tmp"
mv "$workdir/server.json.tmp" "$workdir/server.json"

ok=1
log "OpenSSH test server ready on 127.0.0.1:$port"
native_path "$workdir/server.json"
