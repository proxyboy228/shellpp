# OpenSSH test server

An OpenSSH server in Docker for:

- the `shellpp-ssh` integration suite (`SHELLPP_TEST_SSHD=openssh`;
  `docs/architecture.md`, sections 9.1 and 9.3);
- the session tests of the desktop app (`apps/desktop/tests/sessions.rs`),
  which pick this server automatically when Docker is running and the
  built-in russh server otherwise;
- the terminal screenshots and benchmarks of the desktop app
  (`scripts/ui-screens.ps1`, `scripts/measure-terminal.ps1`; their default
  `-Server auto` picks this server when Docker runs): a real bash with
  `ls --color`, `htop`, `mc` and `seq`.

The same suites run against the built-in russh server (`tests/sshd`,
`shellpp-test-sshd`) on every platform; this server checks the client against
a real OpenSSH on Linux CI (`docs/decisions.md`, decision 6).

Test use only; nothing here is shipped.

## Files

| File | Purpose |
|---|---|
| `Dockerfile` | Debian slim, pinned by digest, with `openssh-server`, `htop`, `mc` and `procps`; no users, passwords or keys |
| `sshd_config` | the server configuration (below) |
| `entrypoint.sh` | runs at container start: creates the users, generates the host keys, starts `sshd` |
| `container-control.sh` | in the image as `shellpp-openssh-control`: new host keys, dropped connections (run by `control.sh`) |
| `start.sh <workdir>` | builds the image, generates the run's credentials, starts a container, writes `<workdir>/server.json` |
| `control.sh <workdir> <command>` | server-side faults on that container: `rotate-host-key`, `drop-connections`, `freeze`, `thaw` (below) |
| `stop.sh <workdir>` | removes that container; `stop.sh --all` removes every container started by `start.sh` |

## Requirements

Docker (a running daemon for Linux containers: `docker info` must succeed),
bash and `ssh-keygen` on the host. Linux and macOS (Docker Desktop) work; on
Windows the scripts run in Git for Windows' bash (the desktop app's tests and
scripts find it next to `git`, or take `SHELLPP_BASH`; WSL's `bash.exe`
cannot read Windows paths). When Docker is not available, `start.sh` exits
with status 3; the desktop app's session tests and scripts then use the
russh test server and say so, and the openssh provider of the `shellpp-ssh`
suite skips its tests with a message.

## Usage

```sh
workdir="$(mktemp -d)"
bash tests/openssh/start.sh "$workdir"      # prints the path of server.json
# ... connect to the server described in "$workdir/server.json" ...
bash tests/openssh/stop.sh "$workdir"
rm -rf "$workdir"
```

Run the suite against it (the provider glue in `crates/ssh/tests` calls the
scripts itself):

```sh
SHELLPP_TEST_SSHD=openssh cargo test -p shellpp-ssh
```

`start.sh`:

1. builds `shellpp-test-openssh:local` from this directory (Docker's layer
   cache makes repeated builds quick), unless `SHELLPP_OPENSSH_IMAGE` names a
   prebuilt image (CI builds `shellpp-test-openssh:ci` once);
2. generates the credentials of the run in `<workdir>`: two random passwords
   and an Ed25519 client key without passphrase (`client_ed25519`,
   `client_ed25519.pub`). Tests that need an encrypted key or other key types
   generate their own and use the key user only through its authorized key;
3. creates a container that publishes port 22 on `127.0.0.1:<random port>`
   only, with a minimal init as PID 1 (`docker create --init`: it reaps the
   processes of ended sessions, so `ps` and `htop` show no zombies), copies
   the credentials into it as a tar stream and starts it;
