# Security and threat model

This is the threat model required by `SPEC.md` ("Security"). It states what Shell++ protects, from whom, how, and what it does not protect. It is normative: code that contradicts it is a bug, and changes to it follow the specification change process.

> **Note (2026-10-19).** The UI moved from Tauri + React to GPUI Kit ([decisions.md](decisions.md), D27). The sections that describe Tauri, the webview, IPC commands and events, and the CSP (mainly section 13) are superseded by D27 and will be rewritten; where they differ, D27 and `SPEC.md` win.

- `Dn` cites decision n in [decisions.md](decisions.md). "UI n.m" cites section n.m of [ui-behavior-spec.md](ui-behavior-spec.md).
- Choices made while writing the stage 0 documents that are not in the specifications or the decisions are marked "(stage 0 decision)".
- This document owns: the threat model, the key hierarchy, the KDF and HKDF parameters, HPKE parameters, fingerprints and human-readable secrets, lock behavior, the list of server-visible metadata, the history capture heuristics and the accepted risks.
- Byte layouts (wrap blob, record envelope, AAD, padding, export and recording files) are in [storage-format.md](storage-format.md). Components, crates and directories are in [architecture.md](architecture.md); record types and sync classes in [data-model.md](data-model.md); the HTTP API, epochs, invitations, roles and limits in [sync-protocol.md](sync-protocol.md).

## 1. Goals and non-goals

Goals:

1. Confidentiality at rest: everything in the data directory except the metadata listed in section 9.3 and the short-lived files of explicit user actions (section 9.3) is encrypted under keys that only the master password (or the recovery key) opens, and, while "remember on this device" is on (the default since D29), the device key in the OS keychain (section 8).
2. Zero-knowledge sync: the sync server stores ciphertext and the metadata of section 14.2; keys and plaintext never leave the client (principle 5).
3. Integrity: a substituted, reordered or modified record is detected and not applied.
4. Local availability: no server, member or administrator can wipe or lock data already on the device (principle 3).
5. Host authenticity: SSH connections never silently accept a changed host key (UI 10.2).
6. Least privilege inside the app: the webview gets only the commands and data it needs; untrusted data never becomes code.
7. Secret hygiene: secrets never reach logs, URLs or plaintext files except through the explicit user actions of section 9.3.
8. No phone-home: no telemetry, no license checks, no remote disabling (principle 2).

Non-goals:

- Protection against malware running as the same OS user while the app is unlocked or while the password is typed, or against a compromised OS.
- Hiding metadata from the sync server (section 14.2) or hiding activity patterns from a network observer.
- Protection against the members of a team vault: every member, viewers included, can read every secret of the vault (D13).
- Detecting rollback or deletion of records by the sync server (D14).
- Availability of the sync server.

## 2. Assets

| Asset | Examples | Where it lives | Main property |
|---|---|---|---|
| Master password | | the user's memory; process memory during key derivation | confidentiality |
| Recovery key | `RK1-…` | record `recovery_key` in the personal vault; the user's paper copy once it is shown at registration (in local mode it is never shown, D29) | confidentiality |
| Device key | 32 random bytes | OS keychain while "remember on this device" is on (the default, D29) | confidentiality |
| Vault keys and derived keys | section 5.3 | wrapped in the database and on the server; memory while unlocked | confidentiality |
| Credentials | SSH private keys, passphrases, host and identity passwords, certificates, FIDO2 data | records (personal or team vault) | confidentiality |
| Host configuration | addresses, usernames, groups, jump hosts, proxies, environment variables, startup snippets, forwarding rules | records | integrity (a change can redirect a connection or run a command), confidentiality |
| Snippets | scripts with variables | records | integrity, confidentiality |
| Command history, connection log, session recordings | | records; `recordings/` files | confidentiality |
| Known hosts | trusted host keys, revoked keys | records in the personal vault | integrity |
| Account keys | X25519 private key, SSH ID private key | records in the personal vault | confidentiality |
| Account access | `auth_key`, access and refresh tokens, TOTP secret | memory; wrapped refresh token; server | confidentiality |
| Live sessions | SSH channels, terminal output, forwarded ports, transfers | memory, network | confidentiality, integrity |
| Local data availability | everything already on the device | data directory | availability (principle 3) |
| Metadata | counts, timestamps, memberships, emails | plaintext columns; server | minimization |
| Code and updates | app binaries, update packages, dependencies | releases, registries, CI | integrity |

## 3. Actors and attacker capabilities

| ID | Actor | Can | Cannot (assumed) |
|---|---|---|---|
| A1 | Local attacker with disk access | Read, copy and modify every file of the user: the data directory, `backups/`, logs, `tmp/`, OS backups, swap and hibernation files; put back an older copy of the database. | Run code in the user's logged-in session; read OS keychain items without the OS account password. |
| A2 | Malware as the same OS user | Everything A1 can, plus: read process memory, log keystrokes, read and write the clipboard, read the user's keychain items (Windows Credential Manager, an unlocked Secret Service collection), connect to local listeners, replace the per-user installed binary. | — (out of scope while unlocked: section 1, RR8) |
| A3 | Network attacker | Observe, block, delay and modify traffic to SSH servers, the sync server and the update source; hold a TLS certificate mis-issued for the sync server's name, which makes it an active sync server (A6) for that connection, without the stored data. | Break TLS (rustls), the SSH transport, AEAD, HPKE or Argon2id. |
| A4 | Malicious SSH server | Control everything a host sends: terminal output and escape sequences, banners and prompts, SFTP listings, file names and contents, channel open requests, history files, OS detection output; learn passwords sent to it; use a forwarded agent while the session lasts. | Obtain data the client never sends to it. |
| A5 | Passive sync server | Honest-but-curious operator, or a leaked database or backup: read everything stored and every request (metadata, envelopes, wraps, verifiers, KDF parameters and salts); run offline password guessing. | Change responses. |
| A6 | Active sync server | A5, plus: answer anything; roll back, drop, withhold, reorder or replay records; show different states to different devices; forge member lists, public keys, epochs and wraps; force full resyncs (`cursor_expired`); deny service. | Produce valid envelopes or wraps without the keys; read data under keys it does not hold. |
| A7 | Malicious team member | Hold the vault key of each vault it belongs to. Viewers read all secrets of the vault (D13); editors also change and delete records; owners also manage membership. With A6, a viewer can also write. | Read other vaults or other members' personal vaults. |
| A8 | Removed member | Keep a read-only local archive of everything received before removal (principle 3) and the keys of the epochs before removal. | Decrypt epochs created after removal, as long as the server is honest (with an active server, see RR21); sync with an honest server (`not_member`). |
| A9 | Lost or stolen device | A1 on that device; A2 as well if the OS session was open. | — |
| A10 | Malicious content source | Craft import files (ssh config, PuTTY, CSV, JSON, known_hosts, keys, themes), `shellpp://` links opened by a web page or another app, and text the user pastes. | — |

## 4. Trust boundaries

```text
+---------------------------------- user's device, one OS user ----------------------------------+
|                                                                                                |
|  Webview (React UI, xterm.js)  <== TB1: Tauri IPC, allowlisted commands ==>  Rust core         |
|                                                                              keys, crypto,     |
|                                                                              SSH, SFTP, sync,  |
|                                                                              SQLite            |
|                                                                                                |
|  TB2: operating system: data directory, OS keychain, clipboard, child processes, crash dumps   |
|  TB6: local listeners (forwarded ports, SOCKS) reachable by other processes and users          |
|                                                                                                |
+------------------------------------------------------------------------------------------------+
        | TB3: HTTPS                        | TB4: SSH                     | TB7: external input
        v                                   v                              v
  Sync server (ciphertext only)       SSH servers (untrusted output)   import files, shellpp:// links,
        |                                                              paste, update manifest
        | TB5: shared vault keys
        v
  Other members of a team vault
```

| ID | Boundary | What crosses | Controls (section) |
|---|---|---|---|
| TB1 | Webview ↔ Rust core | UI requests, display data, events | CSP and capabilities (13); secret fields stay in the core (9.5) |
| TB2 | App ↔ OS | files, keychain items, clipboard, child processes, dumps | encryption at rest (5), permissions (9.3), only the device key in the keychain, while "remember on this device" is on (8), clipboard clearing (9.4), no core dumps (9.5) |
| TB3 | Client ↔ sync server | auth keys, tokens, envelopes, wraps, public keys | HTTPS, end-to-end encryption, AAD, KDF bounds, pins (5, 14) |
| TB4 | Client ↔ SSH server | credentials, keystrokes, output, files, channel requests | host key checks, escape-sequence policy, name checks, channel allowlist (11, 12) |
| TB5 | User ↔ team members | vault keys and every record of the vault | HPKE Auth, pins, rosters, fingerprints, epochs, server-enforced roles (5.7, 15) |
| TB6 | Local listeners ↔ other processes | forwarded connections | loopback bind by default (11.3) |
| TB7 | External input ↔ app | files, links, paste, updates | strict parsing, no execution of imported commands, confirmations, update signatures (12, 13) |

## 5. Cryptography and keys

### 5.1 Primitives

No homemade cryptography: only the constructions and crates below (SPEC "Default stack", D4, D5).

| Purpose | Construction | Crate |
|---|---|---|
| Password and export-password KDF | Argon2id, version 0x13 | `argon2` |
| Key derivation | HKDF-SHA256 | `hkdf`, `sha2` |
| Records, key wraps, encrypted export | XChaCha20-Poly1305, 24-byte random nonce | `chacha20poly1305` |
| Session recordings | STREAM over XChaCha20-Poly1305 | `chacha20poly1305` (`aead` stream module) |
| Team vault key wrapping | HPKE (RFC 9180), section 5.7 | `hpke`, `x25519-dalek` |
| Hashes, fingerprints, MAC | SHA-256, HMAC-SHA256 | `sha2`, `hmac` |
| Randomness | OS CSPRNG | `getrandom`, `rand_core` (`OsRng`) |
| Constant-time comparison | | `subtle` |
| Secret memory | wipe on drop, redacted `Debug` | `zeroize`, `secrecy` |
| SSH | russh with the `ring` backend (D4); `rsa` is a documented exception (RUSTSEC-2023-0071) | `russh`, `russh-sftp`, `ssh-key` |
| Sync TLS | rustls with the `ring` provider, system roots | `reqwest` |
| Hashed `known_hosts` entries | HMAC-SHA1 as defined by OpenSSH, for matching only (D21) | `hmac` and RustCrypto `sha1` ([architecture.md](architecture.md), section 8.2) |
| Server verifiers and TOTP | Argon2id PHC; RFC 6238 | `argon2`, `totp-rs` (server only) |

### 5.2 Key hierarchy

```text
master password ──Argon2id(argon2_salt(email, kdf_salt), params)──► master_key (32 B, memory only)
   master_key ──HKDF "shellpp/v1/kek"──► kek
   master_key ──HKDF "shellpp/v1/auth" ‖ origin─► auth_key ──► sent to server, stored as Argon2id verifier

recovery_key (32 B random) ──HKDF "shellpp/v1/recovery-kek"──► recovery_kek
recovery_key ──HKDF "shellpp/v1/recovery-auth" ‖ origin─► recovery_auth_key ──► server verifier

device_key (32 B random, OS keychain, with "remember on this device", on by default) ──HKDF "shellpp/v1/device-wrap"──► device_wrap_key

vault_key[personal, e]  wrapped by: kek, recovery_kek, device_wrap_key (optional)
vault_key[team, e]      wrapped by: HPKE Auth to each member's X25519 public key
vault_key[v, e-1]       wrapped by: chain_key[v, e]  (epoch chain, so the newest key opens all older ones)

vault_key[v, e] ──HKDF──► record_key[v, e]   (encrypts record envelopes)
                ──HKDF──► chain_key[v, e]    (wraps vault_key[v, e-1])
vault_key[personal, e] ──HKDF──► local_secrets_key, recording_key[file]

personal vault records hold: user X25519 keypair, SSH ID private key, recovery key copy
```

`kek` and `auth_key` are independent HKDF outputs: the server, which sees `auth_key`, learns nothing about `kek`. `auth_key` and `recovery_auth_key` also depend on the server origin (section 5.5), so the values one server receives are useless on another.

### 5.3 Keys

