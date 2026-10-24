#!/bin/sh
# The backup service of docker-compose.yml: a pg_dump of the Shell++
# database, encrypted with age to the operator's public key, every
# BACKUP_INTERVAL_SECONDS. The database holds only ciphertext, but the dump
# also holds the account verifiers: keep the age private key off the server.
#
# Rotation (D199): backups younger than BACKUP_RETENTION_DAYS stay; so do
# the first backup of each of the last BACKUP_KEEP_WEEKS calendar weeks and
# of each of the last BACKUP_KEEP_MONTHS calendar months, and the newest
# backup. Nothing is deleted while the newest backup is less than
# BACKUP_MIN_PERCENT percent of the size of the last good one (a database
# emptied or cut down by mistake would otherwise age the good backups out);
# after a deliberate large deletion, BACKUP_ACCEPT_SMALLER=1 accepts the
# newest backup as the new measure (server/README.md, "Backups").
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
WEEKS=${BACKUP_KEEP_WEEKS:-4}
MONTHS=${BACKUP_KEEP_MONTHS:-3}
MIN_PERCENT=${BACKUP_MIN_PERCENT:-50}
# The newest backup accepted as the measure of size (BACKUP_ACCEPT_SMALLER).
ACCEPTED="$DIR/.accepted-size"

# What a run killed in the middle left behind (never a backup); a day old,
# so a run going on at the same time (`docker compose run ... backup`)
# keeps its file.
find "$DIR" -maxdepth 1 -type f -name 'shellpp-*.sql.age.part' -mmin +1440 -delete

fail() {
    echo "backup failed: $1" >&2
    # Restarted by Docker: wait a little before the next try.
    [ "${BACKUP_ONCE:-0}" = "1" ] || sleep 300
    exit 1
}

# Every finished backup, oldest first: "<mtime> <size> <week> <month> <name>"
# (the week and month in UTC, by modification time).
backups() {
    for file in "$DIR"/shellpp-*.sql.age; do
        [ -f "$file" ] || continue
        set -- $(stat -c '%Y %s' "$file")
        echo "$1 $2 $(date -u -d "@$1" +%G-%V) $(date -u -d "@$1" +%Y-%m) $(basename "$file")"
    done | sort -n
}

prune() {
    for value in "$RETENTION" "$WEEKS" "$MONTHS" "$MIN_PERCENT"; do
        case "$value" in
            '' | *[!0-9]*) return 1 ;;
        esac
    done
    list=$(backups)
    [ -n "$list" ] || return 0
    if [ "${BACKUP_ACCEPT_SMALLER:-0}" = "1" ]; then
        echo "$list" | tail -n 1 | cut -d ' ' -f 5 >"$ACCEPTED"
        echo "accepted as the measure of size: $(cat "$ACCEPTED")"
    fi
    now=$(date -u +%s)
    # The calendar weeks and months whose first backup stays.
    weeks=""
    i=0
    while [ "$i" -lt "$WEEKS" ]; do
        weeks="$weeks $(date -u -d "@$((now - i * 604800))" +%G-%V)"
        i=$((i + 1))
    done
    decisions=$(echo "$list" | awk         -v now="$now" -v retention="$RETENTION" -v percent="$MIN_PERCENT"         -v weeks="$weeks" -v months="$MONTHS" -v month="$(date -u +%Y-%m)"         -v accepted="$(cat "$ACCEPTED" 2>/dev/null)" '
        { n++; t[n] = $1; size[n] = $2; week[n] = $3; mon[n] = $4; name[n] = $5 }
        END {
            # The measure: the last good backup. One less than the
            # percentage of it is small, and does not become the measure
            # unless accepted.
            measure = -1
            for (i = 1; i <= n; i++) {
                small = measure >= 0 && size[i] * 100 < measure * percent && name[i] != accepted
                if (!small) measure = size[i]
            }
            if (small) { print "PAUSE " name[n] " " size[n] " " measure; exit }
            split(weeks, w, " ")
            for (k in w) keep_week[w[k]] = 1
            # The last months, counted back from this one.
            y = substr(month, 1, 4) + 0; m = substr(month, 6, 2) + 0
            for (k = 0; k < months; k++) {
                keep_month[sprintf("%04d-%02d", y, m)] = 1
                if (--m == 0) { m = 12; y-- }
            }
            for (i = 1; i <= n; i++) {
                # find -mtime +N: older than N whole days.
                keep = int((now - t[i]) / 86400) <= retention || i == n
                if ((week[i] in keep_week) && !(week[i] in seen_week)) keep = 1
                if ((mon[i] in keep_month) && !(mon[i] in seen_month)) keep = 1
                seen_week[week[i]] = 1
                seen_month[mon[i]] = 1
                if (!keep) print "DELETE " name[i]
            }
        }') || return 1
    case "$decisions" in
        PAUSE*)
            set -- $decisions
            echo "pruning paused: the newest backup $2 ($3 bytes) is less than $MIN_PERCENT% of the last good one ($4 bytes); no backup deleted. After a deliberate large deletion, run once with BACKUP_ACCEPT_SMALLER=1." >&2
            return 0
            ;;
    esac
    echo "$decisions" | while read -r action name; do
        [ "$action" = DELETE ] || continue
        rm -f "$DIR/$name" && echo "deleted: $name"
    done
}

backup() {
    for tool in pg_dump age; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            fail "$tool is missing; build the backup image (docker compose build backup)"
        fi
    done
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    target="$DIR/shellpp-$stamp.sql.age"
    if ! pg_dump --no-owner --format=plain | age -r "$AGE_RECIPIENT" -o "$target.part"; then
        rm -f "$target.part"
        fail "pg_dump or age exited with an error; no file written"
    fi
    mv "$target.part" "$target" || fail "the finished backup could not be renamed"
    echo "backup written: $(basename "$target")"
}

if [ "${BACKUP_PRUNE_ONLY:-0}" = "1" ]; then
    prune || fail "the old backups could not be pruned (BACKUP_RETENTION_DAYS, BACKUP_KEEP_WEEKS, BACKUP_KEEP_MONTHS and BACKUP_MIN_PERCENT are whole numbers)"
    exit 0
fi

while :; do
    backup
    prune || fail "the old backups could not be pruned (BACKUP_RETENTION_DAYS, BACKUP_KEEP_WEEKS, BACKUP_KEEP_MONTHS and BACKUP_MIN_PERCENT are whole numbers)"
    [ "${BACKUP_ONCE:-0}" = "1" ] && exit 0
    sleep "$INTERVAL"
done
