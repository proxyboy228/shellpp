#!/bin/sh
# Entrypoint of the Dropbear compatibility image (tests/openssh/compat):
# the users of tests/openssh/entrypoint.sh from the run's credentials in
# /run/shellpp-input (kiuser gets a password too: Dropbear asks it as a
# password), fresh host keys with their SHA256 fingerprints in
# /run/shellpp/host_keys, then Dropbear in the foreground for at most
# SHELLPP_OPENSSH_LIFETIME seconds. Secrets are never printed.
set -eu
input=/run/shellpp-input
state=/run/shellpp
lifetime="${SHELLPP_OPENSSH_LIFETIME:-3600}"

fail() {
    echo "shellpp-dropbear: $*" >&2
    exit 1
}

add_user() {
    id -u "$1" >/dev/null 2>&1 || adduser -D -s /bin/bash "$1"
}

set_password() {
    [ -s "$2" ] || fail "missing password for $1"
    password=$(head -n 1 "$2")
    printf '%s:%s\n' "$1" "$password" | chpasswd
    password=''
}

[ -d "$input" ] || fail "no credentials in $input: start the container with tests/openssh/start.sh"
add_user pwuser
set_password pwuser "$input/pwuser.password"
add_user keyuser
# A locked account cannot sign in with a key either: an unusable password.
printf 'keyuser:%s\n' "$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')" | chpasswd
install -d -m 0700 -o keyuser -g keyuser /home/keyuser/.ssh
install -m 0600 -o keyuser -g keyuser "$input/keyuser.authorized_keys" /home/keyuser/.ssh/authorized_keys
add_user kiuser
set_password kiuser "$input/kiuser.password"
rm -rf "$input"

mkdir -p /etc/dropbear "$state"
: >"$state/host_keys.tmp"
for type in ed25519 ecdsa rsa; do
    key="/etc/dropbear/dropbear_${type}_host_key"
    rm -f "$key"
    dropbearkey -t "$type" -f "$key" >/dev/null 2>&1
    dropbearkey -y -f "$key" | grep '^ssh-\|^ecdsa-' >"$key.pub"
    key_type=$(cut -d ' ' -f 1 "$key.pub")
    fingerprint=$(ssh-keygen -l -E sha256 -f "$key.pub" | cut -d ' ' -f 2)
    printf '%s %s\n' "$key_type" "$fingerprint" >>"$state/host_keys.tmp"
done
mv "$state/host_keys.tmp" "$state/host_keys"

# -F foreground, -E log to stderr, -p port; host keys given one by one.
exec timeout "$lifetime" dropbear -F -E -p 22 \
    -r /etc/dropbear/dropbear_ed25519_host_key \
    -r /etc/dropbear/dropbear_ecdsa_host_key \
    -r /etc/dropbear/dropbear_rsa_host_key