| Key | Size | Origin | Stored where | Destroyed or replaced | Source |
|---|---|---|---|---|---|
| `master_key` | 32 B | Argon2id output | memory only, zeroized right after HKDF | right after HKDF | D11 |
| `kek` | 32 B | HKDF from `master_key` | memory only, zeroized after unwrapping | replaced on password change, email change, registration and recovery | D8, D11 |
| `auth_key` | 32 B | HKDF from `master_key` | sent to server on login and sensitive operations; never stored by the client | wiped after the request | D11 |
| `vault_key[v, e]` | 32 B | CSPRNG | only wrapped (local `key_wraps`, server); plaintext in memory while unlocked | wiped on lock; superseded by epoch `e + 1` | D8 |
| `record_key[v, e]`, `chain_key[v, e]` | 32 B | HKDF from vault key | memory only | wiped on lock | stage 0 decision |
| `device_key` | 32 B | CSPRNG | OS keychain, only with "remember on this device" (on by default, D29) | deleted with its wrap when the option is turned off or the local copy is deleted; kept at "Start with a new vault", where it opens only the archived file (section 8) | D8, D29 |
| `recovery_key` | 32 B | CSPRNG at first password | copy inside the personal vault (record `recovery_key`) so it can be shown after password re-entry (UI 13.4). In local mode it is never shown (D29); with an account it is shown at registration (UI 13.5), and the user keeps a paper copy | replaced on reissue; a new one at a registration that moves data bound to another account (section 5.9) | D11, D29; vault copy: stage 0 decision |
| `recovery_kek`, `recovery_auth_key` | 32 B | HKDF from `recovery_key` | memory only; server keeps a verifier of `recovery_auth_key` | wiped after recovery or reissue | D11 |
| user X25519 keypair | 32 B + 32 B | CSPRNG at registration | private: record `user_keypair` in the personal vault; public: server | no rotation in the MVP | D13 |
| SSH ID key | Ed25519 | generated at registration | private: record `ssh_id` in the personal vault (OpenSSH format); public: server | regenerated by the user | D13 |
| `local_secrets_key` | 32 B | HKDF from current personal vault key | memory only; encrypts the refresh token at rest | wiped on lock | stage 0 decision |
| `recording_key[file]` | 32 B | HKDF from personal vault key | memory only | wiped when the file is closed (an open recording keeps its key while locked, section 8) | D16; derivation: stage 0 decision |
| `export_key` | 32 B | Argon2id(export password) then HKDF | memory only | wiped after the export or import | stage 0 decision |
| `server_secret` | ≥ 32 B | operator, env `SHELLPP_SERVER_SECRET` | server env only; never leaves the server | keep stable (section 14.4) | stage 0 decision |

Other secrets:

- Access token: memory only, 15 minutes.
- Refresh token: `server_account.refresh_token`, a wrap blob under `local_secrets_key`; 30 days, rotated on every use; reuse revokes the whole family ([sync-protocol.md](sync-protocol.md)).
- SSH private keys, saved passphrases, host and identity passwords, FIDO2 data: fields of record payloads, inside envelopes.
- TOTP secret: server only, wrap blob under `totp_at_rest_key`.
- Tokens, tickets, invitation tokens, registration codes and email codes: the server stores only their SHA-256.

Rules:

- All key material is held in `zeroize` / `secrecy` types and wiped on lock (D9). While locked, only live SSH transport state stays in memory (sessions keep running, D9).
- Unlock check: a successful AEAD unwrap of `vault_key[personal, current]` proves the password. There is no separate password verifier on disk.
- A device that was never bound to an account has no X25519 keypair and no SSH ID; both are created at registration. A device that was bound before keeps the earlier account's keypair and SSH ID after log out, as records of that account that are never carried into another account ([data-model.md](data-model.md), section 18.5). (stage 0 decision)
- Rotation of the user X25519 keypair is not in the MVP. (stage 0 decision)

### 5.4 KDF (Argon2id)

| Parameter | Value | Source |
|---|---|---|
| Algorithm | Argon2id, version 0x13 (1.3), crate `argon2` (RustCrypto) | SPEC, D11 |
| KDF scheme version `v` | `1` = Argon2id → 32-byte output → HKDF labels `shellpp/v1/*`. The Argon2id salt is `argon2_salt` (below) for the master password, whose output is `master_key`, and the file's own random `salt` for an encrypted export | D11; value: stage 0 decision |
| Stored salt | 16 random bytes (`kdf_salt`), new on every password change and email change, and at a registration that moves data bound to another account (section 5.9) | D11; size: stage 0 decision |
| Argon2id salt | `argon2_salt = SHA-256("shellpp/v1/kdf-salt" ‖ be16(len(e)) ‖ e ‖ kdf_salt)`, all 32 bytes, where `e` is the UTF-8 of the normalized account email (trim, NFC, lowercase) and `len(e)` its length in bytes. A device that has never been bound to an account has no email: `e` is empty. The email binds the salt to one account, so a server that hands one `kdf_salt` to many users cannot test a password guess against all of them at once | stage 0 decision |
| Output | 32 bytes | stage 0 decision |
| Secret / associated data | not used | stage 0 decision |
| Desktop preset (default) | `m_kib = 65536` (64 MiB), `t = 3`, `p = 4` | stage 0 decision |
| Mobile preset | `m_kib = 32768` (32 MiB), `t = 4`, `p = 2` | stage 0 decision |
| Client-enforced floor | `alg = argon2id`, `v = 1`, `m_kib ≥ 32768`, `t ≥ 3`, `m_kib × t ≥ 131072` (the cost of the mobile preset), `1 ≤ p ≤ 16` | stage 0 decision |
| Client-enforced ceiling | desktop: `m_kib ≤ 1048576`, `t ≤ 16`; mobile: `m_kib ≤ 262144`, `t ≤ 16` | stage 0 decision |
| Minimum master password length | 8 characters, counted as Unicode scalar values after NFC normalization; no composition rules | D29 (was 10, stage 0 decision) |

JSON form, used locally (`local_auth.kdf`), in the API and in the encrypted export:

```json
{"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4}
```

- Password encoding before the KDF: UTF-8 of the NFC-normalized string, no trimming.
- Parameters outside the floor or the ceiling of the platform are refused, wherever they come from (`local_auth`, pre-login, an encrypted export). The floor stops a malicious server from making the captured `auth_key` cheaper to attack than the weakest preset; the ceiling stops it from forcing derivations that run for minutes or exhaust the memory of a phone. (stage 0 decision)
- Parameters belong to the account, not the device: every device uses the stored parameters. A preset applies only when a password is created or changed on that platform. Clients never write parameters above the mobile ceiling, so every account can sign in on every platform. (stage 0 decision for the last rule)
- When the stored parameters are below the current preset, the client upgrades them at the next password change.
- The email inside `argon2_salt` is the email the salt is bound to, stored as `local_auth.kdf_email` ([storage-format.md](storage-format.md)). It is always the normalized email used in the derivation that produced the stored `kek` wrap (the email the user typed at sign-in, the email accepted by `recover/begin` at a recovery), never an email the server reports. Registration re-derives the keys with the account email, an email change re-derives them with the new email (section 5.9), and log out and account deletion keep the binding, so the same password keeps opening the local copy. (stage 0 decision)
- On a device bound to an account, a pre-login answer whose `kdf` or `kdf_salt` differs from `local_auth` means the password or the email changed on another device. The client says so and asks for the current account password before it derives anything. (stage 0 decision)
- The encrypted export has no email: its Argon2id salt is its own random `salt` ([storage-format.md](storage-format.md)). The floor and the ceiling of the importing platform apply when it is imported, so a crafted file cannot force weak or excessive parameters. (stage 0 decision)
- Server-side verifiers use Argon2id PHC strings with `m = 19456 KiB, t = 2, p = 1` over the high-entropy auth keys. (stage 0 decision)

### 5.5 HKDF derivations

HKDF-SHA256 (crates `hkdf`, `sha2`), zero-length salt, output length 32 bytes. `info` is the ASCII label followed by the binary context, if any. (labels: stage 0 decision)

| Output | IKM | info | Source |
|---|---|---|---|
| `kek` | `master_key` | `"shellpp/v1/kek"` | D11 |
| `auth_key` | `master_key` | `"shellpp/v1/auth"` ‖ `be16(len(o))` ‖ `o` | D11; origin: stage 0 decision |
| `recovery_kek` | `recovery_key` | `"shellpp/v1/recovery-kek"` | D11 |
| `recovery_auth_key` | `recovery_key` | `"shellpp/v1/recovery-auth"` ‖ `be16(len(o))` ‖ `o` | D11; origin: stage 0 decision |
| `device_wrap_key` | `device_key` | `"shellpp/v1/device-wrap"` ‖ `db_id` (16) | D8 |
| `record_key[v, e]` | `vault_key[v, e]` | `"shellpp/v1/record"` ‖ `vault_id` (16) ‖ `be32(e)` | stage 0 decision |
| `chain_key[v, e]` | `vault_key[v, e]` | `"shellpp/v1/epoch-chain"` ‖ `vault_id` (16) ‖ `be32(e)` | stage 0 decision |
| `epoch_commit[v, e]` (roster entries) | `vault_key[v, e]` | `"shellpp/v1/roster-epoch"` ‖ `vault_id` (16) ‖ `be32(e)`; only the first 16 bytes of the output are stored | stage 0 decision |
| `local_secrets_key` | `vault_key[personal, e]` | `"shellpp/v1/local-secrets"` ‖ `device_id` (16) ‖ `be32(e)` | stage 0 decision |
| `recording_key[file]` | `vault_key[personal, e]` | `"shellpp/v1/recording"` ‖ `file_id` (16) | D16 |
| `export_key` | Argon2id(export password, export salt) | `"shellpp/v1/export"` | stage 0 decision |
| `prelogin_salt_key` (server) | `server_secret` | `"shellpp-server/v1/prelogin-salt"` | D11 |
| `totp_at_rest_key` (server) | `server_secret` | `"shellpp-server/v1/totp-at-rest"` | stage 0 decision |

Fake pre-login salt for an unknown email (D11): `HMAC-SHA256(prelogin_salt_key, normalized_email)[0..16]`, returned with the desktop preset.

In `auth_key` and `recovery_auth_key`, `o` is the ASCII of the normalized origin of the sync server the request goes to (`server_account.server_url`, [sync-protocol.md](sync-protocol.md), section 2) and `len(o)` its length in bytes. A server that answers pre-login with the salt and parameters another server stores for the same email, or that receives a registration or a recovery from a device that used the same password or recovery key with another server, gets values that the other server rejects. The cost is that an account signs in and recovers only under the origin it was registered with (section 14.4). (stage 0 decision)

### 5.6 Encrypted objects

Layouts and exact AAD bytes are in [storage-format.md](storage-format.md); this table states what each object protects.

| Object | Key | Construction and binding |
|---|---|---|
| Record envelope | `record_key[v, e]` | XChaCha20-Poly1305; the AAD binds version, class, flags (tombstone), vault id, record id, revision and key epoch (D14); plaintext padded to a multiple of 256 bytes (D10) |
| Wrap blob | `kek`, `recovery_kek`, `device_wrap_key`, `chain_key[v, e]`, `local_secrets_key` | XChaCha20-Poly1305; the AAD names the purpose, the vault or device and the epoch, so a wrap cannot be moved to another slot |
| Team vault key | HPKE | section 5.7 |
| Encrypted export | `export_key` | single-shot XChaCha20-Poly1305; the AAD binds the format id, the salt and the KDF parameters |
| Session recording | `recording_key[file]` | STREAM with 64 KiB chunks; the AAD of every chunk is the file header; the last-block flag detects truncation |

Rules:

- Every encryption uses a fresh 24-byte nonce from the OS CSPRNG. Nonces are never derived, counted or reused, except the counter inside STREAM.
- A record that fails the AEAD check, whose payload `type` belongs to another class than the envelope `class`, or whose `deleted` field disagrees with the tombstone flag is never applied; the local version stays and sync status shows an integrity warning ([sync-protocol.md](sync-protocol.md), "Applying pulled records").
- Padding hides sizes only up to 256-byte steps.

### 5.7 HPKE wrapping and member pins

(D14; parameters and crate: stage 0 decision)

| Parameter | Value |
|---|---|
| Standard | RFC 9180 |
| Mode | `mode_auth` (0x02): the sender authenticates with its static X25519 key |
| KEM | DHKEM(X25519, HKDF-SHA256), id 0x0020 |
| KDF | HKDF-SHA256, id 0x0001 |
| AEAD | ChaCha20Poly1305, id 0x0003 |
| Crate | `hpke` (rust-hpke, MIT OR Apache-2.0), only the X25519 KEM enabled; built on RustCrypto and dalek primitives. Check audit status and pin the exact version when it is added (stage 8). |
| `info` | `"shellpp/v1/hpke/vault-key"` (25 bytes) ‖ `vault_id` (16) ‖ `be32(epoch)` ‖ `recipient_user_id` (16) ‖ `sender_user_id` (16), 77 bytes |
| `aad` | empty |
| PSK | none |
| Plaintext | `vault_key[v, epoch]` (32 bytes) |
| Output | `enc` (32 bytes) and `ct` (48 bytes), stored as separate fields |

- Only the newest epoch is wrapped per member; older epochs come from the chain.
- The recipient opens a wrap only if the sender's public key matches a pin (`member_pin`) or, on first contact, creates the pin (TOFU); a device without an earlier key of the vault opens its first wrap without a fingerprint comparison only on a verified pin or on the pin made when this account accepted the sender's invitation (below). A changed key for a pinned user blocks wrapping and opening and shows a warning, like a host key change. Pins and fingerprints are described in [sync-protocol.md](sync-protocol.md), "Member keys, pinning and fingerprints".
- A team vault creator wraps epoch 1 to itself (sender = recipient).

Rosters (stage 0 decision): the member list comes from the server, which can add accounts it controls to it. So each client keeps, for every team vault, its own roster: `roster_entry` records in the personal vault, one per other user of that vault, with the user's status (`accepted`, `seen` or `removed`), last seen role and, for `accepted`, the epoch this account last wrapped to the user with `epoch_commit`, a 16-byte commitment to that epoch's key (section 5.5; [data-model.md](data-model.md), section 6.8). The keys stay in the pins. A user becomes `accepted` only through an invitation confirmation made on one of this account's devices or an explicit accept after a fingerprint comparison in the recipient dialog below; both mark the user's pin verified in the same local transaction. A user this client saw removed from the vault stays `removed`, even when the server lists it again, until such an accept or until the user accepts it as a sender in an epoch warning, which makes it `seen`; when two versions of one entry meet in a sync conflict, the one that says `removed` wins. Roster rules on the wire: [sync-protocol.md](sync-protocol.md), sections 6.7 and 7.1.

