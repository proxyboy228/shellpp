#!/usr/bin/env bash
# The stage 5 lab (tests/openssh, README.md): OpenSSH servers behind a jump
# host, in a Docker network of their own without a way out.
#
#   bash tests/openssh/lab.sh up <workdir>     start; writes <workdir>/lab.json
#   bash tests/openssh/lab.sh down <workdir>   remove the containers and the network
#
# Layout (container names get a random suffix; the names below are the
# hosts' names inside the lab network):
#
#   127.0.0.1:<port> --> jump1 --+--> jump2
#                                +--> target   (busybox httpd on port 8080)
#                                +--> app1, app2, app3
#   127.0.0.1:<port> --> proxy (tinyproxy on 3128, microsocks on 1080)
#                                +--> every server of the lab
#
# Only jump1 is published (on 127.0.0.1, a random port); the lab network is
# `--internal`: the other servers cannot be reached from this machine nor
# reach anything outside the lab. Every server has the same three users as
# start.sh (pwuser, keyuser, kiuser) with the credentials generated for this
# lab, and its own host keys (generated at its start). `target` also serves
# HTTP on port 8080 (the answer is lab.json's "http_body"). The proxies (image
# proxy/Dockerfile) need the lab's generated proxy user and password.
#
# Exit status: 0 done; 1 failed; 2 usage; 3 Docker is not available.
# Environment: SHELLPP_OPENSSH_IMAGE, SHELLPP_OPENSSH_LIFETIME,
# SHELLPP_OPENSSH_TIMEOUT as for start.sh.

set -euo pipefail

readonly LABEL='dev.shellpp.test=openssh-lab'
readonly SERVERS='jump1 jump2 target app1 app2 app3'

log() {
    printf 'openssh/lab.sh: %s\n' "$*" >&2
}

die() {
    log "$*"
    exit 1
}

if [ "$#" -ne 2 ] || [ -z "$2" ]; then
    printf 'usage: bash %s up|down <workdir>\n' "$0" >&2
    exit 2
fi
command="$1"
export MSYS_NO_PATHCONV=1

if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    log 'Docker is not available (docker info failed)'
    exit 3
fi

mkdir -p "$2"
workdir="$(cd "$2" && pwd)"

down() {
    if [ -f "$workdir/containers" ]; then
        while read -r name; do
            [ -n "$name" ] && docker rm -f "$name" >/dev/null 2>&1 || true
        done <"$workdir/containers"
        rm -f "$workdir/containers"
    fi
    if [ -f "$workdir/network" ]; then
        docker network rm "$(cat "$workdir/network")" >/dev/null 2>&1 || true
        rm -f "$workdir/network"
    fi
}

case "$command" in
    down)
        down
        exit 0
        ;;
    up) ;;
    *)
        printf 'usage: bash %s up|down <workdir>\n' "$0" >&2
        exit 2
        ;;
esac

lifetime="${SHELLPP_OPENSSH_LIFETIME:-3600}"
wait_seconds="${SHELLPP_OPENSSH_TIMEOUT:-60}"
case "$lifetime" in '' | *[!0-9]*) die 'SHELLPP_OPENSSH_LIFETIME must be a number of seconds' ;; esac
case "$wait_seconds" in '' | *[!0-9]*) die 'SHELLPP_OPENSSH_TIMEOUT must be a number of seconds' ;; esac
command -v ssh-keygen >/dev/null 2>&1 || die 'ssh-keygen is required to generate the client key'

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

random_hex() {
    od -An -N16 -tx1 /dev/urandom | tr -d ' \n'
}

json_string() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '"%s"' "$value"
}

umask 077
rm -f "$workdir/lab.json"
ok=0
trap 'rm -rf "$workdir/container-input"; if [ "$ok" -ne 1 ]; then down; fi' EXIT

image="${SHELLPP_OPENSSH_IMAGE:-}"
if [ -z "$image" ]; then
    image='shellpp-test-openssh:local'
    log "building $image"
    docker build --quiet --tag "$image" "$here" >/dev/null || die 'docker build failed'
elif ! docker image inspect "$image" >/dev/null 2>&1; then
    die "image $image not found"
fi
proxy_image="${SHELLPP_OPENSSH_PROXY_IMAGE:-}"
if [ -z "$proxy_image" ]; then
    proxy_image='shellpp-test-proxy:local'
    log "building $proxy_image"
    docker build --quiet --tag "$proxy_image" "$here/proxy" >/dev/null || die 'docker build of the proxies failed'
elif ! docker image inspect "$proxy_image" >/dev/null 2>&1; then
    die "image $proxy_image not found"
fi

suffix="$(random_hex | cut -c 1-10)"
network="shellpp-lab-$suffix"
docker network create --internal --label "$LABEL" "$network" >/dev/null || die 'docker network create failed'
printf '%s\n' "$network" >"$workdir/network"
: >"$workdir/containers"

# The credentials of this lab (one set for every server).
password_secret="$(random_hex)"
ki_secret="$(random_hex)"
http_body="shellpp-lab-$(random_hex | cut -c 1-12)"
key="$workdir/client_ed25519"
rm -f "$key" "$key.pub"
ssh-keygen -q -t ed25519 -N '' -C 'shellpp-openssh-lab' -f "$key" >/dev/null || die 'ssh-keygen failed'
input="$workdir/container-input"
rm -rf "$input"
mkdir -p "$input/shellpp-input"
printf '%s\n' "$password_secret" >"$input/shellpp-input/pwuser.password"
printf '%s\n' "$ki_secret" >"$input/shellpp-input/kiuser.password"
cp "$key.pub" "$input/shellpp-input/keyuser.authorized_keys"
proxy_user="proxy$(random_hex | cut -c 1-6)"
proxy_secret="$(random_hex)"

