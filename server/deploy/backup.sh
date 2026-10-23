#!/bin/sh
# The backup service of docker-compose.yml: a pg_dump of the Shell++
# database, encrypted with age to the operator's public key, every
# BACKUP_INTERVAL_SECONDS; files older than BACKUP_RETENTION_DAYS are
# deleted. The database holds only ciphertext, but the dump also holds the
# account verifiers: keep the age private key off the server.
#
# With BACKUP_ONCE=1 it makes one backup (or with BACKUP_PRUNE_ONLY=1 only
# prunes) and exits: the restore test and `docker compose run backup`.
set -eu

: "${AGE_RECIPIENT:?AGE_RECIPIENT is required}"
DIR=${BACKUP_DIR_IN_CONTAINER:-/backups}
RETENTION=${BACKUP_RETENTION_DAYS:-30}
INTERVAL=${BACKUP_INTERVAL_SECONDS:-86400}

if ! command -v pg_dump >/dev/null 2>&1 || ! command -v age >/dev/null 2>&1; then
    apk add --no-cache postgresql17-client age >/dev/null
fi

prune() {
    # Older than the retention (in days, by modification time).
    find "$DIR" -maxdepth 1 -type f -name 'shellpp-*.sql.age' -mtime +"$RETENTION" -print -delete
}

backup() {
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    target="$DIR/shellpp-$stamp.sql.age"
    pg_dump --no-owner --format=plain | age -r "$AGE_RECIPIENT" -o "$target.part"
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