Roster entries are ordinary synced records, so an active server that keeps a second device of the account on a forked view can make that device write a newer revision that says `accepted` for a user another device removed, and deliver it as the newest version. On the fork the same epoch number can hold another key. So an entry counts as accepted for the current epoch only when its `epoch` equals the current epoch, its `epoch_commit` matches the key this device holds for that epoch, and the user's pin is verified. A revision from another fork makes the user "not confirmed for the current epoch"; a pin the server withheld, so that this device pinned the server's key on first sight, stays unverified. Either stops automatic rotation. (stage 0 decision)

Epoch acceptance (stage 0 decision):

- A client learns a new epoch only through its own HPKE wrap. Before it accepts epoch `e + k` (`k ≥ 1`) of a vault whose epoch `e` it holds, it fetches the member list and updates the roster.
- Sender: the wrap opens only with the sender's pinned key. The sender must also be this account, or a user whose roster entry for this vault has the role `owner` and is not `removed`. Wraps from a user this client saw removed from the vault are refused for that vault.
- Chain: the client opens `prev_key_wrap[v, e + k]` with `chain_key[v, e + k]`, continues down the chain to epoch `e` and compares the result with the key it holds (constant time). A mismatch, or a wrap that does not open, means the epoch was not made by a holder of the previous key.
- A refused epoch: the client keeps its keys for reading, stops writing to that vault and shows a warning in Team with the sender's email and locally computed fingerprint. A `removed` sender is trusted again only by an explicit accept. Its key did not change, so the warning asks whether the user was added back to the vault on purpose instead of asking for a fingerprint comparison, and the accept sets its roster entry to `seen`, not `accepted`: the user is again a valid sender but not a recipient of automatic rotations.
- These checks stop an active server from introducing a key it chose for existing members, and stop a removed member from planting an epoch on devices that hold the removed roster entry; the server can withhold that entry from the account's other devices or serve a new device an older revision, which puts them in the position described in RR21. They cannot stop a server that hides the removal from some members (RR21).
- A client that holds no earlier key of the vault (a new member, or a new device of a member) can rely only on the sender pin and the sender's `owner` role in the member list, see section 15. It opens its first wrap only when the sender is this account, or a user whose pin has `verified = true` or `origin = invitation_accept` (made when this account accepted an invitation whose dialog showed that user's fingerprint). Pins are synced and created unverified on first sight of any key, on any device and for any vault, so an account the server controls can already have one; such a pin, wherever it was made, does not count. Otherwise the client shows the epoch warning with a fingerprint comparison, and the wrap is opened only after an explicit accept that follows the comparison, which marks the pin verified. (stage 0 decision for the pin conditions)
- Team (UI 12.3) shows, for each team vault, the current epoch, the email and locally computed fingerprint of the user whose wrap this client opened, and the epoch's creation time ("epoch 4, key from alice@example.com, 3f2a 91c0 …, 2026-10-20 15:00"), so members can compare them out of band after a removal.

Rotation recipients (stage 0 decision; UI detail in stage 8):

- A rotation never pins a recipient on the spot. A pin made on first sight of a member list (TOFU, unverified) does not make its user a recipient; only the roster does, together with a verified pin. Every recipient's wrap uses its pinned key, which must match the key the server returns and have `verified = true`.
- Automatic rotation (`rotation_pending`, or a resumed rotation job) may only keep or shrink the recipient set: it runs only when every remaining member in the server's list is accepted for the current epoch (`accepted` in the roster with `epoch` equal to the vault's current epoch and a matching `epoch_commit`, and a verified pin), and then wraps to exactly those members. If the server names anyone who is not accepted for the current epoch, through the member list or `409 epoch_conflict`, the automatic rotation stops, Team shows "rotation waiting for confirmation" and the owner gets a notification. A withheld or rolled-back roster entry therefore only stops automatic rotation: an older revision that still says `accepted` holds an older epoch. A withheld pin only stops it too, because the pin this device then creates for the server's key is unverified, and so does a revision written on another fork, whose `epoch_commit` names another key.
- Interactive rotation (removal, explicit rotation, or a stopped automatic one): the confirmation lists every recipient with email, locally computed fingerprint and pin state. A recipient outside the `accepted` roster (new, `seen` or `removed`), or an `accepted` one whose pin is unverified, is marked and needs its own confirmation after an out-of-band fingerprint comparison; that confirmation marks its pin verified and its roster entry `accepted`. The owner can remove such a user in the same rotation instead. An `accepted` recipient with a verified pin whose `epoch` is older than the current epoch, or whose `epoch_commit` does not match the key this device holds, is marked "not confirmed for the current epoch".
- A retry after `409 epoch_conflict` whose recipient set grew is checked again under these rules: an automatic rotation stops unless every recipient is accepted for the current epoch, and an interactive one repeats the confirmation. A retry that only drops recipients proceeds.
- The rotating client marks the removed users `removed` in its roster, and sets `epoch` to the new epoch and `epoch_commit` to the commitment of the new key in the `accepted` entry of every recipient, in the same local transaction that stores the new epoch.

### 5.8 Human-readable secrets and fingerprints

- Recovery key: 32 bytes ‖ first 2 bytes of SHA-256(key) = 34 bytes → Crockford base32, 55 characters, shown as `RK1-` followed by 11 groups of 5 separated by `-`. Input ignores case, spaces and hyphens and maps `I`/`L` → `1`, `O` → `0`. (D11; format: stage 0 decision)
- Identity fingerprint: `SHA-256("shellpp/v1/identity-fp" ‖ x25519_public_key)`, first 16 bytes as lowercase hex in 8 groups of 4, for example `3f2a 91c0 …`. Always computed locally; fingerprints sent by the server are never shown. (D14; format: stage 0 decision)
- Invitation token and registration code: 16 random bytes → Crockford base32, 26 characters. (stage 0 decision)
- SSH key and host key fingerprints: SHA256, in the OpenSSH presentation (UI 7.3, 10.1).

### 5.9 Key events

Storage-level changes for each event are in [storage-format.md](storage-format.md), "What changes when"; API steps are in [sync-protocol.md](sync-protocol.md).

| Event | Key operations | Source |
|---|---|---|
| First run | password (≥ 8 characters) → new `db_id`, `device_id`, personal `vault_id`, `vault_key[personal, 1]`, `kdf_salt`, `recovery_key` → `kek` and `recovery` wraps → records `vault_meta`, `recovery_key`, `device_info`. The recovery key is not shown in local mode. Then, with "remember on this device" (on by default, UI 13.1) and an OS keychain: `device_key` into the OS keychain → `device` wrap | D8, D11, D29 |
| Unlock | Argon2id → `kek` → open `(personal, current, kek)` → older epochs through the chain → `user_keypair` (if any) → each team vault's HPKE wrap (sender pin checked) → in-memory index; `master_key` and `kek` wiped. With "remember on this device" at launch: `device_key` opens `(personal, current, device)` instead | D8, D9 |
| Wrong password | `unlock` fails; the attempt counter grows. From the fifth wrong password in a row, `unlock` is refused before Argon2id runs until the pause ends (section 8). No key operation | D29 |
| Lock | section 8 | D9 |
| "Remember on this device" on / off | on (at the first run by default, or later with a warning): `device_key` into the OS keychain → `device` wrap. Off: keychain item and wrap deleted. Unavailable without an OS keychain (D5) and in unsigned macOS builds (D7) | D5, D7, D8, D29 |
| Start with a new vault | only while locked: the database is closed, `shellpp.db` and its `-wal` and `-shm` files are renamed to an archive name ([data-format.md](data-format.md)), and a new empty database puts the app in first run. Nothing is deleted. A keychain item of the old `db_id` is kept and opens only the archive | D29 |
| Register | master password entered again (`master_key` is not kept) → `user_keypair` and `ssh_id` generated → keys re-derived with the account email in `argon2_salt` (existing `kdf_salt` and parameters) → new `kek` wrap and `auth_key` (bound to the server origin, section 5.5) → `auth_key`, `recovery_auth_key`, the new `kek_wrap`, the `recovery` wrap and public keys sent; `local_auth.kdf_email` set. Same password, no new recovery key. A personal vault that was bound to an earlier account is first moved to a new `vault_id` ([data-model.md](data-model.md), section 18.5), and in that case registration also generates a new `kdf_salt` and a new `recovery_key` (new `recovery_key` record, `recovery` wrap and `recovery_auth_key`; the key is shown with "I saved it" in step 4 of UI 13.5), so the new server never receives an auth key or a recovery auth key the earlier server accepts | D11, D13; email binding, move, new salt and recovery key: stage 0 decision |
| Sign in on an empty device | pre-login → Argon2id → login (→ TOTP) → open `kek_wrap` → local `kek` wrap → pull → local `recovery` wrap built from the `recovery_key` record → the server's `recovery_wrap` checked against it | D11; check: stage 0 decision |
| Sign in on a device with local data | as above; local records re-encrypted into the account's personal vault, recordings re-encrypted, then the local personal vault, its wraps and its recovery key are replaced by the account's. When the account's personal vault id equals the local one (the account this device was last bound to), it is a rebind instead: records and ids are kept and the vault runs a full resync. Records of an earlier account are never merged ([data-model.md](data-model.md), section 18.5) | D15; rebind and earlier account: stage 0 decision |
| Password change | new salt, current preset → new `kek` and `auth_key` → rewrap of the current personal vault key → with an account, the server replaces verifier, salt, parameters and `kek_wrap` atomically and signs out the other devices. Recovery and device wraps unchanged; the vault key does not change and no record is re-encrypted. With "my old password may be compromised" (off by default) the change also rotates the personal vault, as a recovery does | D11; option: stage 0 decision |
| Email change | current password re-entered → new salt and keys derived with the new email → rewrap, as a password change that keeps the password → the request names the email the material was derived for, and the server applies it only when that is the pending new email → the server replaces email, verifier, salt, parameters and `kek_wrap` atomically, deletes every ticket of the user and signs out the other devices; `local_auth.kdf_email` set to the new email | stage 0 decision |
| Recovery | `recovery_key` → `recovery_auth_key` (server) and `recovery_kek` → open the recovery wrap → personal vault rotation (`vault_key[personal, e + 1]`, chain wrap, `kek` and `recovery` wraps of the new epoch, background re-encryption) and new password material as for a password change, in one server transaction, with the Argon2id salt bound to the email accepted by `recover/begin` → all other devices revoked; `local_auth.kdf_email` and `server_account.email` set to that email. `recover/complete` names that email and fails if the account email changed in between. A server epoch below the local one, or a `recovery_wrap` that opens to another key than the one this device holds for that epoch, stops the recovery (rollback). Local mode: the app offers no recovery, because the key is never shown there (D29); the core operation (local `recovery` wrap, the same rotation locally) remains. A bound device whose server cannot be reached, refuses or rolled back can recover its local copy offline: local `recovery` wrap, new local password, no rotation, account signed out on that device until the next sign-in with the account password or a recovery through the server ([sync-protocol.md](sync-protocol.md), section 5.2) | D11; rotation, email, rollback check and offline recovery: stage 0 decision |
| Recovery key reissue | master password re-entered → new `recovery_key` → personal vault rotation as in a recovery, with the `recovery` wrap and verifier of the new key → new revision of the `recovery_key` record committed in the same server transaction → all other devices revoked; they rebuild their local `recovery` wrap after signing in again. The old key stops working, except with old database copies and with other devices' local copies until they sign in (RR6) | D11; rotation and single transaction: stage 0 decision |
| Team vault creation | `vault_key[team, 1]` from the CSPRNG → HPKE wrap to self | D14 |
| Invitation confirmation | owner compares the invitee's fingerprint out of band → pin `verified = true`, roster entry `accepted` with the current epoch and its `epoch_commit` → HPKE wrap of the current epoch | D14; roster: stage 0 decision |
| Member removal, leave, explicit rotation | `vault_key[v, e + 1]` → chain wrap → HPKE wraps for the remaining members under the recipient rules of section 5.7 (verified pins and roster checked) → removed users marked `removed`, recipients' roster entries set to epoch `e + 1` with its `epoch_commit` → background re-encryption → the dialog says that changing the host secrets is the only guaranteed remedy, and Team tracks them (section 15) | D14; recipient rules and tracking: stage 0 decision |
| Log out | refresh token family revoked and its wrap deleted → keep (default) or delete the local copy; team vaults become read-only archives that still open with the kept keys | D14, UI 12.2 |
| Account deletion | account deleted on the server; local copy kept; the personal vault becomes local-only | D14 |

## 6. What is protected against whom

"Yes": protected by design. "Partial": see the note. "No": not protected. "—": not applicable. The A3 column assumes no valid certificate for the sync server's name; with one, the A6 column applies to sync traffic.

| Asset or property | A1 Disk | A2 Malware | A3 Network | A4 SSH server | A5 Passive server | A6 Active server | A7 Member | A8 Removed member | A9 Lost device |
|---|---|---|---|---|---|---|---|---|---|
| Personal vault contents | Yes (a) | No (b) | Yes | Yes (c) | Yes (a) | Yes (a) | Yes | Yes | Partial (d) |
| Team vault contents | Yes (a) | No (b) | Yes | Yes (c) | Yes (a) | Partial (e) | No (f) | Partial (g) | Partial (d) |
| Master password and recovery key | Yes (a) | No (b) | Yes | Yes (h) | Yes (a) | Yes (a) | Yes | Yes | Yes (a) |
| Record integrity (forgery, substitution, reordering) | Yes (i) | No | Yes | — | Yes | Yes (i) | Partial (j) | Partial (k) | Yes (i) |
| Freshness and completeness | No (l) | No | Yes | — | Yes | No (m) | Partial (n) | Yes | No (l) |
| SSH credentials and sessions | Yes (a) | No (b) | Yes (o) | Partial (p) | Yes | Yes | Partial (f) | Partial (g) | Partial (d) |
| Local availability (principle 3) | No | No | Yes | Yes | Yes | Yes (q) | Partial (n) | Yes | No |
| Metadata | No (r) | No | Partial (s) | Partial (t) | No (r) | No (r) | No (f) | Partial (g) | No (r) |

