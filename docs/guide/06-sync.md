# Sync between your devices

English | [Русский](ru/06-sync.md)

Sync is optional: Shell++ works fully without it. With it, your devices
keep the same hosts, keys, snippets, forwarding rules and known hosts
through a server **you run yourself**. App settings stay per device (only
custom terminal themes and highlighting rules sync). Everything is encrypted on your devices before it is sent: the
server stores only encrypted records and never sees your master password,
your recovery key or the content of your vault.

This chapter covers:

1. [installing the server](#installing-the-server) in a homelab;
2. [making it reachable](#three-ways-to-reach-the-server) from your devices
   (three ways);
3. [connecting the first device](#connecting-the-first-device) and
   [a second device](#a-second-device);
4. [day-to-day use](#day-to-day-sync): what syncs, reviews, Recently
   deleted, devices;
5. [backups and restore](#backups).

## Installing the server

You need a machine that stays on (a homelab box, a NAS with Docker, a small
VPS) with Docker and Docker Compose v2. The steps below follow
[`server/README.md`](../../server/README.md), which has the full reference.

### 1. Get the server image

From a checkout of the repository (in its root folder):

```bash
docker build -f server/Dockerfile -t shellpp-server .
```

When a release is published, you can load its image instead of building
it: download `shellpp-server-<version>-docker-image.tar.gz` from the
release, check it ([docs/release.md](../release.md#checking-a-download)),
and run `docker load -i shellpp-server-<version>-docker-image.tar.gz`. It
gives the images `shellpp-server:<version>` and `shellpp-server:latest`.

### 2. Configure it

```bash
cd server/deploy
cp .env.example .env
```

Fill in `.env` (all commands below run in `server/deploy`):

| Setting | What to put there |
|---|---|
| `POSTGRES_PASSWORD` | a long random password: `openssl rand -base64 32` |
| `SHELLPP_PUBLIC_URL` | the address your devices will type, for example `https://sync.example.com` |
| `SHELLPP_SERVER_SECRET` | `openssl rand -base64 48`. Keep a copy with your backup key: a restored server needs the same secret, or two-step sign-in has to be set up again for every account |
| `BACKUP_AGE_RECIPIENT` | the public key of an `age` key pair made **on another machine** with `age-keygen -o shellpp-backup.key`; the private key never goes on the server |
| `SHELLPP_REGISTRATION` | `invite` (the default: new accounts need a code from you) or `open` |
| `SHELLPP_TRUSTED_PROXIES` | keep the value from `.env.example` (`172.16.0.0/12`): with the compose file, your proxy's connections reach the server from Docker's network. (`server/README.md` says `127.0.0.1/32` in its Access section; it only affects which client addresses the rate limits see.) |

Decide the server's address (`SHELLPP_PUBLIC_URL`) before you create the
account, and type it the same way on every device (the same name and
port; case and a trailing slash do not matter): the account's sign-in key
is derived from the address the devices type. Moving the server to another
address later means signing in no longer works.

### 3. Start it

```bash
docker compose up -d
docker compose logs -f server
```

The compose file runs PostgreSQL, applies the database schema (the
`migrate` service), starts the server on `127.0.0.1:8080` only, and makes
an encrypted backup every day. Check it with
`curl http://127.0.0.1:8080/healthz` (`{"status":"ok"}`).

### 4. Create an invitation

```bash
docker compose run --rm server admin user invite --uses 1 --days 7
```

The code is shown once; you will type it when you create the account.
Other admin commands (`docker compose run --rm server admin --help`):

| Command | What it does |
|---|---|
| `admin user list` | lists the accounts |
| `admin user delete <email or id>` | deletes an account and its data |
| `admin user reset-totp <who>` | for someone who lost their authenticator |
| `admin user disable <who>` / `enable <who>` | signs every device of an account out / allows it again |
| `admin device revoke <device id>` | signs one device out |
| `admin user quota <who> --records N --bytes N` | storage limits (none unless set) |

## Three ways to reach the server

The server itself only listens on `127.0.0.1:8080`. Your devices reach it
over HTTPS through one of these (pick one):

### 1. Your own domain, with Caddy and Let's Encrypt

The simplest when the machine can be reached from the internet: a DNS name
such as `sync.example.com` points to it, and ports 80 and 443 are open.
Copy `Caddyfile.example` to `Caddyfile` (so that a later `git pull` does not
clash with your edit), put your name in it and run Caddy on the same
machine:

```bash
cp Caddyfile.example Caddyfile
caddy run --config Caddyfile --adapter caddyfile
```

Caddy gets and renews a certificate from Let's Encrypt. The app trusts it
like any website: there is nothing to confirm.

### 2. Cloudflare Tunnel

No open ports and no public IP needed. Create a tunnel and route your name
to it:

```bash
cloudflared tunnel create shellpp
cloudflared tunnel route dns shellpp sync.example.com
```

Copy `cloudflared.example.yml` to `cloudflared.yml`, put the tunnel's id
and credentials file into it and run
`cloudflared tunnel --config cloudflared.yml run`. Cloudflare
terminates TLS: it sees the encrypted records and your devices' metadata,
never your data.

### 3. Only inside your VPN

The server is reachable only inside your WireGuard or Tailscale network,
for example as `sync.home.arpa`, with a certificate from Caddy's own CA
(`Caddyfile.internal.example`, `tls internal`). The app does not know that
CA, so the first connection shows the certificate: compare its SHA-256
fingerprint with the server's

```bash
openssl s_client -connect sync.home.arpa:443 </dev/null | openssl x509 -noout -fingerprint -sha256
```

and click **Trust this certificate**. The app then trusts this certificate
for this address only.

Caddy's internal certificates last 12 hours and are renewed before they
expire, and a renewed certificate no longer matches the one you trusted:
sync stops with "Last error: certificate_changed". In this version the only
way to trust the new certificate is **Sign out…**, then **Connect sync
server** again (it shows the new certificate), **Trust this certificate**
and **Sign in**. So with a VPN, set things up **before you create the
account** so that the app trusts the certificate by itself, with either:

- Tailscale: use the machine's Tailscale name
  (`<machine>.<tailnet>.ts.net`, with HTTPS certificates turned on for the
  tailnet) in the Caddyfile instead of `tls internal`, and as
  `SHELLPP_PUBLIC_URL`. Caddy then gets a publicly trusted certificate for
  it from Tailscale;
- or add Caddy's root certificate (`pki/authorities/local/root.crt` in
  Caddy's data folder) to the trusted root certificates of each device's
  operating system **before the device first connects**: the app then
  accepts the server through the system's trust store and asks nothing. (A
  device that already trusted a single certificate keeps comparing with
  it.) Every program on that device then trusts certificates from this
  CA.

## Connecting the first device

On the device whose data should start the account:

1. Settings (Ctrl+,) > **Account & Sync** > **Connect sync server**.
2. Enter the server address (`sync.example.com` is enough: `https://` is
   added) and click **Continue**. If the certificate is not trusted yet,
   check it as described above.
3. Choose **Create account**: your email, **this device's master
   password** (it becomes the account's password), and the invitation
   code.
4. Shell++ shows your account's **recovery key**. With it you can set a
   new master password if you forget yours. Save it outside this device
   (**Copy** clears the clipboard after 30 seconds), tick **I saved the
   recovery key** and click **Finish**. You can see it again later in
   Settings > **Security** > **Recovery key** > **Show…**.
5. Sync starts and sends the vault, encrypted, in the first sync.

![Connecting a sync server](images/sync-connect.png)

## A second device

**A new device without data**: on the first-run screen, click **Restore
from sync…**, enter the same server address and sign in with your email and
the **account's** master password (and the two-step code if it is on). The
device gets the account's vault; from then on it opens with the account
password.

**A device that already has its own data**: Settings > **Account & Sync** >
**Connect sync server** > **Sign in**. Shell++ first writes a backup of the
local database (`backups\` in the data folder), then merges the local data
into the account; from then on the device opens with the account password.
Items that exist on both sides with identical content are merged; items
that only look alike (for example the same host in another group) are kept
side by side. The screens that show them say so ("2 hosts may be
duplicates", with **Review**), and they are listed under **Review…** >
**Possible duplicates**: **Compare…** shows the two side by side, with the
fields that differ marked; **Keep left** or **Keep right** keeps one (the
links move to it, the other goes to Recently deleted), **Keep both** keeps
both and does not ask again.

![Possible duplicates side by side](images/sync-duplicates.png)

A device that was connected to one account cannot be connected to another
account in this version.

![The review dialog](images/sync-review.png)

## Day-to-day sync

Shell++ syncs right after you unlock, every minute while it is unlocked,
and a couple of seconds after each change. Settings > **Account & Sync**
shows the account, when the device last synced and what is still waiting,
with **Sync now**.

![The account](images/sync-account.png)

- **What syncs** is chosen per device. Hosts and groups, keys and
  identities, snippets, forwarding rules, known hosts, workspaces, terminal
  themes and highlighting rules sync by default; command history and the
  connection log do not. Turning a type off stops syncing it and deletes
  nothing.
- **Conflicts**: when two devices changed the same item, the newer change
  is in place and the other version is kept. Account & Sync then shows
  "Conflicts: N", and the item's card (a host, key, snippet and so on) a
  **Conflict** badge; both open
  **Compare versions**, the current version and the other one side by side
  with the device and time of each and the differing fields marked.
  **Keep this** or **Keep other** decides; **Close** leaves it for later.
- **Review…** (with a count when there is something to look at) lists the
  conflicts, the possible duplicates (above) and the items **Brought
  back**: deleted elsewhere long ago but still here, now restored to the
  server (**Keep** or **Delete again**).
- **Recently deleted…** opens the Recently deleted screen (below).
- **Devices…** lists the account's devices. **Sign out** next to another
  device asks whether it was lost or stolen. **Lost or stolen** leads you
  to change the master password with **My old password may be
  compromised** fixed on: the new recovery key is shown first; after **I
  saved the recovery key**, **Change password** signs the device out and
  changes the password, the vault key and the recovery key at once, so
  the old password and the old recovery key the device holds stop
  working. **Cancel** only signs the device out. Nothing is wiped on the
  lost device itself: change the passwords of your important servers
  too.
- **Two-step sign-in…** adds a one-time code (TOTP) to every new sign-in:
  enter the master password, scan the QR code with an authenticator app
  and enter its code. It cannot be turned off from the app in this
  version; the server owner can reset it (`admin user reset-totp`).

![Comparing two versions](images/sync-conflict.png)

## Recently deleted

Deleted hosts, groups, keys, snippets and other items go to the
**Recently deleted** screen (in the sidebar, below the separator), with or
without a sync server. They stay there for 90 days, on this device only.

![Recently deleted](images/recently-deleted.png)

- **Search by name** and **Filter by type** narrow the list; the table
  shows each item's name, type, vault, when it was deleted and where (this
  device or another one).
- Select items (click, Ctrl+click, Shift+click, Ctrl+A), then **Restore**
  (it comes back where it was, and syncs), **Export…** (saves them to a
  file with Export Data) or **Delete forever**. **Empty trash** deletes
  everything. Both ask first. The same actions are in a row's right-click
  menu.
- Click one item to see its details, read-only, with secrets masked.

## Signing out and deleting the account

- **Sign out…** stops sync on this device. The local copy stays and opens
  with the same password. (In this version, leave **Also delete the local
  copy on this device** off: with it, the device is signed out but the
  copy is not deleted, and the dialog says "Something went wrong".)
- **Delete account…** deletes the account and its data on the server
  (after the master password, and the two-step code if it is on). Every
  device keeps its local copy; **Export data…** first if you want a file.

## Forgot password? Restore from sync

On a device connected to a sync server, **Forgot password?** on the unlock
screen offers **Restore from sync**: enter the account's recovery key
(**Recovery key**), a new master password twice, and the two-step code if
it is on. The new
password becomes the account's password; other devices sign in again with
it. If the server cannot be reached, **Recover only this device** sets the
new password for the local copy only.

## The recovery key

The recovery key is shown when you create the account. Later, Settings >
**Security** > **Recovery key** (on any device of the account) offers:

- **Show…**: after the master password, the key, with **Copy**.
- **Reissue…**: a new key replaces the current one (if someone else may
  know it). The new key is shown first; nothing changes until you tick **I
  saved the recovery key** and click **Reissue**. The old key then stops
  working, and your other devices sign in again.

If the change does not go through, the dialog says what is in effect.
When another device changed the recovery key at the same time, sign in
again on this device and use **Show…** to see the valid key.

When your vault gets a new key on another device, a device with
**Remember on this device** keeps the option on after it signs in again.
If its key is no longer in the system's credential store, the option goes
off, and Settings > **Security** says so; turn it on again there.

## Backups

### The server

The compose file's `backup` service writes
`backups/shellpp-<time>.sql.age` every 24 hours (a database dump encrypted
to your `age` key) and deletes the ones older than 30 days. Copy that
folder off the machine regularly. Make one now with

```bash
docker compose run --rm -e BACKUP_ONCE=1 backup
```

To restore a backup:

1. `docker compose stop server`
2. On the machine with the private key:
   `age -d -i shellpp-backup.key shellpp-<time>.sql.age > restore.sql`,
   then copy `restore.sql` into `server/deploy` on the server.
3. Recreate the database and load the dump:

   ```bash
   docker compose exec postgres dropdb -U shellpp shellpp
   docker compose exec postgres createdb -U shellpp shellpp
   docker compose exec -T postgres psql -U shellpp shellpp < restore.sql
   ```

4. **Always** run `docker compose run --rm server admin restored`. Devices
   then compare their data with the server and send back what the backup
   lost.
5. `docker compose start server`

If a device holds newer keys than the backup (after a password change with
"My old password may be compromised"), it shows "Server state rolled
back" with **Bring the server up to date…**, which asks for the account
password before it sends anything.

### Your devices

Each device keeps the three newest copies of its database in `backups\`
of its data folder, made before an upgrade changes the database format and
before a sign-in merges its data into an account. For a backup you control, use
[Export Data](07-export-and-migration.md#export-data).

### Updating the server

From the repository root:

```bash
git pull
docker build -f server/Dockerfile -t shellpp-server .
cd server/deploy
docker compose run --rm -e BACKUP_ONCE=1 backup
docker compose up -d
```

The `migrate` service updates the database schema before the server
starts.
