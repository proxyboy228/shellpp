# Shell++ sync server

The server that lets your own devices sync their Shell++ vaults. It stores
only ciphertext: record envelopes, key wraps and verifiers of the keys your
devices derive. Your master password, your recovery key and the plaintext
of your hosts, keys and snippets never reach it.

License: AGPL-3.0-only (the desktop app is a separate workspace).

## What you need

- A machine that stays on: a homelab box, a NAS with Docker, a small VPS.
- Docker with Compose v2.
- One way for your devices to reach it over HTTPS (see "Access" below).
  The app speaks plain `http://` only to `localhost`, `127.0.0.1` and `[::1]`.

## Install

From a checkout of the repository:

```bash
docker build -f server/Dockerfile -t shellpp-server .
cd server/deploy
cp .env.example .env
```

The server secret goes into a file that Compose hands to the server as a
Docker secret (`/run/secrets/server_secret`), not into `.env`: the
environment of a container shows in `docker inspect`. It derives the
server's own keys (the fake pre-login salts, the TOTP secrets at rest).
Keep a copy with your backup key: a restored database needs the same
secret, or every account's TOTP has to be reset.

```bash
mkdir -p secrets
openssl rand -base64 48 > secrets/server_secret
chmod 0400 secrets/server_secret
sudo chown 10001 secrets/server_secret   # the server's user in the image
```

The image runs as user 10001, which must be able to read the file. If the
server stops with "SHELLPP_SERVER_SECRET_FILE cannot be read: permission
denied" (rootless Docker, user namespaces, Podman, SELinux), give that user
read access the way your setup maps it, or relabel the folder (`:z`).
The server also still reads `SHELLPP_SERVER_SECRET` from the environment,
with a warning in its log; both at once is refused. It refuses to start
when the secret opens none of the TOTP secrets stored in the database, so
a wrong secret never runs.

Fill in `.env`:

- `POSTGRES_PASSWORD`: `openssl rand -base64 32`.
- `SHELLPP_PUBLIC_URL`: the https origin your devices will type, for example
  `https://sync.example.com`.
- `BACKUP_AGE_RECIPIENT`: the public key of an `age` key pair made on
  another machine with `age-keygen -o shellpp-backup.key`. Keep the private
  key away from the server.
- `SHELLPP_REGISTRATION`: `invite` (default) or `open`.
- `SHELLPP_TRUSTED_PROXIES`: keep `172.16.88.1/32` (see "Client addresses"
  below).

Start it:

```bash
docker compose up -d
docker compose logs -f server
```

The server listens on `127.0.0.1:8080`. Health: `curl http://127.0.0.1:8080/healthz`.

### Accounts

Registration needs an invitation code by default:

```bash
docker compose run --rm server admin user invite --uses 1 --days 7
```

The code is shown once. In the app: Settings → Account & Sync → Connect a
sync server → your address → Create account, with the code. The device
keeps its data and its master password; the password becomes the
account's.

Other commands (`docker compose run --rm server admin --help`):

| Command | What it does |
|---|---|
| `admin user list` | accounts (id, email, created) |
| `admin user delete <email or id>` | deletes an account and its data |
| `admin user quota <who> --records N --bytes N` / `--clear` | storage limits (off unless set) |
| `admin user reset-totp <who>` | for someone who lost their authenticator |
| `admin user disable <who>` / `enable <who>` | signs every device out / allows again |
| `admin device revoke <device id>` | signs one device out |
| `admin vault delete <vault id>` | deletes a vault and its records |
| `admin restored` | after every restore from a backup (below) |
| `admin purge` | runs the daily purge now |

### Connect your devices

1. On the first device: Settings → Account & Sync → Connect a sync server.
   Enter the address, then Create account (email, the device's master
   password, the invitation code). Its data goes up in the first sync.
2. On a new device, on the first screen: Restore from sync… → the same
   address → Sign in. The device gets the account's vault; its local copy
   then opens with the account password.
3. On a device that already has its own data: Settings → Account & Sync →
   Connect → Sign in. Its data is merged into the account (a backup of the
   local database is written first); items that look the same are kept
   side by side and offered under Review → Possible duplicates.

