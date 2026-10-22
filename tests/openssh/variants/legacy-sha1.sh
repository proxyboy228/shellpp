#!/bin/sh
# Variant legacy-sha1 of the OpenSSH test server (tests/openssh/README.md,
# "Variants"): only an ssh-rsa (SHA-1) host key, the SHA-1 key exchange
# diffie-hellman-group14-sha1 and AES-CBC with the SHA-1 MAC, as old servers
# and appliances have them; ssh-rsa user key signatures are accepted too.
# Run by the entrypoint as root before sshd starts.
set -eu
config=/etc/ssh/sshd_config
sed -i '/^HostKey .*ed25519/d; /^HostKey .*ecdsa/d' "$config"
# Global options go before the first Match block (after it they would
# belong to that block).
block=/tmp/shellpp-variant
cat >"$block" <<'CONF'
# Variant legacy-sha1.
HostKeyAlgorithms ssh-rsa
KexAlgorithms diffie-hellman-group14-sha1
Ciphers aes128-cbc
MACs hmac-sha1
PubkeyAcceptedKeyTypes +ssh-rsa
CONF
awk 'NR == FNR { lines = lines $0 "\n"; next }
     !done && /^Match / { printf "%s\n", lines; done = 1 }
     { print }
     END { if (!done) printf "%s", lines }' "$block" "$config" >"$config.new"
mv "$config.new" "$config"
chmod 0644 "$config"
rm -f "$block"