for server in $SERVERS; do
    name="shellpp-lab-$server-$suffix"
    if [ "$server" = jump1 ]; then
        # The way in: published on loopback, on the default network; joins
        # the lab network below.
        docker create --init --name "$name" --label "$LABEL" --hostname "$server" \
            --publish 127.0.0.1::22 \
            --env "SHELLPP_OPENSSH_LIFETIME=$lifetime" "$image" >/dev/null ||
            die "docker create $server failed"
    else
        docker create --init --name "$name" --label "$LABEL" --hostname "$server" \
            --network "$network" --network-alias "$server" \
            --env "SHELLPP_OPENSSH_LIFETIME=$lifetime" "$image" >/dev/null ||
            die "docker create $server failed"
    fi
    printf '%s\n' "$name" >>"$workdir/containers"
    COPYFILE_DISABLE=1 tar -C "$input" -cf - shellpp-input | docker cp - "$name:/run" >/dev/null ||
        die "copying the credentials into $server failed"
    if [ "$server" = jump1 ]; then
        docker network connect --alias jump1 "$network" "$name" >/dev/null ||
            die 'connecting jump1 to the lab network failed'
    fi
    docker start "$name" >/dev/null || die "docker start $server failed"
done
rm -rf "$input"

# The proxies: published on loopback, reaching the lab network.
proxy_name="shellpp-lab-proxy-$suffix"
docker create --init --name "$proxy_name" --label "$LABEL" --hostname proxy \
    --publish 127.0.0.1::3128 --publish 127.0.0.1::1080 \
    --env "SHELLPP_OPENSSH_LIFETIME=$lifetime" "$proxy_image" >/dev/null ||
    die 'docker create proxy failed'
printf '%s\n' "$proxy_name" >>"$workdir/containers"
mkdir -p "$input/shellpp-input"
printf '%s\n' "$proxy_user" >"$input/shellpp-input/proxy.user"
printf '%s\n' "$proxy_secret" >"$input/shellpp-input/proxy.password"
COPYFILE_DISABLE=1 tar -C "$input" -cf - shellpp-input | docker cp - "$proxy_name:/run" >/dev/null ||
    die 'copying the proxy credentials failed'
rm -rf "$input"
docker network connect "$network" "$proxy_name" >/dev/null || die 'connecting the proxy failed'
docker start "$proxy_name" >/dev/null || die 'docker start proxy failed'
http_mapping="$(docker port "$proxy_name" 3128/tcp | head -n 1)" || die 'reading the proxy port failed'
socks_mapping="$(docker port "$proxy_name" 1080/tcp | head -n 1)" || die 'reading the proxy port failed'
http_port="${http_mapping##*:}"
socks_port="${socks_mapping##*:}"

name_of() {
    printf 'shellpp-lab-%s-%s' "$1" "$suffix"
}

# The HTTP service of `target` (busybox httpd in the foreground, detached).
docker exec "$(name_of target)" sh -c "mkdir -p /srv/www && printf '%s\n' '$http_body' >/srv/www/index.html" ||
    die 'writing the HTTP page failed'
docker exec -d "$(name_of target)" busybox httpd -f -p 8080 -h /srv/www || die 'starting httpd failed'

mapping="$(docker port "$(name_of jump1)" 22/tcp | head -n 1)" || die 'reading the published port failed'
port="${mapping##*:}"
case "$port" in '' | *[!0-9]*) die 'jump1 has no published SSH port' ;; esac

# Every server is ready once it has written its host keys (the entrypoint
# writes them just before sshd starts); jump1 also answers on its port.
deadline=$((SECONDS + wait_seconds))
host_keys_json=''
for server in $SERVERS; do
    lines=''
    while [ "$SECONDS" -lt "$deadline" ]; do
        lines="$(docker exec "$(name_of "$server")" cat /run/shellpp/host_keys 2>/dev/null || true)"
        if [ -n "$lines" ] &&
            docker exec "$(name_of "$server")" sh -c 'pgrep -x sshd >/dev/null' 2>/dev/null; then
            break
        fi
        lines=''
        sleep 1
    done
    [ -n "$lines" ] || die "$server did not start"
    keys=''
    while read -r key_type fingerprint; do
        [ -n "$key_type" ] || continue
        keys="${keys:+$keys,}$(json_string "$key_type"):$(json_string "$fingerprint")"
    done <<<"$lines"
    host_keys_json="${host_keys_json:+$host_keys_json,}$(json_string "$server"):{$keys}"
done

{
    printf '{\n'
    printf '  "host": "127.0.0.1",\n'
    printf '  "port": %s,\n' "$port"
    printf '  "host_keys": {%s},\n' "$host_keys_json"
    printf '  "users": {\n'
    printf '    "password": {"name": "pwuser", "password": %s},\n' "$(json_string "$password_secret")"
    printf '    "publickey": {"name": "keyuser", "private_key_path": %s},\n' "$(json_string "$key")"
    printf '    "keyboard_interactive": {"name": "kiuser", "password": %s}\n' "$(json_string "$ki_secret")"
    printf '  },\n'
    printf '  "http_body": %s,\n' "$(json_string "$http_body")"
    printf '  "proxy": {"http_port": %s, "socks_port": %s, "user": %s, "password": %s},\n' \
        "$http_port" "$socks_port" "$(json_string "$proxy_user")" "$(json_string "$proxy_secret")"
    printf '  "network": %s\n' "$(json_string "$network")"
    printf '}\n'
} >"$workdir/lab.json.tmp"
mv "$workdir/lab.json.tmp" "$workdir/lab.json"
ok=1
log "lab ready: jump1 on 127.0.0.1:$port"
printf '%s\n' "$workdir/lab.json"
