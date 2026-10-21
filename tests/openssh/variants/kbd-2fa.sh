#!/bin/sh
# Variant kbd-2fa (tests/openssh/README.md, "Variants"): the key user signs
# in with its key and then answers a keyboard-interactive question (its
# password through PAM), as a second factor works.
set -eu
config=/etc/ssh/sshd_config
sed -i '/^Match User keyuser/,/^$/ s/AuthenticationMethods publickey/AuthenticationMethods publickey,keyboard-interactive/' "$config"