Notes:

- (a) Only as strong as the master password: anyone with the database or the server data can run offline guesses at the cost of one Argon2id per guess (RR7); old database copies keep old wraps (RR6).
- (b) Same-user malware can capture the password, read memory while unlocked, and read the device key when "remember on this device" is on (RR8).
- (c) A host learns only what the user sends to it: the credentials used for it, typed input and uploaded files.
- (d) Protected while the app is locked and "remember on this device" is off (it is on by default, D29). If the app was unlocked, or "remember on this device" is on and the OS session is open, the finder has full access (section 8, RR8).
- (e) Confidential unless an active server substitutes keys during key distribution and nobody compares fingerprints (section 15, RR5), or works with a removed member (RR21). Adding an account the server controls as a recipient needs an owner's explicit confirmation (section 5.7).
- (f) Every member, viewers included, decrypts every record of the vault, including host passwords and keys, and sees the member list (D13). Members' own credential bindings stay in their personal vaults.
- (g) A removed member keeps everything received before removal (principle 3) and, with an honest server, cannot read epochs created after it. With an active server it can keep reading new records or plant an epoch (RR21); changing the host secrets is the only guaranteed remedy (section 15).
- (h) The app never sends the master password or the recovery key to a host and never asks for them inside a session tab (section 11.2).
- (i) Tampering fails the AEAD check; putting back an older valid version is a freshness problem (next row).
- (j) Editors may change records by role; viewers are stopped only by the server. With an active server, any key holder can forge records, because records are not signed (D13, RR4).
- (k) With an active server, a removed member can still inject records encrypted under the epochs it held (RR4).
- (l) An attacker can delete files or put back an older copy of the database; this is not detected.
- (m) The server can roll back, drop or withhold records and show different states to different devices; a forced full resync can bring back records deleted elsewhere, but never deletes local ones (RR1).
- (n) Editors can delete shared records for every member. Tombstones are kept 90 days and conflict copies keep losing versions, but there is no version history (RR3).
- (o) SSH transport encryption and host key verification; the first connection relies on TOFU (RR12).
- (p) The host sees its sessions and any password sent to it. Private keys never leave the client; a forwarded agent can be used during the session (RR19). Escape sequences and file names are filtered (sections 11.4 and 12).
- (q) Removal, log out, account deletion and `not_member` never wipe local data (principle 3). Only a received, authenticated tombstone deletes a synced record: records missing from a full or class-limited snapshot are kept and pushed again ([sync-protocol.md](sync-protocol.md), section 6.9), and recordings are deleted only when this device applies the tombstone of their log entry.
- (r) Plaintext metadata on disk (section 9.3) and on the server (section 14.2).
- (s) TLS and SSH hide content; addresses, timing and sizes of connections are visible.
- (t) A host sees the client's address and the times of its own sessions.

## 7. Mandatory requirements and their mitigations

