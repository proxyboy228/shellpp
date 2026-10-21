# Compatibility

What Shell++ was checked against, how, and what is known not to work.
Servers: section 1 (run 4, plan 9.1; D80). Terminal programs: section 2
(run 4, plan 9.2).

## 1. SSH servers

Each server runs in its own Docker container from `tests/openssh`
(`start.sh`, with `SHELLPP_OPENSSH_IMAGE` for the image and
`SHELLPP_OPENSSH_VARIANT` for a variant; README.md there). The suite is
`crates/ssh/tests/compat.rs`; CI runs it for every row (job "SSH
integration").

| Server | Image | Password | Key | Keyboard-interactive | Terminal | SFTP | Local forward | Notes |
|---|---|---|---|---|---|---|---|---|
| OpenSSH 10.0p2 (Debian 13) | `tests/openssh/Dockerfile` | yes | yes | yes | yes | yes | yes | the main test server of every suite |
| OpenSSH 7.4 (CentOS 7) | `compat/centos7.Dockerfile` | yes | yes | yes | yes | yes | yes | Ed25519 host key, curve25519, AES-GCM and ChaCha20 already there |
| Dropbear 2024.85 (Alpine 3.20) | `compat/dropbear.Dockerfile` | yes | yes | — | yes | yes (OpenSSH's sftp-server) | yes | Dropbear has no keyboard-interactive authentication |
| Only `ssh-rsa` (SHA-1) and `diffie-hellman-group14-sha1` | variant `legacy-sha1` | with the host setting | with the host setting | with the host setting | yes | yes | yes | without the setting the connection fails with the mismatch error below |
| Key plus a keyboard-interactive second factor | variant `kbd-2fa` | — | yes, then the question | yes | yes | yes | yes | `AuthenticationMethods publickey,keyboard-interactive` |
| No SFTP subsystem | variant `no-sftp` | yes | yes | yes | yes | no: "The server refused to open a session." | yes | the terminal and forwarding work as usual |
| A banner before login and a 120-line message of the day | variant `motd-banner` | yes | yes | yes | yes | yes | yes | the banner reaches the connection's interaction, the message of the day the terminal |

Each check is: password login (`pwuser`), key login (`keyuser`, Ed25519),
keyboard-interactive (`kiuser`, a masked question), a shell with a computed
marker, `stty size` and a UTF-8 line both ways, the home folder over SFTP,
and a Local rule to the server's own port 22 that reads the SSH greeting.

### Legacy algorithms

Off by default; the host setting "Allow legacy algorithms" (Show more of
the host panel) turns them on for that host only, with a warning shown
while it is on (D80). Then Shell++ also offers, after all modern ones of
each list:

- key exchange: `diffie-hellman-group14-sha1`,
  `diffie-hellman-group-exchange-sha1`, `diffie-hellman-group1-sha1`;
- host key signatures: `ssh-rsa` (SHA-1);
- MACs: `hmac-sha1-etm@openssh.com`, `hmac-sha1`;
- ciphers: `aes256-cbc`, `aes192-cbc`, `aes128-cbc`.

Never offered: 3DES, RC4 (`arcfour*`), Blowfish, CAST, `ssh-dss`, MD5 and
96-bit MACs, `umac-*`.

### When nothing matches

The failure screen names the negotiation ("no key exchange algorithm in
common", or host key, cipher, MAC, compression), then "The server offers:
…" and "Shell++ supports: …". When the server offers a legacy algorithm, it
adds how to turn the host setting on. The server's list shows only names
Shell++ knows (others are counted, "and 2 others"): no text the server sent
is shown or logged.

### Not checked

- Servers that need a real device (network equipment, embedded systems):
  the legacy-sha1 variant stands in for their algorithm lists.
- Tectia, Bitvise and the Windows OpenSSH server: no Linux container of
  them.
- GSSAPI/Kerberos authentication: not supported.

## 2. Terminal programs

See section 2 below once it is filled in (plan 9.2).
