#!/bin/sh
# Generates the .ppk test keys into /out (a mounted temporary folder):
# for each key type, a fresh key from puttygen, saved as .ppk version 3 and
# 2, each plain and encrypted with a passphrase generated here, plus the
# public key in the OpenSSH format (the expected fingerprint). Version 3
# uses puttygen's default Argon2id settings. -P makes puttygen write the
# key again even when only the format version changes.
#
# /out/passphrase holds the passphrase; every file is removed with the
# folder by the test. Nothing here is a real credential.
set -eu
out=/out
umask 077
head -c 18 /dev/urandom | base64 | tr -d '/+=' > "$out/passphrase"
empty="$out/.empty"
: > "$empty"
for spec in ed25519 ecdsa:256 ecdsa:384 ecdsa:521 rsa:2048; do
    type=${spec%%:*}
    bits=${spec#*:}
    name=$type
    set -- -t "$type"
    if [ "$bits" != "$spec" ]; then
        name="$type-$bits"
        set -- "$@" -b "$bits"
    fi
    base="$out/$name"
    puttygen "$@" -C "shellpp-test-$name" --new-passphrase "$empty" -o "$base.v3.ppk" -q
    puttygen "$base.v3.ppk" -O public-openssh -o "$base.pub" -q
    puttygen "$base.v3.ppk" -P -O private --ppk-param version=2 --new-passphrase "$empty" -o "$base.v2.ppk" -q
    puttygen "$base.v3.ppk" -P -O private --new-passphrase "$out/passphrase" -o "$base.v3.enc.ppk" -q
    puttygen "$base.v3.ppk" -P -O private --ppk-param version=2 --new-passphrase "$out/passphrase" -o "$base.v2.enc.ppk" -q
done
# Comments a careless normalization breaks: none at all (the header line is
# "Comment: "), and trailing spaces, which the MAC covers.
puttygen -t ed25519 -C "" --new-passphrase "$empty" -o "$out/no-comment.v3.ppk" -q
puttygen -t ed25519 -C "spaced  " --new-passphrase "$out/passphrase" -o "$out/spaced-comment.v3.enc.ppk" -q
rm -f "$empty"