Optional: Settings → Account & Sync → set up two-step sign-in (TOTP) from
another device; the server asks for the code at every new sign-in.

## Access

Pick one. In each case the server itself stays on `127.0.0.1:8080`.

### 1. Your own domain, Caddy and Let's Encrypt

DNS of `sync.example.com` points to the machine; ports 80 and 443 are open
to the internet. Run Caddy on the host with `deploy/Caddyfile.example`
(change the name):

```bash
caddy run --config server/deploy/Caddyfile.example --adapter caddyfile
```

Caddy gets and renews the certificate; the app trusts it through your
system's roots, nothing to confirm. `SHELLPP_TRUSTED_PROXIES`: see
"Client addresses" below.

### 2. Cloudflare Tunnel

No open ports. Create a tunnel and route the name to it:

```bash
cloudflared tunnel create shellpp
cloudflared tunnel route dns shellpp sync.example.com
```

Put the tunnel's id and credentials file into
`deploy/cloudflared.example.yml` and run
`cloudflared tunnel --config deploy/cloudflared.example.yml run`.
Cloudflare terminates TLS (it sees the encrypted envelopes and your
devices' metadata, never plaintext). `SHELLPP_TRUSTED_PROXIES`: see
"Client addresses" below.

### 3. Only over your VPN, with an internal CA

The server is reachable only inside your WireGuard or Tailscale network,
for example as `sync.home.arpa`. Caddy issues the certificate from its own
CA (`deploy/Caddyfile.internal.example`, `tls internal`).

The first time a device connects, the app shows the certificate (subject,
issuer, dates, SHA-256 fingerprint) and offers two ways to trust it, for
this server address only (never for other servers, SSH or the system):

- **Add a CA certificate…** with Caddy's root, the way for `tls internal`:
  `pki/authorities/local/root.crt` in Caddy's data directory
  (`~/.local/share/caddy/` for Caddy run by a user on Linux,
  `/var/lib/caddy/.local/share/caddy/` for the packaged service,
  `/data/caddy/` in the Docker image, `%AppData%\Caddy\` on Windows,
  `~/Library/Application Support/Caddy/` on macOS). The app shows the
  CA's name and fingerprint for confirmation
  (`openssl x509 -noout -fingerprint -sha256 -in root.crt` prints the same)
  and keeps it only when the server's certificate is issued by it. Caddy
  renews its 12-hour certificates by itself, and the app accepts each new
  one of that CA without a question (its root lives 10 years).
- **Trust this certificate** pins the one certificate; compare its
  fingerprint first
  (`openssl s_client -connect sync.home.arpa:443 </dev/null | openssl x509 -noout -fingerprint -sha256`).
  A pin suits a long-lived certificate: when the server shows another one,
  sync stops before any request and Account & Sync offers **Review certificate…**:
  "Trust the new certificate", "Use the system's trust" (only when the
  system trusts the new one) or Cancel. Confirming keeps the device signed
  in.

An expired certificate or one for another name is refused, pinned or not.
With Tailscale, the machine's `ts.net` name and Tailscale's HTTPS
certificates instead of `tls internal` give a publicly trusted certificate,
with no question at all.

### Client addresses

The rate limits use the client's address: the requests of the sign-in
endpoints per address, failed sign-ins per email and address (with a delay
of at most 4 minutes per email over all addresses, which never holds the
account's own devices, its signed-in requests or an address it signed in
from in the last 30 days), and the delays of two-step codes per account and
address (`docs/sync-protocol.md`, section 10.1). These counts are in
memory: a restart of the server forgets them. Behind a proxy
every request reaches the server from the proxy, which names the client in
`X-Forwarded-For`; the server believes that header only from
`SHELLPP_TRUSTED_PROXIES`, and from nobody else.

- A proxy on the host (Caddy, `cloudflared`) connects to `127.0.0.1:8080`;
  Docker hands the connection to the server from the gateway of the compose
  network, `172.16.88.1`, not from `127.0.0.1`. Keep
  `SHELLPP_TRUSTED_PROXIES=172.16.88.1/32`. With `127.0.0.1/32` every client
  would share the proxy's address and one budget of sign-ins.
- A proxy container in the compose project: give it a fixed address on the
  `default` network (`ipv4_address: 172.16.88.10`) and trust that address
  instead.
- If `172.16.88.0/24` is taken on your machine (`ip route` or `ipconfig`
  shows it, or `docker compose up` answers "Pool overlaps"), set
  `SHELLPP_SUBNET`, `SHELLPP_GATEWAY` and `SHELLPP_TRUSTED_PROXIES`
  (the gateway with `/32`) in `.env` together.

This holds only while the port is published on `127.0.0.1` alone, as in
the compose file: then only programs on this machine come from the
gateway. Published on every address (`8080:8080`), outside clients would
arrive from the gateway too and could name any address. Never trust a
whole range such as `172.16.0.0/12` either: anything inside it could name
any client address and step around the limits. The server logs once when a
request carried `X-Forwarded-For` from a peer it does not trust.

Check it: `docker network inspect shellpp_default -f '{{range .IPAM.Config}}{{.Gateway}}{{end}}'`
prints the address `SHELLPP_TRUSTED_PROXIES` names.

## Backups

The `backup` service writes `backups/shellpp-<time>.sql.age` every 24 hours
(a `pg_dump`, encrypted to `BACKUP_AGE_RECIPIENT`). Copy the directory off
the machine.

Rotation, after each backup (D199):

- backups not older than `BACKUP_RETENTION_DAYS` (30) whole days stay;
- before them, so do the first backup of each of the `BACKUP_KEEP_WEEKS`
  (4) calendar weeks and of each of the `BACKUP_KEEP_MONTHS` (3) calendar
  months before the end of the retention, and always the newest backup;
  older ones are deleted. With the defaults that keeps weekly backups up
  to about two months back and monthly ones up to four;
- nothing is deleted while the newest backup is less than
  `BACKUP_MIN_PERCENT` (50) percent of the size of the largest one since
  the last accepted shrink. A database emptied or cut down by mistake, at
  once or in steps, would otherwise push the good backups out night after
  night; `docker compose logs backup` then says "pruning paused". If you
  deleted that much on purpose, accept the new size once:
  `docker compose run --rm -e BACKUP_PRUNE_ONLY=1 -e BACKUP_ACCEPT_SMALLER=1 backup`
  (it keeps the newest backup's time and size in `backups/.accepted-size`;
  older, larger backups no longer count).
- A backup's time is the stamp in its name, so a backup copied back by
  hand keeps its place. The four settings must be whole numbers: anything
  else stops the service before it dumps (`docker compose logs backup`).
- On a very small database (a few records) the fixed part of the dump, the
  schema, can keep the dump of an emptied database above half the size of
  a good one; there, set `BACKUP_MIN_PERCENT` higher (80).

Its image is built once from `deploy/backup.Dockerfile` (Alpine with the
PostgreSQL client and `age`) by the first `docker compose up`, so a backup
needs no network. A dump or encryption that fails ends the run with an
error in `docker compose logs backup` (Docker starts it again, which tries
again after 5 minutes) and leaves no `.sql.age` behind; old backups are not
deleted after a failure.

Make one now: `docker compose run --rm -e BACKUP_ONCE=1 backup`.

## Restore

1. Stop the server: `docker compose stop server`.
2. Decrypt the backup on the machine that holds the private key and copy
   the `.sql` back: `age -d -i shellpp-backup.key shellpp-<time>.sql.age > restore.sql`.
3. Recreate the database and load it:

   ```bash
   docker compose exec postgres dropdb -U shellpp shellpp
   docker compose exec postgres createdb -U shellpp shellpp
   docker compose exec -T postgres psql -U shellpp -v ON_ERROR_STOP=1 --single-transaction shellpp < restore.sql
   rm restore.sql   # the plaintext dump: verifiers, wraps, metadata
   ```

4. **Always** run `docker compose run --rm server admin restored`. It gives
   the server a new generation: every device then compares its data with
   the server, pushes back what the backup lost, and keeps its own newer
   records (no conflict copies for the stale server versions).
5. Start: `docker compose start server`.

A restore brings back what was deleted after the backup on the server's
side: accounts deleted since (delete them again with `admin user delete`),
devices revoked since (`admin device revoke`), vaults deleted since.

A device whose sign-in was renewed after the backup (it renews every 15
minutes while it syncs, so after a nightly backup that is most of them) is
signed out by the restored server. Signing in again on it keeps its local
data: it compares it with the server and pushes back what the backup
lacks, as above. A load that fails half way stops at the first error
(`ON_ERROR_STOP`) and changes nothing (`--single-transaction`): fix the
cause and load again before `admin restored`.

If a device holds newer keys than the backup (a password change with "my
old password may be compromised" or a recovery key reissue after the
backup), it stops with "Server state rolled back" and offers "Bring the
server up to date…": it sends the newer key epochs it holds, after you
confirm with the account password. Nothing happens without that
confirmation, because a hostile server could claim the same.

## Update

From a version with `SHELLPP_SERVER_SECRET` in `.env`, first, while the
old server still runs, move the value it runs with into the file, never a
new one (a new secret makes every TOTP unreadable; the server refuses to
start with one):

```bash
cd server/deploy
mkdir -p secrets
docker compose exec -T server printenv SHELLPP_SERVER_SECRET > secrets/server_secret
chmod 0400 secrets/server_secret && sudo chown 10001 secrets/server_secret
```

then delete the `SHELLPP_SERVER_SECRET=` line from `.env`. Compose refuses
to start while the file is missing. Then:

```bash
git pull
docker build -f server/Dockerfile -t shellpp-server .
cd server/deploy
docker compose pull postgres
docker compose build --pull --no-cache backup
docker compose run --rm -e BACKUP_ONCE=1 backup
docker compose up -d
```

The `migrate` service applies schema changes before `server` starts; the
server refuses to run on a schema older than itself. `docker compose build
--pull --no-cache backup` rebuilds the backup image with its script and the
current PostgreSQL client and `age`.

From a version before the fixed network (an `.env` with
`SHELLPP_TRUSTED_PROXIES=127.0.0.1/32` or `172.16.0.0/12`): set it to
`172.16.88.1/32`, then `docker compose down` (never with `-v`, which would
delete the database volume) and `docker compose up -d`, so the network is
made again with its subnet, and check the gateway as in "Client
addresses".

## Configuration reference

| Variable | Default | |
|---|---|---|
| `SHELLPP_DATABASE_URL` | required | `postgres://…` or `sqlite:/data/server.db` |
| `SHELLPP_PUBLIC_URL` | required | the origin devices use |
| `SHELLPP_SERVER_SECRET_FILE` | required (compose: `/run/secrets/server_secret`) | a file holding the secret: base64, at least 32 bytes |
| `SHELLPP_SERVER_SECRET` | | the secret itself, instead of the file; works with a warning in the log; not together with the file |
| `SHELLPP_BIND` | `127.0.0.1:8080` (image: `0.0.0.0:8080`) | |
| `SHELLPP_REGISTRATION` | `invite` | or `open` |
| `SHELLPP_TRUSTED_PROXIES` | none (compose: `172.16.88.1/32`) | CIDRs whose `X-Forwarded-For` is believed ("Client addresses") |
| `SHELLPP_RATE_LIMIT_*` | see `docs/sync-protocol.md`, section 10.1 | rate limit overrides |
| `SHELLPP_QUOTA_MAX_RECORDS`, `SHELLPP_QUOTA_MAX_BYTES` | no limit | per account |
| `SHELLPP_TOMBSTONE_RETENTION_DAYS` | 90 | |

SQLite works for one person (`SHELLPP_DATABASE_URL=sqlite:/data/server.db`
with a volume on `/data`); the compose file uses PostgreSQL.

## Tests

From the repository root (PowerShell):

```
$env:SHELLPP_TEST=1; cargo test --manifest-path server/Cargo.toml --workspace
$env:SHELLPP_TEST_POSTGRES='docker'; cargo test --manifest-path server/Cargo.toml --workspace
$env:SHELLPP_TEST_DEPLOY='docker'; cargo test --manifest-path server/Cargo.toml --test deploy -- --test-threads=1
$env:SHELLPP_SYNC_LOAD=1; cargo test --manifest-path server/Cargo.toml --release --test load -- --nocapture
```
