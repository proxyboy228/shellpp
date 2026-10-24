#!/bin/sh
# The backup service of docker-compose.yml: a pg_dump of the Shell++
# database, encrypted with age to the operator's public key, every
# BACKUP_INTERVAL_SECONDS. The database holds only ciphertext, but the dump
# also holds the account verifiers: keep the age private key off the server.
#
# Rotation (D199): backups not older than BACKUP_RETENTION_DAYS whole days
# stay; so do, before them, the first backup of each of the last
# BACKUP_KEEP_WEEKS calendar weeks and of each of the last BACKUP_KEEP_MONTHS
# calendar months, and always the newest backup. Nothing is deleted while
# the newest backup is less than BACKUP_MIN_PERCENT percent of the size of
# the largest one since the last accepted shrink (a database emptied or cut
# down by mistake, at once or in steps, would otherwise age the good backups
# out); after a deliberate large deletion, BACKUP_ACCEPT_SMALLER=1 accepts
# the newest backup as the new measure (server/README.md, "Backups").
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
# The time and size of the backup accepted as the measure of size
# (BACKUP_ACCEPT_SMALLER): "<time> <size>".
ACCEPTED="$DIR/.accepted-size"

fail() {
    echo "backup failed: $1" >&2
    # Restarted by Docker: wait a little before the next try.
    [ "${BACKUP_ONCE:-0}" = "1" ] || sleep 300
    exit 1
}

# The settings are checked before anything else: a bad value must not make
# the service dump the database again every five minutes.
for setting in "BACKUP_RETENTION_DAYS=$RETENTION" "BACKUP_KEEP_WEEKS=$WEEKS" \
    "BACKUP_KEEP_MONTHS=$MONTHS" "BACKUP_MIN_PERCENT=$MIN_PERCENT"; do
    case "${setting#*=}" in
        '' | *[!0-9]*) fail "${setting%%=*} must be a whole number" ;;
    esac
done

# What a run killed in the middle left behind (never a backup); a day old,
# so a run going on at the same time (`docker compose run ... backup`)
# keeps its file.
find "$DIR" -maxdepth 1 -type f -name 'shellpp-*.sql.age.part' -mmin +1440 -delete

# Every finished backup, oldest first: "<time> <size> <week> <month> <name>".
# The time is the UTC stamp of the name this script gives a backup
# (shellpp-<yyyymmddThhmmssZ>.sql.age), else the modification time, so a
# copy put back by hand keeps its place; the week (ISO) and month are UTC.
# A file that goes away meanwhile, or a name with a space, is left out.
backups() {
    for file in "$DIR"/shellpp-*.sql.age; do
        [ -f "$file" ] || continue
        name=$(basename "$file")
        case "$name" in *' '*) continue ;; esac
        meta=$(stat -c '%Y %s' "$file" 2>/dev/null) || continue
        set -- $meta
        when=$1
        stamp=${name#shellpp-}
        stamp=${stamp%.sql.age}
        case "$stamp" in
            [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z)
                text=$(echo "$stamp" | sed 's/^\(....\)\(..\)\(..\)T\(..\)\(..\)\(..\)Z$/\1-\2-\3 \4:\5:\6/')
                if parsed=$(date -u -d "$text" +%s 2>/dev/null); then
                    when=$parsed
                fi
                ;;
        esac
        echo "$when $2 $(date -u -d "@$when" +%G-%V) $(date -u -d "@$when" +%Y-%m) $name"
    done | sort -k1,1n -k5,5
}

# Called as `prune || fail`, so `set -e` does not hold here: every step
# that can fail is checked.
prune() {
    list=$(backups) || return 1
    [ -n "$list" ] || return 0
    if [ "${BACKUP_ACCEPT_SMALLER:-0}" = "1" ]; then
        echo "$list" | tail -n 1 | cut -d ' ' -f 1,2 >"$ACCEPTED" || return 1
        echo "accepted as the measure of size: $(echo "$list" | tail -n 1 | cut -d ' ' -f 5)"
    fi
    accepted=$(cat "$ACCEPTED" 2>/dev/null || true)
    now=$(date -u +%s)
    # The weeks and months whose first backup stays, counted back from the
    # end of the retention: they reach further back than the dailies.
    edge=$((now - (RETENTION + 1) * 86400))
    weeks=""
    i=0
    while [ "$i" -lt "$WEEKS" ]; do
        weeks="$weeks $(date -u -d "@$((edge - i * 604800))" +%G-%V)" || return 1
        i=$((i + 1))
    done
    month=$(date -u -d "@$edge" +%Y-%m) || return 1
    decisions=$(echo "$list" | awk -v now="$now" -v retention="$RETENTION" \
        -v percent="$MIN_PERCENT" -v weeks="$weeks" -v months="$MONTHS" \
        -v month="$month" -v accepted="$accepted" '
        { n++; t[n] = $1; size[n] = $2; week[n] = $3; mon[n] = $4; name[n] = $5 }
        END {
            # The measure: the largest backup before the newest one since
            # the accepted shrink (its size counts), so a shrink in steps is
            # caught as well as one at once.
            measure = -1
            accepted_t = -1
            if (split(accepted, a, " ") == 2 && a[1] ~ /^[0-9]+$/ && a[2] ~ /^[0-9]+$/) {
                accepted_t = a[1] + 0
                measure = a[2] + 0
            }
            for (i = 1; i < n; i++)
                if (t[i] > accepted_t && size[i] > measure) measure = size[i]
            if (t[n] > accepted_t && measure >= 0 && size[n] * 100 < measure * percent) {
                print "PAUSE " name[n] " " size[n] " " measure
                exit
            }
            split(weeks, w, " ")
            for (k in w) keep_week[w[k]] = 1
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
            echo "pruning paused: the newest backup $2 ($3 bytes) is less than $MIN_PERCENT% of the largest since the last accepted shrink ($4 bytes); no backup deleted. After a deliberate large deletion, run once with BACKUP_ACCEPT_SMALLER=1." >&2
            return 0
            ;;
    esac
    # A file that cannot be deleted is reported; the next run tries again.
    echo "$decisions" | while read -r action name; do
        [ "$action" = DELETE ] || continue
        if rm -f "$DIR/$name"; then
            echo "deleted: $name"
        else
            echo "could not delete: $name" >&2
        fi
    done
    return 0
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
    prune || fail "the old backups could not be listed or pruned"
    exit 0
fi

while :; do
    backup
    prune || fail "the old backups could not be listed or pruned"
    [ "${BACKUP_ONCE:-0}" = "1" ] && exit 0
    sleep "$INTERVAL"
done