4. waits until the server sends its SSH identification line, reads the host
   key fingerprints and writes `<workdir>/server.json` (mode 0600 through the
   script's umask), then prints its path.

Progress and errors go to stderr; passwords and keys are never printed. On any
failure the container is removed again.

| Exit status | Meaning |
|---|---|
| 0 | started; `server.json` written |
| 1 | failed (the reason is on stderr, with the container output when `sshd` did not start) |
| 2 | usage |
| 3 | Docker is not available |

| Environment variable | Meaning |
|---|---|
| `SHELLPP_OPENSSH_IMAGE` | run this prebuilt image instead of building `shellpp-test-openssh:local` |
| `SHELLPP_OPENSSH_LIFETIME` | seconds after which the container stops by itself (default 3600), so a container left behind by a crashed run does not keep running |
| `SHELLPP_OPENSSH_TIMEOUT` | seconds to wait for the server (default 60) |

## server.json

```json
{
  "host": "127.0.0.1",
  "port": 49153,
  "host_keys": {
    "ecdsa-sha2-nistp256": "SHA256:<fingerprint>",
    "ssh-ed25519": "SHA256:<fingerprint>",
    "ssh-rsa": "SHA256:<fingerprint>"
  },
  "users": {
    "password": {"name": "pwuser", "password": "<generated>"},
    "publickey": {"name": "keyuser", "private_key_path": "<workdir>/client_ed25519"},
    "keyboard_interactive": {"name": "kiuser", "password": "<generated>"}
  },
  "container": "shellpp-openssh-<random>"
}
```

- `host_keys` maps the key type (the first field of the public key line) to
  the SHA256 fingerprint in OpenSSH form (unpadded base64). The server offers
  RSA host keys only with `rsa-sha2-256` and `rsa-sha2-512` signatures.
- `private_key_path` is absolute (on Git for Windows in `C:/...` form).
- `container` is the container name; `start.sh` also writes it to
  `<workdir>/container` for `stop.sh`.

## Server configuration

- OpenSSH from the Debian release of the base image (10.0 for Debian 13).
- Users, one authentication method each (`AuthenticationMethods` per user, so
  the server offers only that method):
  - `pwuser`: `password` (through PAM);
  - `keyuser`: `publickey`, the run's client key in `authorized_keys`;
  - `kiuser`: `keyboard-interactive` through PAM: one question, the password,
    with echo off.
- `AllowAgentForwarding yes`: in a session with forwarding, `ssh-add -l`
  lists the forwarded keys.
- `AllowTcpForwarding yes` and `Subsystem sftp internal-sftp` for the later
  stages (forwarding, SFTP).
- `AcceptEnv LANG LC_* SHELLPP_*`: environment requests for other names are
  refused, as by a typical server.
- `PerSourcePenalties no`: all connections come from the Docker gateway
  address, and the tests fail authentication on purpose.
- Host keys (RSA, ECDSA, Ed25519) are generated once per container; a new
  container has new keys.
- Logs go to the container output (`docker logs <container>`); they contain
  user names and addresses of the test run only.
- Sessions: bash with `LANG=C.UTF-8` (from `/etc/default/locale` through
  PAM), Debian's `.bashrc` (`ls` is `ls --color=auto`) with a colored prompt
  for every `TERM`, and a few fictional files in every home directory
  (directories, an executable script, a symlink, a broken symlink, an archive
  and an image name) so that `ls --color` shows its colors. `htop` and `mc`
  are installed; `seq` comes with coreutils.

The suite cannot use the deterministic shell of `shellpp-test-sshd` here; the
users have bash. Portable equivalents of its commands: `stty size` (rows and
columns), `echo "$TERM"`, `printenv NAME`, `ssh-add -l` (forwarded agent),
`exit <n>`, `seq 1 <n>` (for `lines <n>`).

## Server-side faults

`control.sh <workdir> <command>` acts on the container of `start.sh
<workdir>`:

| Command | Effect | Output |
|---|---|---|
| `rotate-host-key` | new host keys of every type; the listening `sshd` restarts with them (SIGHUP), sessions already open keep running; returns once the server offers the new Ed25519 key | `server.json` is updated (`host_keys`), its path is printed |
| `drop-connections` | every connection process (`sshd-session`, `sshd-auth`) is killed: clients see the connection closed without an exit status; the listener keeps accepting | the number of processes killed |
| `freeze` | `docker pause`: open connections stop answering without being closed (keepalive timeouts), new connections hang | nothing |
| `thaw` | `docker unpause` | nothing |

Exit status: 0 done, 1 failed, 2 usage, 3 Docker is not available.
`stop.sh` also removes a frozen container. Refusing connections for a while
(as `shellpp-test-sshd`'s `accept off` does) has no equivalent here; tests
that need it run against `shellpp-test-sshd` only.

## Secrets

Nothing secret is part of the repository or the image (`CONTRIBUTING.md`,
"Secrets and test data"): the Debian package's host keys are deleted at build
time, and users, passwords, the client key and the host keys are generated for
each run. The copy of the credentials inside the container is deleted by the
entrypoint once the users exist. The server listens on the loopback address
only.

## Updating the base image

The `FROM` line pins a dated Debian tag together with the digest of its
multi-platform index. To move to a newer point release, look up the digest of
the new tag and change both:

```sh
docker buildx imagetools inspect debian:trixie-YYYYMMDD-slim   # "Digest: sha256:..."
```

Then run the suite locally (`SHELLPP_TEST_SSHD=openssh cargo test -p
shellpp-ssh`) or let the `linux` CI job do it.
