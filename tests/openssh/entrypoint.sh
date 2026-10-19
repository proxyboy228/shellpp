#!/bin/sh
# Entrypoint of the OpenSSH test server container (tests/openssh, README.md).
#
# Runs as root at container start:
#   1. creates the test users from the credentials that start.sh copied to
#      /run/shellpp-input (generated for this run), then removes that copy;
#   2. generates host keys (none are part of the image) and writes their
#      types and SHA256 fingerprints to /run/shellpp/host_keys;
#   3. runs sshd in the foreground, logging to stderr, for at most
#      SHELLPP_OPENSSH_LIFETIME seconds (default 3600), so a container that
#      a crashed test run left behind stops by itself.
#
# Secrets are never printed.

set -eu

input=/run/shellpp-input
state=/run/shellpp
lifetime="${SHELLPP_OPENSSH_LIFETIME:-3600}"

fail() {
    echo "shellpp-openssh: $*" >&2
    exit 1
}

case "$lifetime" in
    '' | *[!0-9]*) fail 'SHELLPP_OPENSSH_LIFETIME must be a number of seconds' ;;
esac

# add_user NAME: a regular user with a home directory and bash.
add_user() {
    if ! id -u "$1" >/dev/null 2>&1; then
        useradd --create-home --user-group --shell /bin/bash "$1"
    fi
}

# set_password NAME FILE: sets the password read from FILE (one line).
set_password() {
    [ -s "$2" ] || fail "missing password for $1"
    password=$(head -n 1 "$2")
    case "$password" in
        '' | *:*) fail "unusable password for $1" ;;
    esac
    printf '%s:%s\n' "$1" "$password" | chpasswd
    password=''
}

if [ -d "$input" ]; then
    # Password authentication.
    add_user pwuser
    set_password pwuser "$input/pwuser.password"

    # Public key authentication: no password, the run's client key only.
    add_user keyuser
    [ -s "$input/keyuser.authorized_keys" ] || fail 'missing authorized key for keyuser'
    install -d -m 0700 -o keyuser -g keyuser /home/keyuser/.ssh
    install -m 0600 -o keyuser -g keyuser "$input/keyuser.authorized_keys" /home/keyuser/.ssh/authorized_keys

    # Keyboard-interactive authentication (PAM asks for the password).
    add_user kiuser
    set_password kiuser "$input/kiuser.password"

    rm -rf "$input"
elif ! id -u pwuser >/dev/null 2>&1; then
    fail "no credentials in $input: start the container with tests/openssh/start.sh"
fi

# Host keys of every default type (RSA, ECDSA, Ed25519), generated once per
# container.
ssh-keygen -A >/dev/null
mkdir -p "$state"
: >"$state/host_keys.tmp"
for pub in /etc/ssh/ssh_host_*_key.pub; do
    key_type=$(cut -d ' ' -f 1 "$pub")
    fingerprint=$(ssh-keygen -l -E sha256 -f "$pub" | cut -d ' ' -f 2)
    printf '%s %s\n' "$key_type" "$fingerprint" >>"$state/host_keys.tmp"
done
mv "$state/host_keys.tmp" "$state/host_keys"

mkdir -p /run/sshd
chmod 0755 /run/sshd
/usr/sbin/sshd -t

exec timeout "$lifetime" /usr/sbin/sshd -D -e
