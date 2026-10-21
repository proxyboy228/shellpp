#!/bin/sh
# Variant motd-banner (tests/openssh/README.md, "Variants"): a banner before
# authentication and a long message of the day after it.
set -eu
config=/etc/ssh/sshd_config
{
    echo 'Authorized access only. This system is monitored.'
    echo 'Shell++ compatibility test banner.'
} >/etc/ssh/shellpp-banner
: >/etc/motd
i=1
while [ "$i" -le 120 ]; do
    printf 'Message of the day, line %s: maintenance window on Sunday 02:00-04:00 UTC.\n' "$i" >>/etc/motd
    i=$((i + 1))
done
sed -i 's/^PrintMotd no/PrintMotd yes/' "$config"
# Before the first Match block, as a global option.
sed -i '0,/^Match /s//# Variant motd-banner.\nBanner \/etc\/ssh\/shellpp-banner\n\n&/' "$config"
