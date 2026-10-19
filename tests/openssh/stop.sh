#!/usr/bin/env bash
# Stops the OpenSSH test server (tests/openssh, README.md).
#
#   bash tests/openssh/stop.sh <workdir>   remove the container that
#                                          start.sh <workdir> started
#   bash tests/openssh/stop.sh --all       remove every container started by
#                                          start.sh (label dev.shellpp.test=openssh)
#
# Safe to run more than once. The files in <workdir> (server.json, the client
# key) belong to the caller, who removes the directory.
#
# Exit status: 0 done (also when there was nothing to remove); 1 failed;
# 2 usage.

set -euo pipefail

readonly LABEL='dev.shellpp.test=openssh'

# Git for Windows: keep Docker arguments as they are.
export MSYS_NO_PATHCONV=1

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    printf 'usage: bash %s <workdir> | --all\n' "$0" >&2
    exit 2
fi

if ! command -v docker >/dev/null 2>&1; then
    exit 0
fi

if [ "$1" = '--all' ]; then
    containers="$(docker ps --all --quiet --filter "label=$LABEL" 2>/dev/null)" || exit 0
    while read -r id; do
        [ -n "$id" ] || continue
        docker rm --force "$id" >/dev/null 2>&1 || true
    done <<<"$containers"
    exit 0
fi

file="$1/container"
[ -f "$file" ] || exit 0
name="$(head -n 1 "$file")"
case "$name" in
    shellpp-openssh-*) ;;
    *)
        printf 'openssh/stop.sh: %s does not name a test container\n' "$file" >&2
        exit 1
        ;;
esac
docker rm --force "$name" >/dev/null 2>&1 || true
rm -f "$file"
