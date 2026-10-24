#!/bin/sh
# The backup service of docker-compose.yml: a pg_dump of the Shell++
# database, encrypted with age to the operator's public key, every
# BACKUP_INTERVAL_SECONDS; files older than BACKUP_RETENTION_DAYS are
# deleted. The database holds only ciphertext, but the dump also holds the
# account verifiers: keep the age private key off the server.
#
# pg_dump and age come with the image (backup.Dockerfile, built once by
# `docker compose build`), so a backup needs no network.
#
# A failed dump or encryption ends the script with an error and leaves no
# file that looks like a backup (only finished ones are renamed to
# .sql.age); nothing is pruned after a failure, so old backups stay.
#
# With BACKUP_ONCE=1 it makes one backup (or with BACKUP_PRUNE_ONLY=1 only
# prunes) and exits: the restore test and `docker compose run backup`.
set -eu
# busybox ash has pipefail: a failing pg_dump fails the pipeline, not only
# a failing age.
set -o pipefail

: "${AGE_RECIPIENT:?AGE_RECIPIENT is required}"
DIR=${BACKUP_DIR_IN_CONTAINER:-/backups}
RETENTION=${BACKUP_RETENTION_DAYS:-30}
INTERVAL=${BACKUP_INTERVAL_SECONDS:-86400}

prune() {
    # Older than the retention (in days, by modification time).
    find "$DIR" -maxdepth 1 -type f -name 'shellpp-*.sql.age' -mtime +"$RETENTION" -print -delete
}

backup() {
    for tool in pg_dump age; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "backup failed: $tool is missing; build the backup image (docker compose build backup)" >&2
            exit 1
        fi
    done
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    target="$DIR/shellpp-$stamp.sql.age"
    if ! pg_dump --no-owner --format=plain | age -r "$AGE_RECIPIENT" -o "$target.part"; then
        rm -f "$target.part"
        echo "backup failed: pg_dump or age exited with an error; no file written" >&2
        exit 1
    fi
    mv "$target.part" "$target"
    echo "backup written: $(basename "$target")"
}

if [ "${BACKUP_PRUNE_ONLY:-0}" = "1" ]; then
    prune
    exit 0
fi

while :; do
    backup
    prune
    [ "${BACKUP_ONCE:-0}" = "1" ] && exit 0
    sleep "$INTERVAL"
done