| Requirement | Source | Mitigation | Verification |
|---|---|---|---|
| Secrets never in logs | SPEC "Security" | section 9.1 | unit test: canary secrets pass through the code paths, captured log output must not contain them (stage 0 decision) |
| Secrets never in plaintext on disk | SPEC "Security" | sections 5.6, 9.3; exceptions in 9.3 | storage test: search the database, WAL, recordings and logs for canary plaintext (stage 0 decision) |
| Secrets never in URLs | SPEC "Security" | section 9.2 | API types carry secret fields only in bodies; test that no request path or query contains a canary (stage 0 decision) |
| Master password from the first launch, also without an account; changeable in local mode | SPEC "Security", D8 | sections 5.3, 5.9 | e2e (UI 13.1, 13.4) |
| Auto-lock on inactivity | SPEC "Security", D9 | section 8 | e2e (UI 13.3) |
| Unlock pauses after 5 wrong passwords in a row, also across restarts | D29, UI 13.2 | section 8 | unit tests of the counter in `crates/core` |
| Clipboard cleared after copying a password or key | SPEC "Security", UI 7.3, 13.5 | section 9.4 | e2e with the clipboard mock (D6) |
| Strict CSP, minimal set of commands | SPEC "Security" | sections 13.1, 13.2 | CI check of the CSP string and snapshot test of the capability files (stage 0 decision) |
| No history during password prompts, control sequences stripped, no secret-like snippet hints | UI 4.2 | sections 10.1, 10.2 | unit tests in `crates/core` (SPEC "Quality": "secret filter in history") with a corpus of prompts and commands; the corpus includes snippet runs and inserts whose variable holds a low-entropy password the detector does not match, for example `htpasswd -b users alice {{pw}}` with `pw = summer2024`, which must not be recorded (stage 0 decision); terminal-level tests that feed the server's bytes, echo included, through a real terminal model and the core: `su`, a bracketed paste of a password, `sudo -S`, `passwd`, `ssh-keygen`, `mysql -p` and `mysql -p<value>`, secret-looking lines, the alternate screen, a leading space, injected input and recording off (`apps/desktop/src/terminal/history_capture/tests.rs`, stage 2b); the same through the session layer against the russh test server's shell and, when `SHELLPP_OPENSSH_SERVER_JSON` names a server of `tests/openssh/start.sh`, a real bash over OpenSSH in Docker (`su`, `passwd`, `ssh-keygen`, bash's bracketed paste, `read -s`; `apps/desktop/src/sessions/tests/history.rs`, ignored by default) |
| Private key hidden, shown only via an explicit button | UI 7.3 | section 9.5 | e2e |
| Port forwarding listens only on 127.0.0.1 by default | UI 8.4 | section 11.3 | unit test of defaults; integration test against OpenSSH in Docker |
| Host key change blocks until explicit confirmation | UI 10.2 | section 11.1 | integration test against OpenSSH in Docker (D6); e2e |
| Shell history import off by default | UI 12.4 | section 10.3 | default-settings unit test; e2e |
| Session output recording off by default | UI 12.6 (D16) | section 10.4 | default-settings unit test; e2e |

## 8. Lock and auto-lock

UI behavior is in UI 13.1–13.4; this section defines what happens underneath (D9, D29).

Triggers:

- The Lock item in the app menu and Ctrl+Shift+L (UI 1.4, 13.3).
- Inactivity: no keyboard or mouse input (keys, mouse buttons, the scroll wheel) in any app window, its dialogs included, for the auto-lock interval (5, 15, 30 or 60 minutes, or "Never", the default since D29; UI 13.4). The core still accepts a stored value of 1 minute, which the app no longer offers. Terminal output, transfers, sync and forwards do not count as activity. The timer lives in the Rust core and covers all windows at once (D23). The desktop app asks the core every second whether the lock is due and then locks through its windows, which save pending edits of the details panel first; the core's own tick locks as well, as a backstop.
- Sleep counts as inactivity: on resume, the core compares wall-clock time with the last input and locks at once if the interval has passed. Monotonic clocks stop during suspend on Linux and macOS, so they are not used for this check. (stage 0 decision)
- Launch: the app always starts locked; with "remember on this device" (on by default since D29) it unlocks at launch with the device key (UI 13.2). When the keychain item is missing or does not open the `device` wrap, the app asks for the password.

On lock:

| Item | Effect |
|---|---|
| Vault keys, `record_key`, `chain_key`, `local_secrets_key`, the X25519 private key | wiped |
| Decrypted in-memory index (all records, secret fields included) | dropped and wiped |
| Access token and decrypted refresh token | wiped; sync paused |
| Keys loaded into the built-in agent | wiped; agent requests are refused until unlock (stage 0 decision) |
| Frontend state | decrypted data dropped; only non-secret view state (open tabs, routes, selected ids) is kept, so windows return to the same place after unlock (UI 13.3) (stage 0 decision) |
| Windows | the unlock screen covers every window, including dialogs and terminal output (UI 13.2) |
| Pending sensitive clipboard content | cleared (section 9.4) |
| Established sessions, forwards, running transfers | keep running with their transport state (D9) |
| Terminal scrollback | stays in memory, hidden: open session tabs stay alive behind the unlock screen (UI 13.3, D29; RR11) |
| An open session recording | keeps its own `recording_key[file]` until the file is closed, so output keeps being recorded; this key opens only that file ([storage-format.md](storage-format.md)) (stage 0 decision) |
| Reconnects, forward autostart, new connections | wait for unlock (D9) |
| Proxying of the system agent for open sessions | continues; the system agent applies its own policy (stage 0 decision) |

Unlock:

- The master password (UI 13.2). The recovery key is not offered in local mode, because it is never shown there (D29); with a sync account, "Restore from sync" uses it (UI 13.2, stage 7). After lock, the password is required even with "remember on this device" (UI 13.3); the device key is used only at launch, so restarting the app bypasses that lock (RR8).
- Attempt limit (D29): wrong master passwords in a row are counted. From the fifth one, `unlock` is refused (`too_many_attempts`, with the seconds left) for a pause of 5 seconds that doubles with every further wrong password, up to 5 minutes: 5, 10, 20, 40, 80, 160, then 300 seconds. The core checks the pause before Argon2id runs, so no screen can skip it, and counts the attempt as wrong under the same lock, taking it back when the password proves right, so concurrent attempts (two windows, other callers) cannot guess in parallel. Any successful unlock (password, device key or recovery) resets the counter.
- The counter and the end of the pause are kept in plaintext in the `meta` table (key `unlock_lockout`, [data-format.md](data-format.md)), so a restart does not reset them; they are not secret. A pause end further away than the counter allows (the clock was set back, or the value was edited) is cut back to the allowed pause. Wrong passwords never wipe data (principle 3).
- The counter brakes guessing through the app's own screen only. An attacker with the database file (A1) guesses offline, where the Argon2id cost per attempt is the only brake (RR7), and same-user malware (A2) can edit the counter.
- The master password is asked only by the app's own lock screen and settings dialogs, never inside a session tab (section 11.2).
- Showing or reissuing the recovery key, changing the password and changing the account email require the master password again even while unlocked (UI 13.4; email: stage 0 decision, section 5.9).

"Remember on this device" (on by default since D29, the owner's choice):

- At the first run, when the toggle is on (the default) and the OS has a keychain, a random 32-byte `device_key` is stored in it (service `shellpp`, account `device-key:<db_id>`) and wraps the personal vault key in the `device` row ([data-format.md](data-format.md)). The app then unlocks at launch without the password.
- The risk: any process running as the same OS user can read the keychain item (Windows Credential Manager, an unlocked Secret Service collection) and, together with the database file, open the data without the password. Starting the app in the user's session opens it without the password as well (RR8). The first-run screen and Settings say this next to the toggle (UI 13.1, 13.4).
- What still holds: lock and auto-lock wipe the keys from memory and ask for the password; a disk-only attacker (A1) cannot read the keychain item; a copy of the database on another machine has no keychain item, except on Windows with a roaming profile (below); turning the option off deletes the item and then the `device` row, and when the OS refuses to delete the item nothing changes and Settings says so.
- Windows roaming profiles: the crate `keyring` (the owner's choice, D29) stores the item as a generic credential with `CRED_PERSIST_ENTERPRISE`. In a domain with Credential Roaming or roaming profiles, the item travels with the profile to every machine the user signs in to and to the profile server, so a copy of the database on such a machine opens without the password (RR8). Users in such domains should turn the option off.
- Tests, the demo mode and the screenshot mode use the in-memory secret store and a temporary `SHELLPP_DATA_DIR`, so they never write to the OS keychain or the permanent profile (D29).

Forgotten master password (D29):

- In local mode there is no recovery: the master password is not stored anywhere, and the recovery key that the core keeps in the personal vault is never shown (section 5.3).
- "Start with a new vault" (UI 13.2) runs only while locked. The core closes the database, renames `shellpp.db` and its `-wal` and `-shm` files to an archive name and opens a new, empty database, so the app is in first run again; it keeps the lock of the data directory all along, so no second process can step in meanwhile. Nothing is deleted: the archive stays encrypted under the old password and can be put back by hand ([data-format.md](data-format.md)). The keychain item of the old `db_id`, if any, is kept and opens only the archive; `backups/` is kept as well. The attempt counter starts from zero in the new database.

## 9. Secret hygiene

### 9.1 Logs

Never written to diagnostic logs (`shellpp.log`), panic messages or the server log:

- passwords, passphrases, PINs, OTP codes, private keys and any key material, auth keys, tokens, tickets, invitation tokens, registration codes, the recovery key, TOTP secrets;
- envelopes, payloads and decrypted record fields;
- host addresses, usernames, command text, terminal output and local or remote file names and paths ([architecture.md](architecture.md), section 3.8; stage 0 decision);
- on the server: emails, keys, tokens and envelopes; server logs carry user ids only ([sync-protocol.md](sync-protocol.md), "Transport").

Allowed: event names, error codes, record, vault and device ids, counts, durations, versions.

Enforcement (stage 0 decision):

- Secret values live in `secrecy` types whose `Debug` prints a placeholder; types that carry host data implement `Debug` without addresses or usernames; database rows that carry envelopes or key wraps print their lengths, not their bytes.
- Log calls use explicit structured fields; whole records, requests or responses are never formatted into a log line.
- The canary test of section 7.
- Logs stay on the device: no upload and no telemetry. "Report a Bug" opens the issue page without attaching logs or system data.

### 9.2 URLs

- API: secrets travel only in request bodies or headers. Paths carry only ids (`vault_id`, `invitation_id`, `user_id`, `device_id`), queries only `since`, `limit` and `classes` ([sync-protocol.md](sync-protocol.md), "Transport").
- `shellpp://host/<record_id>` carries only a record id.
- `shellpp://invite/<invitation_token>?server=<origin>` carries the invitation token. This is an accepted exception (D14): the token is single-use, valid 7 days, stored by the server only as a hash, revocable, and grants nothing by itself. Acceptance needs a signed-in account, and access needs the owner's fingerprint confirmation. The client sends the token only in request bodies and never logs it.
- The `otpauth://` URI from TOTP setup is shown only as a QR code, with the secret as text next to it, inside the app (UI 13.5); it is never passed to the OS, opened as a link or logged.
- External links opened by the app (Documentation, Report a Bug, Changelog, terminal links) never carry app data.
- The update check is a plain GET without identifiers; its User-Agent is `shellpp` without a version (D7).

### 9.3 Plaintext on disk

Plaintext in the data directory is limited to ids, class codes, revisions, epochs, tombstone flags, sequence numbers, timestamps of local bookkeeping, the account email and server URL, KDF parameters and salt, the unlock attempt counter (section 8), and `local_settings`, which never hold secrets, host data or command text ([storage-format.md](storage-format.md), "What is stored in the clear"; the implemented format is in [data-format.md](data-format.md)). Diagnostic logs follow section 9.1.

Explicit user actions that write plaintext (D16):

| Action | Written in plaintext | Safeguard |
|---|---|---|
| Plain JSON export (`shellpp.export.v1`) | every record of the exported vaults, secrets included | warning before saving; the encrypted export is offered next to it |
| Session log export to `.txt` (UI 11.2) | recorded output | warning that output can contain secrets |
| SFTP Open and Open with (UI 5.4) | the downloaded file in `tmp/` | warning; deleted when tracking ends, on exit and on the next start ([storage-format.md](storage-format.md), "Temporary files") |
| Private key export to a file (UI 7.3) | the key, in plaintext if it has no passphrase | the user is offered to set a passphrase (UI 7.3); without one, a warning (stage 0 decision) |
| `~/.ssh/config` and `known_hosts` export | host inventory, no secrets | none |

Rules:

- Unix permissions: directories `0700`, files `0600`. On Windows the data directory is in the local, non-roaming profile.
- SQLite runs with `secure_delete = ON`, which overwrites deleted content inside the database file. Files are deleted by unlinking; copies kept by the file system, SSD wear leveling, snapshots or OS backups are not erased (RR18).
- Pre-migration copies in `backups/` are ciphertext but keep the wraps of their time (RR6).
- "Start with a new vault" (section 8) renames the database files and never deletes them; the archive stays in the data directory until the user removes it.
- "Delete local copy" at log out removes the whole data directory, `backups/` and `recordings/` included, the temporary directory and the keychain item ([storage-format.md](storage-format.md), "Backup and restore").
- Full-disk encryption and encrypted swap are recommended in the user documentation, because memory can reach swap and hibernation files (RR11).

### 9.4 Clipboard

- Sensitive values: host and identity passwords, key passphrases, private keys (UI 7.3) and the recovery key (UI 13.5).
- They are copied by a Rust command straight to the OS clipboard and do not pass through the webview. (stage 0 decision)
- Cleared after 30 seconds (UI 7.3), only if the clipboard still holds the copied value, compared by a hash kept in memory, so a later copy by the user is not erased. Also cleared on lock and on exit. (stage 0 decision)
- Marked as sensitive where the platform has a convention, so clipboard history and cloud clipboard skip them: the Windows clipboard history and cloud exclusion formats, the macOS concealed pasteboard type, the KDE password-manager hint. (stage 0 decision; best effort)
- Not treated as sensitive: terminal copy (Ctrl+Shift+C, copy on select), public keys, fingerprints.
- Paste into the terminal: multi-line paste needs confirmation (SPEC "Terminal"); control characters are filtered (section 12).

### 9.5 Memory and the webview

- Rust holds secrets in `zeroize` / `secrecy` types and wipes them on lock.
- Intermediate buffers are wiped before they are freed as well: the Argon2id working memory (its last blocks give `master_key` without the password), the normalized copies of passwords and private keys, and the hash of a value copied to the clipboard (section 9.4). Buffers that grow are allocated once at their final size, because a reallocation leaves an unwiped copy behind. (stage 2 review, storage)
- Secret fields (passwords, passphrases, private keys, the recovery key) are not part of the data sent to the webview. The webview gets a "set / not set" flag, and for keys the type and SHA256 fingerprint (UI 7.3). The plaintext is sent only for an explicit reveal: the password show button (UI 3.1), the private key show button (UI 7.3), the recovery key view after password re-entry (UI 13.4). Leaving the panel drops it again (UI 7.3). (stage 0 decision)
- New secret values typed in the webview are sent to the core and are not kept in frontend state after saving. (stage 0 decision)
- JavaScript strings cannot be wiped; whatever was revealed stays in webview memory until it is garbage-collected (RR11).
- Known limitation (D83): some copies of secrets are made by libraries, out of the code's reach, and stay in freed memory until it is reused. They are:
  - the internal scratch buffer of serde_json while it parses a decrypted payload (strings with escapes, such as the line breaks of every PEM key);
  - the PPK passphrase and the decrypted PPK bytes inside ssh-key (0.7.0-rc.11 wipes neither);
  - the `String`s russh takes for passwords and keyboard-interactive answers;
  - text shown or typed in GPUI views and inputs (a revealed password, a typed passphrase).

  An allocator that wipes freed memory would cover them; it is not used (D83: it needs `unsafe impl GlobalAlloc`, an exception to CONTRIBUTING.md). Full-disk encryption and encrypted swap remain the recommendation (section 9.3).
- Core dumps are disabled for the app process: on Linux `PR_SET_DUMPABLE = 0` (which also blocks unprivileged same-user `ptrace` attach) and `RLIMIT_CORE = 0`; on macOS `RLIMIT_CORE = 0`. Windows Error Reporting dumps remain a residual risk (section 18). (stage 0 decision)
- DevTools are not available in release builds (section 13.2).

## 10. Command history and session recording

History is the class `history` in the personal vault, encrypted like everything else, never allowed in team vaults and not synced by default (D15, UI 4.2, UI 12.2). Commands that fail the rules below are never written, not even encrypted ([storage-format.md](storage-format.md), "Command history").

### 10.1 Capture rules

The client sees keystrokes and output, not commands, so capture is a heuristic with known limits (D16). OSC 133 is not used in the MVP. History is captured only in sessions to saved hosts, not in quick connects or local terminals (stage 2b decision). The terminal applies rules 1-4 (`apps/desktop/src/terminal/history_capture.rs`); the core applies rule 4 and the secret detector again before it writes (`crates/core/src/history`), so a candidate the terminal lets through can still be dropped, never the reverse.

The capture never keeps what the user typed. Typed input is only classified (a printable character, Enter, Ctrl+C, a bracketed paste marker, another key), and the command is read from the screen, so input the server does not echo never reaches it (stage 2b decision).

1. Prompt boundary: the cursor position in the normal screen buffer when the user types the first character after an Enter (or after the session starts). It is kept as a row offset inside the cursor's logical line (wrapped rows joined) and a column, so it follows the line when the screen scrolls or the shell redraws the line elsewhere (`Ctrl+L`) (stage 2b decision).
2. At Enter, the command is the text of the logical line from the boundary to the end of the line, with trailing whitespace removed. The text is read at Enter and again after every piece of output, until the cursor leaves the line: the shell echoes the Enter as a line feed, or a terminal renderer moves the cursor to another row. The last text read while the cursor was still on the line is the command. Waiting for that moment takes in the echo of keys typed just before Enter on a slow link. A line the cursor has not left after 5 seconds is not recorded (stage 2b decision).
3. Cleaning: C0 and C1 control characters, leftover escape sequences and bracketed paste markers (`ESC[200~`, `ESC[201~`, also in the caret form `^[[200~` a shell without bracketed paste echoes) are removed (UI 4.2). Pasted input reaches the session with its markers (that is the protocol); the capture keeps no typed or pasted text, so no marker is stored anywhere.
4. The command is not recorded if any of these holds:
   - the alternate screen was active when typing started or at Enter;
   - echo is off (see "Detecting input without echo" below);
   - the prompt looks like a password prompt: the text before the boundary on the same line, or the last non-empty line above it when that is empty, matches the pattern below; or the captured line itself contains a match of the in-line pattern below followed by `:` on the line, which is a prompt the program printed after the boundary (input typed before `sudo` showed `[sudo] password for ...:`, or before `su` showed `Password:`, which runs into the input: `hunter2Password:`) (stage 2b decision for the second form; it fails closed: `echo "password: x"` is not recorded either);
   - a line with nothing before its boundary on its own rows (no shell prompt there: input typed ahead, such as a password typed with its Enter before `su` turned echo off, which the terminal echoes on a row of its own) is held back after it ends, until the next key or for at most 5 seconds; it is dropped when the row the output is on looks like a password prompt meanwhile (the pattern below, without the `:` condition), or when the alternate screen comes on (stage 2b decision). A line with the shell's prompt before its boundary is not held;
   - a key was typed (or the app sent input) after the Enter but before the line ended on screen: the echo of that key can land on the line, so the line is dropped (stage 2b decision);
   - the boundary is not known: the line was started while the previous line was still waiting for its end (keys typed ahead on a slow link), after an Enter in the same write (the second and later lines of a multi-line paste), or it received a line break inside a bracketed paste (the shell keeps such a line open over several screen lines) (stage 2b decision);
   - keys typed in a full-screen program that has ended (`q` in a pager) do not start the next line: a line that started on the alternate screen is abandoned when the next key arrives on the normal screen (stage 2b decision);
   - `Ctrl+C` abandoned the line;
   - the command is empty, starts with a space (the shell convention for "do not keep") or is longer than 4096 characters (stage 0 decision);
   - the secret detector matches (section 10.2);
   - history recording is off for the host or globally (UI 4.2, D20). The terminal keeps a copy of the flag, read when the session starts and again whenever hosts, the settings or the lock change; while it is off, a captured line does not leave the terminal. The core checks the current values again at every command, so a running session follows a change at its next command, and a session whose saved host was removed records nothing more (stage 2b decision);
   - the line contains input injected by the app: any byte the app itself sent to the session between the prompt boundary and the Enter that ends the line, including an Enter the app sent. Injected input is a History insert (UI 4.2), a snippet insert or run (UI 4.2, 9.3), a run on several hosts (UI 9.4) and the startup snippet. The app sends such input only through the terminal's injected path (`TerminalHandle::inject_input`), never through the typed-input path, so the terminal marks the line, and a command line built from a snippet or a History entry is never recorded, with or without `{{ variable }}` values typed at run time. OS detection never writes to the session: it runs in a separate exec channel (section 11.4) (stage 0 decision; stage 2b for the injected path of the GPUI app).

Password prompt pattern, case-insensitive (stage 0 decision; the list lives in code with tests):

```text
\b(password|passwd|passphrase|passcode|pin|otp|one[- ]time|verification code|token|2fa|authenticator|secret)\b
(passwort|kennwort|mot de passe|contraseña|senha|parola|wachtwoord|hasło|heslo|пароль|密码|パスワード|비밀번호)
```

In-line pattern, for a prompt inside the captured line (stage 2b decision): the long words without the word boundaries, since typed input runs into the prompt; the short words keep them, so `ping:` or `tokens:` are not prompts.

```text
(password|passwd|passphrase|passcode|verification code|authenticator)
\b(pin|otp|one[- ]time|token|2fa|secret)\b
(passwort|kennwort|mot de passe|contraseña|senha|parola|wachtwoord|hasło|heslo|пароль|密码|パスワード|비밀번호)
```

**Detecting input without echo** (stage 2b decision). A terminal client cannot read the server's `ECHO` flag: SSH carries the terminal modes only once, in the PTY request, and the server's `tcsetattr` calls (`getpass`, `su`, `sudo`, `passwd`, `ssh-keygen`, `mysql -p`) change the remote terminal without telling the client. Echo is therefore inferred from the screen, by comparing what was typed with what appeared:

- the capture counts whether at least one printable character was typed after the boundary (keys, a paste or IME text; control keys, arrows and other escape sequences do not count, and the characters themselves are not kept);
- when the line ends, it reads the screen from the boundary to the end of the logical line;
- echo is taken to be off when printable characters were typed but that text is empty or consists only of mask characters (`*`, `•`, `●`).

A password typed at `su`'s `Password:` prompt leaves the line empty (the program prints only the line feed after reading it), so it is dropped by this rule, and also by the password prompt rule. A password the terminal does echo (`sudo -S` reads standard input with echo on) is dropped by the password prompt rule, since `[sudo] password for ...:` stands before the boundary.

Limits of this method:

- It looks at the screen, not at the terminal modes. A program that reads input with echo off and then prints other text on the same line before the cursor leaves it (for example the next shell prompt after `read -s`, which prints no line feed) is not detected as echo-off; the first key of the next command or the 5-second limit of rule 2 drops such a line, and since the hidden input never reached the screen, it cannot be recorded in any case.
- A program that echoes something other than the input or mask characters (random characters, a fixed word, a moving cursor) is not detected; only the prompt rule and the secret detector apply.
- A program that hides input without a prompt the pattern knows (a prompt in a language or wording that is not listed, or no prompt at all) and echoes the input is not recognized: what it shows is recorded unless the secret detector matches.
- When keys are typed while the previous line is still waiting for its line feed (a fast typist on a slow link, a pasted line break), neither line is recorded: the previous line could take in their echo, and their boundary is unknown. Nothing typed this way can leak, but commands are lost (fail closed).
- A password typed ahead with its Enter, before the program turned echo off, is caught only when its password prompt appears before the next key and within 5 seconds (the held-line rule): a prompt that comes later, or after the user typed on, is not seen, and only the secret detector applies to that line. A shell with an empty prompt has every command held this way; such a command is recorded at the next key, or dropped when the output after it holds one of the pattern's words.
- Synchronized output updates (mode 2026) are applied at once at their end, so the line can be read slightly late; at worst a few trailing characters are missed or the line is dropped.

Other known limits (documented for users):

- Tab completion, history recall and `Ctrl+R` are captured as the shell displays the final line; a redrawn or multi-line prompt, a right-side prompt or input typed before the prompt appears can shift the boundary and capture the wrong text (a prompt printed after the boundary is caught by the second form of the password prompt rule only when it is a password prompt).
- Answers to program questions that are not password prompts (the file name `ssh-keygen` asks for) are recorded like commands.
- A multi-line command (an open quote, a `\` continuation) is recorded line by line, as the shell shows each line.
- Commands typed in a nested `ssh` session are recorded under the outer host.
- The rules fail closed: some harmless commands are not recorded.

### 10.2 Secret detector

Applied to every captured command before it is stored, to every line of an imported shell history, and again before a command is offered in the "save as snippets" hint (UI 4.2). A match means the command is dropped, not redacted. Rules S1–S9 are case-insensitive unless stated.

| ID | Rule (D16) | Match | Example |
|---|---|---|---|
| S1 | `-p<value>` | a token matching `^-p\S+$`, case-sensitive; also matches combined flags such as `-pv` (fail closed) | `mysql -uroot -phunter2` |
| S2 | `--password` | substring `--password` | `--password=x` |
| S3 | `*PASSWORD=` | `[a-z0-9_]*password=` | `PGPASSWORD=…` |
| S4 | `TOKEN=` | `[a-z0-9_]*token=` | `GITHUB_TOKEN=…` |
| S5 | `SECRET` | substring `secret` | `AWS_SECRET_ACCESS_KEY=…` |
| S6 | `API_KEY` | `api[_-]?key` | `--api-key …` |
| S7 | `user:pass@` | `[^\s:@/]+:[^\s@/]+@` | `https://bob:pw@example.com/` |
| S8 | `Authorization:` | substring `authorization:` | `curl -H 'Authorization: …'` |
| S9 | `Bearer` | `\bbearer\b` | `-H "Bearer …"` |
| S10 | known token prefixes | the case-sensitive patterns below | `ghp_…`, `AKIA…` |
| S11 | high-entropy strings of 20 or more characters | the algorithm below | `q7Xv2LmR9tKp4WzN8sBe` |

S10 patterns (stage 0 decision; public token formats, extended as needed):

```text
\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b
\bgithub_pat_[A-Za-z0-9_]{22,}
\bglpat-[A-Za-z0-9_-]{20,}
\bxox[abprs]-[A-Za-z0-9-]{10,}
\b(AKIA|ASIA)[A-Z0-9]{16}\b
\bAIza[A-Za-z0-9_-]{35}
\b(sk|rk)_live_[A-Za-z0-9]{16,}
\bnpm_[A-Za-z0-9]{36}\b
\bpypi-[A-Za-z0-9_-]{50,}
-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----
\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.
\bshellpp_(at|rt|lt|rc)_[A-Za-z0-9_-]{20,}
\bRK1-[0-9A-Za-z]{5}-
```

S11 algorithm (stage 0 decision; thresholds are tuned in stage 2 against the test corpus, and changes are recorded here):

1. Split the cleaned command into tokens on whitespace, quotes and the characters `= : , ; @`.
2. A token is a candidate if it is at least 20 characters long, consists only of `[A-Za-z0-9+/_-]`, and contains no `/` unless it also contains `+` (paths are not candidates; base64 is).
3. The command matches if a candidate:
   - contains lowercase letters, uppercase letters and digits, and its Shannon entropy over its own characters is at least 3.5 bits per character; or
   - consists only of hex digits, is at least 32 characters long and contains both digits and letters; or
   - is at least 32 characters long with an entropy of at least 4.2 bits per character.
4. Hashes such as commit ids and image digests match by design (fail closed).

### 10.3 Shell history import

- Off by default (UI 12.4). When on, `~/.bash_history` and `~/.zsh_history` are read from the server in a separate channel that is not recorded, like OS detection. (stage 0 decision for the channel)
- The content is untrusted text: the zsh extended format prefix is parsed, control characters are removed, lines longer than 4096 characters are skipped, and the secret detector (section 10.2) skips matching lines (D16).

### 10.4 Session recording

- Off by default: a global toggle in UI 12.6 with a warning that output can contain secrets, and a per-host override "Record output" (UI 3.1, D20).
- Only output is recorded (asciicast v2 output events); input typed with echo off does not appear in it, but echoed input and printed secrets do.
- Encrypted with STREAM under `recording_key[file]`, never synced (D16); layout and lifecycle in [storage-format.md](storage-format.md), "Session recordings".
- Export to `.txt` is an explicit action with a warning (section 9.3).

## 11. SSH connections

### 11.1 Host keys

- Connections use only the app's own store: `known_host` records in the personal vault, synced by default (D15, D21). The system `~/.ssh/known_hosts` is read only by import.
- The host key is checked during key exchange, before any authentication data is sent: no password, keyboard-interactive answer, signature or agent request goes out before the key is accepted. (stage 0 decision)
- First connection (TOFU): dialog with address, key type and SHA256 fingerprint; "Accept and save" (default), "Connect once", "Cancel" (UI 10.2).
- Changed key: the connection is blocked; a danger-colored warning shows the old and new fingerprints and explains a possible attack; the only way on is "Replace saved key"; there is no "don't ask again" (UI 10.2).
- Revoked key (marker `revoked`, imported from `@revoked`, D21): the connection is blocked with a danger-colored warning and no button to continue (UI 10.2); the entry must first be removed on the Known Hosts screen. (stage 0 decision for the removal step)
- Hashed entries are kept as imported and matched with the OpenSSH HMAC-SHA1 scheme (D21). The hashing only hides names in the file; it is not a security boundary.
- Every jump host in a chain is checked with the same rules. (stage 0 decision)
- `@cert-authority` lines are skipped on import (D21); host certificates are not trusted in the MVP.
- Because known hosts sync, a key accepted on one device is trusted on the user's other devices. The sync server cannot add or change entries, because they are encrypted records, and cannot remove them, because records missing from a snapshot are never deleted ([sync-protocol.md](sync-protocol.md), section 6.9). So a forced resync can neither turn a "changed key" block into a first-use dialog nor re-enable a revoked key.

### 11.2 Authentication, credentials and agents

- Password, passphrase, username and keyboard-interactive prompts are a form inside the tab; "save" is off by default; keyboard-interactive answers are masked according to their echo flag (D18).
- Text from the server (banner, keyboard-interactive name, instruction and prompts) is shown as plain text in a frame labeled with the host, visually distinct from the app's own dialogs. (stage 0 decision)
- The master password and the recovery key are never asked for inside a session tab, a connection screen or any form that shows server text; only the lock screen (UI 13.2) and the Settings and account dialogs (UI 13.4, 13.5) ask for them (section 8). (stage 0 decision)
- Private keys never leave the client; only signatures are sent. A key passphrase is either saved in the vault or asked on every connection (UI 7.3).
- `rsa` crate: RUSTSEC-2023-0071 is a timing side channel; signing happens locally, without a remote oracle, so the risk is low (D4).
- Agent forwarding is off by default per host (UI 3.1). When it is on, the host can ask the forwarded agent for signatures while the session lasts but cannot extract keys (RR19).
- The built-in agent works only inside the app: no socket or named pipe is exposed to other processes (D5). It refuses requests while locked (section 8).
- Legacy SHA-1 algorithms (`ssh-rsa` signatures, SHA-1 key exchange and MACs) and CBC ciphers are disabled by default; the host setting "Allow legacy algorithms" (`ssh.allow_legacy_algorithms`, with a warning while it is on) offers them after all modern ones (D80). 3DES, RC4, Blowfish, `ssh-dss`, `diffie-hellman-group1-sha1` and MD5 MACs are never offered. (stage 0 decision; opt-in: D80)

### 11.3 Port forwarding

| Rule type | Default bind address | Other addresses |
|---|---|---|
| Local | 127.0.0.1 (UI 8.3, 8.4) | explicit choice with a warning (UI 8.4) |
| Dynamic (SOCKS5) | 127.0.0.1:1080 (UI 8.3) | explicit choice with a warning (UI 8.4) |
| Remote | 127.0.0.1 on the server (UI 8.3) | 0.0.0.0 shows a warning and a `GatewayPorts` hint (UI 8.3) |

- In the core, an empty bind address means 127.0.0.1, never all interfaces. `0.0.0.0`, `::` and every non-loopback address count as "all interfaces" for the warning. (stage 0 decision)
- Rules that start on app launch wait for unlock (D9).
- Loopback listeners are reachable by every local process and user, and the SOCKS proxy has no authentication (RR14).

### 11.4 Data sent by SSH servers

- Channel open requests from the server: `forwarded-tcpip` is accepted only for an active remote rule with a matching address and port; `auth-agent@openssh.com` only when agent forwarding is on for that session; `x11` and every other type are rejected. (stage 0 decision)
- Remote file names are untrusted. Before a downloaded file is written, names that contain `/`, `\`, NUL or other control characters, names `.` and `..`, and absolute paths are rejected; on Windows, names the file system does not allow are handled as in [storage-format.md](storage-format.md), "Temporary files". Recursive transfers do not follow symbolic links; links are skipped and reported. Names are displayed with control and bidirectional override characters made visible. (stage 0 decision)
- "Open" on a file type the OS would execute (executables, scripts, shortcuts, installers, for example `.exe`, `.bat`, `.cmd`, `.ps1`, `.lnk`, `.msi`, `.app`, `.command`, `.sh`, `.desktop`, `.AppImage`) asks for confirmation naming the type. Where the platform supports it, temporary files get its downloaded-file mark (Windows Mark of the Web, macOS quarantine attribute), so OS checks apply. (stage 0 decision)
- OS detection runs in a separate exec channel and is not recorded (UI 12.4); its output is mapped to a fixed set of OS identifiers, anything else shows the generic icon. (stage 0 decision for the mapping)
- Channel data that arrives before the server answers the requests that set up a channel (pty, agent forwarding, shell, exec) is kept only up to 1 MiB (and 4096 messages); beyond that the channel is closed and the attempt fails as a protocol error. Real servers send nothing there, and nothing reads it yet, so without the cap a server could fill memory before the terminal's flow control starts. (stage 2a review)
- Terminal output is handled by section 12; imported history by section 10.3.

## 12. Terminal escape sequences

Terminal output from any host is untrusted. xterm.js renders it in its own renderer, never as HTML.

| Feature | Risk | Policy |
|---|---|---|
| OSC 52 (clipboard) | a host silently replaces the clipboard or reads it | not supported in the MVP: the clipboard addon is not loaded and clipboard queries are never answered (stage 0 decision) |
| OSC 8 hyperlinks | the link text differs from the target; dangerous schemes | custom link handler: opens only on Ctrl+click (Cmd+click on macOS), after a confirmation that shows the full target; only `http:` and `https:`; opened in the system browser, never in an app webview (stage 0 decision) |
| Plain-text URLs (web-links addon) | opening an attacker's URL | the same modifier click and scheme rule; no confirmation, because the visible text is the target (stage 0 decision) |
| Window and icon title (OSC 0, 1, 2) | spoofed or injected text | tabs show the host name (UI 1.1); a remote title, where shown, is plain text without control characters and limited to 256 characters (stage 0 decision) |
| Reports and window operations (title report, window manipulation) | the reply is injected as input | xterm.js `windowOptions` stay disabled; title reporting is never enabled (stage 0 decision) |
| Bracketed paste | pasted text containing `ESC[201~` ends paste mode early and runs the rest | `ESC[200~`, `ESC[201~`, ESC and other control characters except tab, CR and LF are removed from pasted text and inserted snippets; multi-line paste needs confirmation (SPEC "Terminal") (stage 0 decision for the filter) |
| Images (Sixel and similar) | parser attack surface | not supported: no image addon |
| Output that becomes emulator state (an OSC string without an end, endless combining marks on one cell) | memory grows at link speed until the app runs out of it; every frame gets slower | the queue in front of the parser is bounded (flow control); an OSC string keeps at most 4 KiB of payload and the rest is dropped up to its terminator; a cell keeps at most 16 zero-width characters (`apps/desktop/src/terminal/limits.rs`) (stage 2a review) |
| History capture | escape codes stored as commands | removed (section 10.1, UI 4.2) |
| Keyword highlighting | slow user-defined regular expressions | display only and off in the alternate screen (UI 12.4); rules come only from the user's own vault |

## 13. Desktop app hardening

### 13.1 Content Security Policy

Set in `tauri.conf.json` (`app.security.csp`); the development server gets a separate `devCsp` that adds only the local Vite origin and the development-only `style-src` exception below. This is the normative release CSP; [architecture.md](architecture.md) (section 3.7) refers to it. (stage 0 decision)

```text
default-src 'none';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data:;
font-src 'self';
connect-src ipc: http://ipc.localhost;
object-src 'none';
base-uri 'none';
form-action 'none';
frame-src 'none';
frame-ancestors 'none'
```

Rules (stage 0 decision):

- Never allowed: `'unsafe-eval'`, `'unsafe-inline'` in `script-src`, remote origins, wildcards. Fonts and icons are bundled (Inter, JetBrains Mono, Lucide).
- If xterm.js or another dependency needs inline `<style>` elements, `'unsafe-inline'` may be added to `style-src` only, after review, and the reason is recorded here.
- Recorded exception, development builds only (`tauri dev`, stage 1 review): `devCsp` adds `'unsafe-inline'` to `style-src`, because the Vite dev server injects the CSS modules as `<style>` elements and replaces them on hot reload.
- Recorded exception, release builds (stage 2 review, terminal): `style-src` allows `'unsafe-inline'`. xterm.js 6 creates `<style>` elements at run time and writes their text: the viewport's scrollbar colors (always, for every renderer) and the cell dimensions and theme colors of its DOM renderer, which is also the fallback when WebGL is unavailable or its context is lost. xterm.js has no nonce option, and Tauri's nonce and hash injection covers only the `<style>` and `<script>` elements of the bundled HTML, not elements created later. The allowance covers styles only: `script-src` stays `'self'` (no inline scripts, no `eval`), and the strings written into those elements come from xterm.js's own options (theme colors, font family, sizes), not from terminal output. `apps/desktop/src-tauri/tests/config.rs` checks that `'unsafe-inline'` appears in `style-src` and nowhere else.
- Tauri settings: `withGlobalTauri: false`, the asset protocol disabled, Tauri's own nonce and hash injection left on.
- `Object.prototype` is frozen in every window before the IPC backend starts and before any IPC data or terminal output is processed. Recorded change (stage 2 integration, terminal): the frontend entry (`apps/desktop/src/main.tsx`) loads the xterm.js engine and then calls `Object.freeze(Object.prototype)`, and Tauri's `freezePrototype` is `false`. Tauri's setting freezes the prototype before any script runs, but xterm.js 6 assigns `toString` to a plain object while its module initializes (a namespace of the keyboard code it bundles), and in strict mode that assignment throws once `Object.prototype` is frozen, so the terminal could not load. Only the app's own bundled modules run before the freeze. `apps/desktop/src-tauri/tests/config.rs` checks the setting and that the entry freezes the prototype before it starts the backend.
- Untrusted strings (host names, file names, server messages, prompts, emails) are rendered as text. `dangerouslySetInnerHTML` and `innerHTML` are not used; eslint reports them as errors (`no-restricted-syntax` rules in `apps/desktop/eslint.config.js`, equivalent to `react/no-danger`, also covering `innerHTML`, `outerHTML` and `insertAdjacentHTML`).

### 13.2 Capabilities and commands

(stage 0 decision)

- One capability for the app's local windows (all windows are equal, D23) on desktop platforms. No remote origin gets IPC access.
- Every Rust command is granted by name through the Tauri app manifest in `build.rs` (generated `allow-<command>` permissions); there are no wildcard grants.
- Plugin permissions granted to the webview:

| Permission | Why |
|---|---|
| `core:event:allow-listen`, `core:event:allow-unlisten` | events from the core to the windows (D23); the webview does not emit events |
| `core:window:allow-minimize`, `core:window:allow-toggle-maximize`, `core:window:allow-is-maximized`, `core:window:allow-close`, `core:window:allow-start-dragging`, `core:window:allow-set-fullscreen` | frameless title bar (UI 1.1), full screen (UI 1.4) |

- `core:default` is not granted ([architecture.md](architecture.md), section 3.4).
- Not granted to the webview: `dialog`, `clipboard-manager`, `shell`, `fs`, `http`, `opener`, `process`, `updater`, `deep-link`, `notification`, `global-shortcut`, window and webview creation. File dialogs, clipboard access (terminal copy and paste, sensitive copies and clearing, section 9.4), opening URLs and files ("Open", "Open with"), new windows, updates and deep links are app commands that validate their arguments in Rust. File dialogs, the clipboard and external programs go through the platform traits of [architecture.md](architecture.md), section 5.1, so the e2e mocks apply to them (D6).
- Commands take record ids rather than raw data where possible. Local paths come from file dialogs or from the user's navigation in the SFTP local panel. The local shell path (UI 12.4) is a user setting; the CSP and text-only rendering are what keep untrusted data from changing it.
- Release builds have no DevTools and no e2e mock feature (D6); CI fails a release build that enables either.

### 13.3 Navigation and deep links

(stage 0 decision)

- The webview can navigate only within the app's own origin; other navigations and `window.open` are blocked.
- External URLs open in the system browser through an app command that accepts only `https:` (and `http:` for terminal links), and only after a user action.
- `shellpp://host/<record_id>`: brings the app to the front and selects the host after unlock. It never connects or runs anything by itself. An unknown id shows "missing".
- `shellpp://invite/<invitation_token>?server=<origin>`: opens the invitation dialog only. The origin must be `https` (or `http` on a loopback address) and is shown. If it is not the bound server, the dialog says so and nothing is sent until the user connects to that server in the wizard (UI 13.5); the client never sends its tokens to a server other than the bound one. Acceptance is an explicit click.
- Any other or malformed `shellpp://` link is ignored.

### 13.4 Updates

- tauri-plugin-updater with the project's own signing key; the public key is built into the app, and packages without a valid signature are rejected (D7).
- The automatic check is off by default; "Check for updates" in Help is always available; the request is a plain GET of the manifest without identifiers (D7).
- A custom update source (Settings → Updates) must be `https` and does not change the signature key. (stage 0 decision)
- The updater private key is never in the repository; it is held by the owner or as a CI secret. (stage 0 decision)

### 13.5 Untrusted files

(stage 0 decision except where cited)

- Every import parser treats its input as untrusted and fails per item, not per file.
- ssh config: `Match exec`, `ProxyCommand`, `LocalCommand`, `PermitLocalCommand` and `KnownHostsCommand` are never executed or stored as commands; they are skipped and counted in the import summary. `Include` follows only local files, at most 16 levels deep, with cycle detection.
- Keys are parsed with `ssh-key` and RustCrypto crates and imported with a passphrase prompt (D21).
- Encrypted JSON import enforces the KDF bounds (section 5.4); the decrypted content is validated against the export schema before anything is written.
- Terminal themes accept only the fields of the theme format (D25, [design-system.md](design-system.md), section 7.2): colors are `#RRGGBB`, only `selection` may also be `#RRGGBBAA`, and a file with an unknown field or another `version` is rejected.
- Import never overwrites existing records (D21).

## 14. Sync server and accounts

### 14.1 Authentication

- Only `auth_key` reaches the server; it stores an Argon2id verifier of it. The master password, `master_key` and `kek` never leave the client (D11). `auth_key` and `recovery_auth_key` are bound to the server origin (section 5.5), and a registration that moves data bound to another account generates a new salt and recovery key (section 5.9), so no server receives values that sign in to or recover an account on another server.
- TOTP (RFC 6238) is a second login step; a used time step is rejected again; the secret is encrypted at rest with `totp_at_rest_key`; TOTP is reset only with the admin CLI (D11). TOTP protects access to the account, not the encryption of the data.
- Sensitive operations repeat the proof of password as `current_auth_key`.
- Tokens: 32 random bytes with a `shellpp_*` prefix; access tokens live 15 minutes in memory; refresh tokens 30 days, rotated on every use, reuse revokes the family; the server stores only hashes.
- A password change, an email change, a recovery and a recovery key reissue sign out the other devices. Revoking a device stops its sync; there is no remote wipe (principle 3).
- Account enumeration is made harder by the fake pre-login salt, dummy Argon2id checks for unknown emails and one generic `invalid_credentials` code; it is not fully prevented (RR15).
- Registration is invite-only by default (`SHELLPP_REGISTRATION=invite`). Rate limits are in [sync-protocol.md](sync-protocol.md), "Rate limits".
- Transport: HTTPS only, plain `http://` only for loopback; rustls with the system roots; custom CA certificates are not in the MVP, so a self-hosted server needs a publicly trusted certificate (for example from the reverse proxy).

### 14.2 Server-visible metadata

The complete list (D10). Everything not listed is never sent to the server.

- Account: email and its verification state; KDF parameters and salt; Argon2id verifiers of `auth_key` and `recovery_auth_key`; `kek_wrap` and `recovery_wrap` (opaque); X25519 public key; SSH ID public key; TOTP secret (encrypted at rest with a key derived from `SHELLPP_SERVER_SECRET`) and TOTP state; creation time and time of the last credential change.
- Devices: `device_id`, creation time, last-seen day, token metadata. Device names are encrypted records.
- Vaults: `vault_id` (UUIDv7, so creation time), kind, members and roles, who added whom and when, epoch numbers with creator and time, opaque wraps, `rotation_pending`.
- Invitations: inviter, role, optional invitee email, invitee account and public key, timestamps, status.
- Records: `vault_id`, `record_id` (UUIDv7, so creation time), class (sync category), revision, key epoch, tombstone flag and its time, padded size, `seq`, time received, and the user and device of the creating and of the latest write.
- Derived: number of records per class and vault, change frequency and timing (activity pattern), which devices sync when.
- Transport: client IP addresses and request timing are seen during requests and used in memory for rate limiting only; reverse-proxy logs are the operator's responsibility.
- Never sent: master password, `master_key`, `kek`, `recovery_key`, `recovery_kek`, any vault key, record plaintext (names, addresses, usernames, passwords, keys, notes, tags, scripts, history, log contents, group structure, vault names), exact record types, HLC values, X25519 and SSH ID private keys, `device_key`.

### 14.3 What the client does not trust

- KDF parameters outside the floor and ceiling of section 5.4 are refused; the Argon2id salt is bound to the email the user typed, not to anything the server says.
- Envelopes are checked as in section 5.6; a received revision at or below the local synced revision is ignored, and a record whose id belongs to another local vault is rejected ([sync-protocol.md](sync-protocol.md), "Applying pulled records").
- The absence of a record is never taken as its deletion ([sync-protocol.md](sync-protocol.md), section 6.9).
- Public keys are used only through pins; new epochs pass the checks of section 5.7; rotation recipients come from the client's roster, with verified pins and a commitment to the current epoch's key, never only from the server's member list.
- The server's `recovery_wrap` is never stored unchecked: each device builds its local `recovery` wrap from the `recovery_key` record (section 5.9).
- The personal vault epoch never goes back: a server that reports a lower personal epoch, or a wrap of an epoch the device holds that opens to another key, changes no local key row and stops the personal vault's sync with "server state rolled back" ([sync-protocol.md](sync-protocol.md), section 5.1). (stage 0 decision)
- Server text (error messages, emails) is shown as plain text. For known error codes the client shows its own message instead of the server's `message`, so a server cannot place instructions in the app's voice. (stage 0 decision)

### 14.4 Operator guidance

(stage 0 decision; for the server documentation)

- Keep `SHELLPP_SERVER_SECRET` secret and stable. Changing it invalidates the encrypted TOTP secrets and changes the fake pre-login salts, which lets anyone who compared answers before and after tell unregistered emails apart.
- Keep the public origin (`SHELLPP_PUBLIC_URL`) stable and serve the API under that one origin. Clients bind `auth_key` and `recovery_auth_key` to the origin they connect to (section 5.5), so an account cannot sign in or recover under another host name or port; moving a server to another origin is not supported in the MVP. Local data is never affected.
- A database backup holds verifiers, wraps, salts and all metadata; whoever gets it is a passive server (A5) and can guess passwords offline. Encrypt backups and restrict access.
- Keep the default `SHELLPP_BIND` of `127.0.0.1:8080` behind the reverse proxy, and set `SHELLPP_TRUSTED_PROXIES` to the proxy only; otherwise forged `X-Forwarded-For` headers defeat per-IP rate limits.
- Reverse-proxy logs contain client addresses and request paths (paths carry only ids).

## 15. Team vaults

Protocol details are in [sync-protocol.md](sync-protocol.md), "Team vaults".

- The unit of sharing is the vault (D13). Every member, viewers included, decrypts every record of the vault; the invitation dialog says so (D13). The server enforces roles on writes; viewers are not technically prevented from reading anything.
- Credential bindings and the SSH ID stay in each member's personal vault, so a member who connects with their own key exposes nothing of theirs to the team (D13).
- Invitations take two steps (D14): the link carries only a one-time token (section 9.2); the owner compares the invitee's fingerprint out of band before the owner's client wraps the key and marks the pin verified.
- The invitee's client pins the owner's key on first use, marked as made at the invitation accept. An active server present at that moment can give a new member a fake vault under a key the server chose; the defense is to compare the owner's fingerprint in Team (UI 12.3) as well (RR5). A new device of an existing member opens its first wrap without a fingerprint comparison only from such a pin or a verified one (section 5.7).
- Rotation: section 5.7 (rosters, epoch acceptance, rotation recipients). Removal always creates a new epoch and the owner's client re-encrypts the vault in the background (D14).
- The removed member has seen every secret of the vault and, with an active server, may keep reading new records (RR21). So the removal dialog says plainly that changing the passwords and keys of the vault's hosts is the only guaranteed remedy (D14 asks the dialog to suggest it). Until those secrets change, the removed member can still log in to the hosts. (stage 0 decision for the wording and the tracking below)
- After a removal, Team (UI 12.3) shows owners and editors a persistent "Secrets to change" list for that vault: its hosts, identities and keys that hold a password, a private key or a passphrase and whose records have not been modified since the removal (modification time from the HLC, [data-model.md](data-model.md), section 3.2; background re-encryption keeps the HLC and does not count). The list is derived, so it shrinks as the secrets are edited on any device; the removal time is the time of the `removed` roster entry, and hiding the list sets `checklist_hidden` on that entry.
- Editors can change any field of a shared host, including the address, jump hosts, proxy, environment variables, startup snippet and agent forwarding. A member who connects to such a host uses their own credentials with these settings, so an editor can make other members run commands or send credentials to a host of the editor's choice. A changed address that leads to an unknown host triggers the TOFU dialog (section 11.1). Give editor rights only to people trusted at that level (RR3).
- A removed member keeps a read-only archive (principle 3, D14).

## 16. Supply chain

| Control | Rule | Source |
|---|---|---|
| Lockfiles | `Cargo.lock` and `pnpm-lock.yaml` are committed; CI builds with `--locked` and installs with `pnpm install --frozen-lockfile` | stage 0 decision |
| cargo-deny: licenses | the allowlist of D4; never-shipped dev npm packages have a separate list | D4 |
| cargo-deny: advisories | RustSec advisories fail CI; each ignore names the advisory and the reason; RUSTSEC-2023-0071 (`rsa`) is the documented exception | D4 |
| cargo-deny: bans | `aws-lc-fips-sys` (the `fips` feature), `ring` < 0.17.10, `aws-lc-sys` < 0.39 (D4); `libdbus-sys` from keyring's dbus features (D4); `openssl-sys` (stage 0 decision) | D4 |
| cargo-deny: sources | crates.io only; a git dependency needs an explicit allow entry | stage 0 decision |
| npm | license-checker in CI (D4); dependency install scripts run only for packages on an explicit allowlist (pnpm `onlyBuiltDependencies`); `pnpm audit` reported in CI | D4; stage 0 decision |
| GitHub Actions | third-party actions pinned to a full commit SHA with the version in a comment; `permissions: contents: read` unless a job needs more; `persist-credentials: false` on checkout; downloaded tools checked against a SHA-256; no `pull_request_target` workflow runs code from a pull request | stage 0 decision, as in `.github/workflows/secret-scan.yml` |
| Secret scanning | gitleaks on every push and pull request over the full history; a pre-commit hook pinned to v8.30.1; narrow allowlist entries in `.gitleaks.toml`; test secrets generated at runtime | [CONTRIBUTING.md](../CONTRIBUTING.md) |
| New dependencies | license checked and recorded in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) in the same pull request; cryptography only from the crates in section 5.1 | SPEC "Clean room", D4 |
| Dependency updates | automated update pull requests for Cargo, npm and Actions, reviewed like code, never auto-merged | stage 0 decision |
| Releases | built by CI from tags; update packages signed with the project key; SHA-256 checksums published with every release; OS code signing is the owner's decision | D7; checksums: stage 0 decision |

## 17. Residual risks

These risks are accepted for the MVP and are stated so users can judge them.

| ID | Risk | Why accepted, and what limits it |
|---|---|---|
| RR1 | The sync server can roll back, drop or withhold records and show different states to different devices. By forcing a full resync (`cursor_expired`) it can also make records deleted on another device come back, because a client keeps and pushes again every record missing from the snapshot. | Detection needs signed state (D14, deferred). Per-record rollback below the known revision is ignored, and the absence of a record never deletes it, so a forced resync cannot strip pins, roster entries, known hosts, keys or log entries ([sync-protocol.md](sync-protocol.md), section 6.9). Sync status reports records brought back this way. |
| RR2 | The server sees the metadata of section 14.2: who shares with whom, record counts per class, padded sizes, creation times (UUIDv7) and activity patterns. | Needed for sync (D10). Padding to 256 bytes hides exact sizes. |
| RR3 | Every member, viewers included, reads every secret of a team vault; editors can change execution-relevant host settings and delete shared records for everyone. | Inherent in end-to-end shared vaults (D13). Stated in the invitation dialog. |
| RR4 | Records are not signed: a key holder together with an active server can forge records, a viewer for the current epoch, a removed member for the epochs it held. Even with an honest server, any member, viewers included, can publish team connection log entries whose user fields name another member, because clients cannot compare them with the server's `created_by`. | Record signatures are deferred (D13). Checking the author of team log entries is an open item (section 18). |
| RR5 | Key distribution is TOFU: an active server can hand a new member a fake owner key unless fingerprints are compared. A fake member receives a vault key only if an owner confirms it in the recipient dialog without comparing its fingerprint. | Fingerprints in Team and in the recipient dialog; automatic rotations never add recipients outside the client's roster; epoch chain check for existing members (section 5.7). |
| RR6 | Old wraps open with old credentials: old database copies (`backups/`, OS backups, user copies) keep the wraps of their time, and the server and its backups keep the `kek_wrap` and `recovery_wrap` of every earlier password and recovery key, with the old salt. Until the personal vault key is rotated, such a wrap also yields the key of current and future data. Other devices keep opening their local copy with the old password or the old recovery key until they sign in again. | Recovery and recovery key reissue always rotate the personal vault key, and a password change rotates it when the user says the old password may be compromised (section 5.9); after a rotation, old wraps open only data from before it. Deleting pre-migration copies after credential changes is an open item. Users must protect or delete old copies. |
| RR7 | Anyone with the database, a server copy or a server backup can guess the master password offline. The minimum length is 8 characters with no composition rules (D29), so a short password is cheaper to guess, and the attempt limit of the unlock screen (section 8) does not apply offline. | Argon2id 64 MiB, t = 3, never below the cost of the mobile preset; the email-bound salt makes every guess cost one Argon2id per account; the first-run screen shows a strength indicator (UI 13.1); the recovery key has 256 bits. A strong password is the user's part. |
| RR8 | Same-user malware or a compromised OS sees everything while the app is unlocked. With "remember on this device", which is on by default (D29), the device key is readable by same-user programs on Windows and Linux, and starting or restarting the app unlocks it without the password. On Windows with a roaming profile the keychain item also roams to the user's other machines (section 8). | The owner's choice of convenience (D29). The risk is stated next to the toggle on the first-run screen and in Settings, where it can be turned off (UI 13.1, 13.4); lock and auto-lock still wipe the keys and ask for the password; disabled in unsigned macOS builds (D7). |
| RR9 | Unsigned builds: users must bypass Gatekeeper or SmartScreen, the OS cannot check the origin of the first download, and on macOS the keychain-based features are disabled (D7). | Updates are signature-checked; release checksums are published. Buying certificates is the owner's decision. |
| RR10 | History capture is a heuristic (section 10.1): it can miss prompts, capture wrong text, let a secret through (unknown formats, low-entropy passwords) or drop harmless commands. | Fails closed where possible; history is encrypted and not synced by default; any entry can be deleted (UI 4.2). OSC 133 is deferred. |
| RR11 | Secrets can remain in memory beyond their use: terminal scrollback while locked, revealed values in the webview, swap, hibernation files and Windows crash dumps. Also copies the code cannot wipe: serde_json's internal scratch buffer while it parses a decrypted payload (strings with escapes, such as the line breaks of every PEM key), ssh-key's PPK passphrase and decrypted PPK bytes (ssh-key 0.7.0-rc.11 wipes neither), the `String`s russh takes for passwords and keyboard-interactive answers, and text shown or typed in GPUI views (run 4, plan 9.4). | Rust-side zeroization (buffers allocated once at their final size, section 9.5), the webview reveal rule, no core dumps on Linux and macOS. Full-disk encryption recommended. An allocator that wipes freed memory would cover the copies above; it is not used (D83), so they are a known limitation (section 9.5). |
| RR12 | TOFU: an attacker in the path of the very first connection to a host is trusted. | The fingerprint is shown; imported `known_hosts` entries avoid TOFU; host certificates come later. |
| RR13 | Other programs can read sensitive clipboard content during the 30 seconds, and clipboard managers that ignore the platform hints can keep it. | Clearing, platform hints (section 9.4). |
| RR14 | Forwarded ports and the SOCKS proxy on 127.0.0.1 are open to every local process and user; a web page can reach forwarded web services through DNS rebinding. | Loopback default; non-loopback needs a warning (UI 8.4). |
| RR15 | Account enumeration is only partly prevented: registration answers `email_taken`, and real accounts whose KDF parameters differ from the desktop preset (mobile or upgraded) are told apart from fake answers. | Invite-only registration by default; fake salts and dummy Argon2id for the common case. |
| RR16 | The recovery key copy inside the personal vault means anyone who once unlocks the vault learns the recovery key, and can later reset the password with it (plus a TOTP code, if TOTP is on). The core keeps this copy in local mode too, where the app never shows it (D29). | Needed to show the key again (UI 13.4). After a suspected compromise: change the password with "my old password may be compromised" and reissue the recovery key; both rotate the personal vault key and sign out the other devices. |
| RR17 | A user's X25519 private key cannot be rotated in the MVP; if it leaks, every team vault key wrapped to that user is exposed. | Deferred; the key lives only inside the encrypted personal vault. |
| RR18 | Deleted files are not securely erased (SSDs, journaling, snapshots); temporary SFTP files and plaintext exports are plaintext while they exist. | Short-lived temporary files; warnings for exports; full-disk encryption recommended. |
| RR19 | With agent forwarding on, a malicious host can use the forwarded keys for the length of the session. | Off by default per host (UI 3.1). |
| RR20 | An update source or network attacker can withhold updates or keep serving an old manifest. | Signatures stop tampering, not withholding; manual checks are always available. |
| RR21 | With an active server, a removed member can keep reading new records or plant an epoch. The server can hide the new epoch from the remaining members and keep accepting their writes under the old epoch, which the removed member holds. Or it can hide the removal from some members and give them an epoch made by the removed member, who holds the previous key and is still pinned. A removed member's epoch is refused only on devices that hold the removed roster entry; the server can withhold that entry from the account's other devices or serve a new device an older revision, which puts them in the position of members from whom the removal is hidden. | Clients refuse epochs from users they saw removed and from non-owners, and Team shows the epoch, who sent the key and when, so members can compare after a removal (section 5.7). A roster entry written on a device the server keeps on a forked view cannot make a removed user an automatic recipient on the device that removed it: `accepted` counts only with a commitment to the key of the current epoch and a verified pin, and in a sync conflict the version that says `removed` wins (section 5.7). The `owner` role that epoch acceptance checks is the one the server reports in the member list, so an active server can present any listed user who is not `removed` on that device as an owner. Changing the host secrets is the only guaranteed remedy, and Team tracks it (section 15). Signed membership changes are deferred (D14). |

## 18. Open items

| Item | Owner | When |
|---|---|---|
| Delete pre-migration copies in `backups/` after a password change, a recovery and a recovery key reissue (proposal, RR6) | [storage-format.md](storage-format.md) | stage 1 |
| "My old password may be compromised" option in the password change dialog (section 5.9) | UI 13.4 | stage 1 |
| Lock when the OS session locks or the computer sleeps (proposal; not in UI 13.3) | UI 13.3 | stage 1 |
| Exclude the app from Windows Error Reporting dumps; lock key memory pages | [architecture.md](architecture.md) | stage 1 |
| Tune the secret detector thresholds against the test corpus | this document | stage 2 |
| Email change: password re-entry at the step that makes the change effective, and the sign-out of other devices (section 5.9) | UI 12.2 | stage 7 |
| Sync status entry for records brought back by a full resync (RR1) | UI 12.2 | stage 7 |
| Bring a server whose personal vault epoch is below the device's back to the local epochs, after "server state rolled back" (proposal: the client re-publishes its newer epochs and their wraps after password re-entry) | [sync-protocol.md](sync-protocol.md) | stage 7 |
| Recovery dialog: editable account email, the hint that the email may have changed on another device, and the offline recovery of the local copy (UI 13.2) | UI 13.2 | stage 7 |
| Keep the team vault archives of an earlier account readable after the device binds to another account: their HPKE `info` needs that account's `user_id`, which `meta.account_user_id` then no longer holds ([data-model.md](data-model.md), section 18.5) | [storage-format.md](storage-format.md) | stage 7 |
| Recipient dialog for rotations, the "rotation waiting for confirmation" state, epoch refusal warnings and the epoch line in Team (section 5.7) | UI 12.3 | stage 8 |
| "Secrets to change" list after a removal and the removal dialog wording (section 15) | UI 12.3 | stage 8 |
| Highlight changes made by other members to execution-relevant fields of team hosts (RR3) | UI 3.1, 12.3 | stage 8 |
| Return `created_by` with pulled `connection_log` records so clients can check the `user_id` of team log entries (proposal, RR4) | [sync-protocol.md](sync-protocol.md) | stage 8 |
| Audit status and exact version of the `hpke` crate; confirm it accepts the 77-byte `info` | [architecture.md](architecture.md) | stage 8 |
| OSC 52 clipboard writes as an opt-in | UI 12.4 | after the MVP |
| Record signatures, signed membership changes, rollback detection | D13, D14 | after the MVP |
| Biometric unlock | D5 | after stage 6 |

## 19. Reporting vulnerabilities

Report vulnerabilities privately as described in [SECURITY.md](../SECURITY.md). Design-level issues in this document are in scope even before code exists.
