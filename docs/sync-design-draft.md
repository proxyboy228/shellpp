# Sync design draft

Status: draft for the owner, 2026-10-22. Not normative: it changes nothing in `SPEC.md`, [decisions.md](decisions.md) or the stage 0 design documents, and SPEC stage 7 is not started. The owner's answers to section 11 would become decisions and then go into [sync-protocol.md](sync-protocol.md), [security.md](security.md), [data-model.md](data-model.md) and [storage-format.md](storage-format.md).

Conventions:

- **Decided**: a `[decision]` in `SPEC.md` or an accepted decision `Dn`; never offered as an option. An option that would change one says so.
- **Designed**: a "stage 0 decision" of the design documents: the current design, not chosen point by point by the owner.
- References: SP = sync-protocol.md, SEC = security.md, DM = data-model.md, SF = storage-format.md, UI = ui-behavior-spec.md, ARCH = architecture.md; "SP 6.9" is its section 6.9; RRn are the residual risks of SEC 17.
- Choices 1 to 15 are tagged **[open]** (specified nowhere), **[open item]** (listed in SEC 18) or **[revisit]** (designed, shown because the owner asked about the topic). Where an option is the current design, it says so.
- "Recommendation" is advice for the owner, not a decision.

## 1. Purpose and scope

SPEC stage 7 (sync server and client, several devices) and stage 8 (shared vaults, roles, invitations) are already designed in detail in SP: API, record sync, conflicts, team vaults, key epochs, limits, server schema. This draft does not redesign them. For each topic the owner named it collects what is decided or designed, lists what is still open (or worth a second look) with options and a recommendation, and ends with short questions.

In scope: the personal vault across one account's devices, team vaults (membership, removal, rotation), server data, account deletion, server backups, threats. Out of scope: an implementation plan, UI layout, the mobile client, real-time server push (none in the MVP, SP 6.11), record signatures and signed membership changes (deferred, D13, D14), rotation of the user's X25519 key pair (RR17).

## 2. What is already decided

### 2.1 Principles (SPEC)

- P1: local-first; sync is optional.
- P2: no license checks or remote disabling, so no server-side switch may lock a client.
- P3: local data stays readable and exportable; revoking access to a shared vault, deleting the account, signing out or losing the server stops only future sync and never erases or locks data already received.
- P5: zero-knowledge; the server stores encrypted blobs and sync metadata; keys never leave the client in plaintext.

### 2.2 Decided

- Encryption scope: the whole payload of every record; only ids, coarse type, vault, revision, key epoch and sync fields are clear; local and server envelopes are the same bytes; 256-byte padding (D10).
- Login: Argon2id → master key → HKDF → KEK and auth key; only the auth key reaches the server; fake pre-login salt; TOTP; a password change rewraps and signs other devices out. Amended: the salt binds the email, both auth keys bind the server origin (D11).
- Recovery key: 256 bits, wraps the personal vault key, yields a recovery auth key; shown at registration, not in local mode (D11, D29).
- Personal epoch: the client never lowers it on a server's word; a bound device can recover its local copy offline with the recovery key (D11, amended).
- Protocol: cursor per vault; UUIDv7 ids, revisions; tombstones kept 90 days, an older cursor does a full resync; `base_revision` check on push (SPEC, D12).
- Conflicts: record-level last-writer-wins by HLC, ties by device id, delete versus edit alike; the loser kept as a conflict record on all devices (SPEC, D12).
- Sharing: the vault is the only unit; every member, viewers included, gets all secrets; roles enforced on push; records not signed (D13).
- Team keys: per-user X25519 key pair synced in the personal vault; HPKE Auth wraps per member; TOFU pins and fingerprints; two-step invitations (SPEC, D13, D14).
- Removal: new key epoch, background re-encryption, a hint to change host secrets; the removed member keeps a read-only archive (SPEC, D14).
- Rosters: each client keeps its own roster of every team vault; an automatic rotation runs only when every member is accepted for the current epoch (commitment to its key, verified pin); epochs from users seen removed are refused (D14, amended).
- Server attacks: substitution and reordering detected through the AAD; rollback and deletion not detected (D14).
- Owners: several per vault; the only owner cannot delete the account before transferring ownership (D14).
- Selective sync: per-device toggles for the personal vault, deleting nothing; team vaults sync in full (D15).
- Sign-in with local data: merge with deduplication; a rebind keeps ids; an earlier account's records stay in a read-only archive (D15).
- Recordings are never synced (D16).

### 2.3 Designed in stage 0

Transport and tokens (SP 2, 4); the server schema (SP 14), endpoints (SP 3) and limits (SP 10); pull, push, full resync and server-restore handling (SP 6); the detailed roster, epoch acceptance and rotation recipient rules (SEC 5.7, SP 7); team vault epochs never going back on a device (SF 6.2); immediate account deletion (SP 5.8); backups as the operator's matter (SEC 14.4). The choices below assume all of this unless they say otherwise.

### 2.4 Gaps and inconsistencies found while reading

1. A team vault whose server epoch is below the device's (after a restore) is not handled: SP 6.11 covers only a newer epoch, and `stale_key_epoch` cannot re-seal downward, since epochs never go back (SF 6.2). SP 6.9 asks owners to "repeat member removals made after the backup"; that makes a new key under an epoch number the members already hold with another key, which their clients refuse, so the vault forks. SEC 18 covers only the personal vault. See Choice 3.
2. P3 names revocation, account deletion, sign-out and server outage, not deletions: a member with write access, or a removed member with an active server (RR4), can tombstone records off every member's device. Consistent with the letter of P3, close to what motivated it. See Choice 8.
3. Revoking a device stops only its tokens (SP 4.7); a lost device has no key rotation path except a password change with "my old password may be compromised". See Choice 9.
4. UI 13.6 has conflict badges for seven object kinds; conflict copies of the other types count in "Conflicts: N" but have no specified view. See Choice 7.
5. After concurrent SSH ID regeneration on two devices, the winning `ssh_id` version (DM 6.6) need not match the key published last with `PUT /v1/account/ssh-id` (SP 5.6). See question 9.
6. One object created on two devices before they sync (the same `~/.ssh/config` imported on both) becomes two records: import idempotency matches only local records (DM 18.4), and only keys have a merge rule (DM 18.3, D40). See Choice 6.
7. A server restore also brings back deleted accounts with their old verifiers, deleted vaults and revoked devices; SP 6.9 mentions only member removals. See Choice 15.
8. Wording: SPEC and D14 say the AAD binds the record "type"; it binds the class (D10's coarse type); the exact type is inside the ciphertext (SF 5.2).

## 3. Data model on the server

### 3.1 What the server stores

Designed schema: SP 14. The server keeps only the newest version of each record (SP 6.1).

| Data | Tables | Readable by the server |
|---|---|---|
| Account: email, verifiers of `auth_key` and `recovery_auth_key`, KDF parameters and salt, `kek_wrap`, `recovery_wrap`, public keys, TOTP secret (encrypted at rest) | `users` | all but the wraps' contents |
| Devices: id, created, last seen (day), revoked | `devices` | yes; names are encrypted `device_info` records |
| Tokens, tickets, codes | `*_tokens`, `auth_tickets`, `*_codes` | SHA-256 hashes only |
| Vaults: kind, epoch, `last_seq`, `min_valid_seq`, `rotation_pending`, creator; members and roles | `vaults`, `vault_members` | yes; names are in the encrypted `vault_meta` |
| Epoch chain and one HPKE wrap per member for the current epoch | `vault_key_epochs`, `vault_key_wraps` | no |
| Records: envelope, header copies, `seq`, creator, last writer and device, times | `records` | header only |
| Invitations | `invitations` | yes; tokens as hashes |

- Wraps are per member, not per device: a user's devices share one HPKE wrap through the X25519 key in the personal vault (D13, SP 7.1); the personal vault key is wrapped once per account (D11); device keys stay on the device (SF 6.3).
- Tombstones are records with a flag, purged after 90 days, raising `min_valid_seq` (D12, SP 6.9).
- Visible metadata is the list of SEC 14.2 (D10): ids (UUIDv7, so creation times), class, revision, epoch, padded size, timing, memberships, the author of each write (RR2).

### 3.2 Choice 1: storage quotas [open]

The server limits record size, request size and request rate (SP 10), not what an account stores in total. A quota can only stop pushes; local data stays untouched (P2, P3).

| Option | For | Against |
|---|---|---|
| (a) No quotas | nothing to build; registration is invite-only by default (SEC 14.1) | one account can fill an open-registration server |
| (b) Optional per-account limits (records, bytes) set by the operator, off by default; a push item status `quota_exceeded` marks the record "not synced" | protects open servers; a new item status is a compatible change (SP 13) | whose quota a member's write to a team vault uses; one more error state |
| (c) Per-vault limits | simple accounting | does not stop one account creating many vaults |

Recommendation: (b), off by default. Why: protects operators who open registration and changes nothing for invite-only servers.

## 4. Pull/push protocol

### 4.1 Designed

- Cursor: one per vault, the server `seq` up to which every write was applied (D12, SP 6.1).
- Pull: pages of up to 1000 records or 8 MiB; the cursor moves over records filtered out by class; a page and its cursor are stored in one transaction (SP 6.3, 6.4).
- Push: up to 500 `{base_revision, envelope}` items or 16 MiB, each judged on its own; accepted items commit together (SP 6.5, 6.6).
- Idempotency: a retry after a lost response gets `conflict` with the client's own version and adopts it; pulled versions at or below `synced_revision` are ignored (SP 6.4, 6.5).
- Offline queue: none of its own; a record is pending while `revision ≠ synced_revision` (SP 6.1).
- Partial failures: client-bug statuses mark the record "not synced"; unknown statuses retry; network and `5xx` errors back off from 5 s to 5 min; `429` honours `Retry-After` (SP 6.5, 6.11).
- Cycle: unlock, 2 s after a change, every 60 s, "Sync now"; paused while locked (SP 6.11).
- Full resync on a purged cursor; absence from a snapshot never deletes, a hard rule for seven security-relevant types (SP 6.9).
- A lost response to a token refresh signs the device out, "accepted in the MVP" (SP 4.6); see question 18.

### 4.2 Choice 2: records missing from a full snapshot [revisit; open item SEC 18]

After a full resync, synced records the snapshot lacks are pushed back (SP 6.9). This upholds P3 against a server that forces resyncs, but undoes deletions whose tombstones were purged before this device saw them (RR1). The sync status entry for them is an open item. The seven hard-rule types are pushed back under every option.

| Option | For | Against |
|---|---|---|
| (a) Push back, show a count (current design) | converges without user action | a deliberate deletion (a leaked key) returns everywhere, unseen |
| (b) Keep them local; list "Not on the server: N" with Push back, Keep local only, Delete | nothing returns without consent | other devices lack them until reviewed; a hostile server can force the list |
| (c) Push back and list them for review, with Delete again per record | converges; a deletion is redone in one place | a deleted record is briefly back elsewhere |

Recommendation: (c). Why: keeps the designed convergence and P3 and gives the open status item a concrete form.

## 5. Encryption and key wrapping

### 5.1 Designed

- Key chain: password → Argon2id (email-bound salt) → `master_key` → HKDF → `kek` and origin-bound `auth_key`; random vault keys; the personal key wrapped by `kek`, `recovery_kek` and optionally the device key (D8, D11, D29, SEC 5.2).
- Records: XChaCha20-Poly1305 per record, the header as AAD (D10, D14, SF 5).
- Team vaults: HPKE Auth, `info` binding vault, epoch, recipient and sender; one wrap per member for the current epoch; older keys through the epoch chain (D14, SEC 5.7, SP 7.7).
- Trust: TOFU pins, local fingerprints, rosters with epoch commitments, epoch acceptance, rotations that never add a recipient automatically (SEC 5.7).
- Rotation: removal and rotation in one transaction; leave and account deletion set `rotation_pending`; owner clients re-encrypt in the background without conflict copies (D14, SP 7.7–7.9). The personal vault rotates on recovery and reissue, optionally on a password change (SP 5.1–5.3).
- Recovery through the server rotates; an offline recovery reopens only the local copy (SP 5.2).

### 5.2 Choice 3: the server's epoch is below the device's [open item, extended to team vaults]

After a restore (or a rollback by a hostile server) the server can report an older epoch than a device holds. The personal vault then stops syncing (SP 5.1); team vaults are unspecified (section 2.4, item 1).

| Option | For | Against |
|---|---|---|
| (a) Stop syncing the vault, show "server state rolled back" (current design for the personal vault, D11) | safe; nothing to build | no way back inside the app |
| (b) The device holding the newer epochs re-posts them, same keys, one by one through the existing `key-epochs` endpoint (personal: after password re-entry; team: an owner, under the interactive recipient rules, removing again users its roster marks `removed`), then pushes records back (SP 6.9) | no new endpoint (each step is `current + 1`); held keys stay valid; no fork | needs an owner device for a team vault; never automatic, as a hostile server could provoke it |
| (c) Start over in a new vault, moving data as at sign-in to another account (DM 18.5) | no special rules | new ids break links; members must accept a new vault |

Recommendation: (b), confirmed by the user each time. Why: it is the SEC 18 proposal, fits the existing API and removes the fork of section 2.4, item 1.

### 5.3 Choice 4: team vault archives of an earlier account [open item]

An archived team vault opens through its HPKE wrap, whose `info` needs the recipient's `user_id`; `meta.account_user_id` names only the last bound account (SF 4.5).

| Option | For | Against |
|---|---|---|
| (a) Store `account_user_id` and `server_url` on each `vaults` row (additive migration, SF 11) | smallest change; the manual decryption guide (SF 14.5) stays valid | still needs the earlier account's key pair and pins, kept in its archived personal vault |
| (b) At archive time, rewrap the held keys under a key derived from the current personal vault key (a new local wrap kind), redone on every personal rotation | archives open without HPKE, pins or old key pairs | new wrap kind and derivation; SF 14 changes |
| (c) A list of earlier accounts in `meta`, tried in turn | no change to `vaults` | trial and error at unlock |

Recommendation: (a). Why: closes the open item with one column and leaves the third-party decryption steps as they are.

### 5.4 Choice 5: a pulled record whose key epoch goes down [open]

An honest server accepts writes only under the current epoch (SP 6.6, check 5), and epochs only rise, so versions of one record never go down in epoch. A removed member with an active server can still write under epochs it held (RR4), including tombstones of already re-encrypted records.

| Option | For | Against |
|---|---|---|
| (a) Apply any higher revision whatever its epoch (current design) | nothing to build | such a member can edit or delete re-encrypted records on every device |
| (b) Refuse a pulled version whose `key_epoch` is below that of the local synced version, with the integrity warning of SP 6.4 | client-only; never fires against an honest, unrestored server | protects only re-encrypted records; new records under old epochs still get in |
| (c) Record signatures | closes RR4 | deferred by D13 and D14 |

Recommendation: (b). Why: one check removes the most damaging case of RR4 at no cost to honest servers.

## 6. Conflicts

### 6.1 Decided and designed

- Decided (SPEC, D12): record-level last-writer-wins by HLC, ties by device id, delete versus edit alike; the loser kept as a conflict record on all devices, with "Keep this" and "Keep other" (UI 13.6). Keeping both copies is the decided model; SPEC excludes field merge, and no option below merges fields.
- Designed (SP 6.7): equal data adopts the server copy; a side that only re-encrypted loses without a copy; for `roster_entry` the version saying `removed` wins; no copy of a copy. The server only checks `base_revision`.

### 6.2 Choice 6: the same object created on two devices [open]

Section 2.4, item 6: two records with different ids and the same natural key are not a conflict, so nothing reconciles them.

| Option | For | Against |
|---|---|---|
| (a) Leave duplicates for the user | nothing to build | duplicates after every offline import on two devices |
| (b) Merge on pull by natural key (DM 18.6) | no user work | silent tombstones; devices may merge differently; against "existing records are never overwritten" (D21) |
| (c) A duplicate hint with Merge, extending the key merge (DM 18.3) to the other types by natural key | the user decides; D40 already plans the Keychain hint for stage 7 | a merge rule per type (survivor, references) |

Recommendation: (c). Why: the approach D40 already chose for keys, with no silent changes.

### 6.3 Choice 7: conflict copies of types without a view [open]

Section 2.4, item 4: for example records of class `vault`, snippet packages, workspaces, appearance, history, the connection log and credential bindings.

| Option | For | Against |
|---|---|---|
| (a) Copies for all types; a generic comparison (fields, secrets masked) in UI 13.6 for types without a card | no SPEC change; nothing lost | conflicts of `device_info` or `history_hint_state` are noise |
| (b) Silent resolution without a copy for device-generated types | less noise | changes a SPEC `[decision]` |
| (c) Winner rules per type where the meaning is clear, as for `roster_entry` | fits record-level LWW | each rule needs a security review |

Recommendation: (a), with (c) only where a type needs it. Why: keeps SPEC as written and gives every counted conflict a way out.

## 7. Access revocation (principle 3)

### 7.1 What can and cannot be revoked

| Event | What stops | What stays on the device | Source |
|---|---|---|---|
| Member removed or leaves | sync; later epochs (honest server) | read-only archive with Export and Copy to personal vault | SPEC, D14, SP 7.8, 7.11 |
| Team vault deleted | the server copy | an archive on every member's device | SP 7.11 |
| Device revoked | its tokens | everything; a new sign-in clears the revocation | SP 4.7 |
| Log out | sync on that device | the local copy, unless the user deletes it there | UI 12.2 |
| Account deleted | the server account | local-only personal vault, team archives | SP 5.8 |
| Server gone | sync | everything; offline recovery works | P1, SP 5.2 |
| Sync toggle off | sync of that class | everything, here and on the server | D15 |

Cannot be revoked: anything already received, host passwords, keys and past epoch keys included. After a removal only changing host secrets helps; the dialog says so and Team tracks "Secrets to change" (D14, SEC 15). With an active server a removed member may keep reading (RR21).

### 7.2 Choice 8: deletions made by other devices and members [open]

A received tombstone removes the local copy (SF 4.6). P3 does not cover deletions (section 2.4, item 2).

| Option | For | Against |
|---|---|---|
| (a) As designed: the tombstone deletes; only conflict copies, archives and exports keep old data (RR3) | simple; "delete everywhere" works | one editor can erase a team's data from all devices |
| (b) Local "Recently deleted": when a pulled tombstone arrives in a team vault, keep the last live version for 90 days, encrypted and never synced, to copy back or export | P3 in spirit; offline; no protocol change; also catches forged tombstones | deleted secrets live longer on each device; a new table and view |
| (c) The server keeps the previous N versions of each record, restorable by members | covers overwrites; works across devices | more storage; edit history visible to the server; no help when the server is gone or hostile |

Recommendation: (b), for team vaults only. Why: covers the case where someone else deletes your data, without a server change.

### 7.3 Choice 9: a lost or stolen device [open]

A stolen device holds the current personal and team keys, encrypted at rest (SEC 6, note d). Revoking it stops its tokens only, and a sign-in with the password undoes the revocation (SP 4.7).

| Option | For | Against |
|---|---|---|
| (a) Revoke tokens only; a password change with rotation is separate (current design) | nothing to build | users may think a revoke protects future data |
| (b) Revoke asks "Lost or stolen?"; yes continues with a password change that rotates the personal vault (SP 5.1) and offers to rotate owned team vaults; for others it says to ask an owner | covers future data with existing operations | no rotation of vaults the user does not own (a "request rotation" call would be new) |
| (c) Rotate the personal vault on every revoke | no question | re-encryption on every routine clean-up |

Recommendation: (b). Why: makes an existing operation the obvious next step, without new protocol.

## 8. Account deletion

### 8.1 Designed

- Password proof and TOTP; `409 sole_owner` while the user is the only owner of a team vault with other members (D14, SP 5.8).
- One transaction deletes the user, the personal vault (records, epochs, invitations), devices, tokens and tickets; team vaults without other members go; elsewhere membership and wraps go and `rotation_pending` is set (SP 5.8).
- The personal vault needs no tombstones: only this account's devices sync it, and they keep their copies (follows from SP 5.8).
- The user's records in team vaults stay, with a dangling `created_by` (follows from SP 14; not stated).
- Local: the personal vault becomes local-only and opens with the same password; team vaults become archives (SP 5.8, SEC 5.4).
- Deletion is immediate; backups are the operator's matter (SP 5.8; section 9).

### 8.2 Choice 10: grace period [revisit]

| Option | For | Against |
|---|---|---|
| (a) Immediate (current design) | simple; the user's devices keep all data | no undo on the server; members lose the user at once |
| (b) Disable now, delete after N days; a sign-in cancels | undo for a mistaken or coerced deletion | data stays N more days; a disabled state for every endpoint |
| (c) Immediate, after a dialog that offers Export and says what members lose | no server change | no undo |

Recommendation: (a) with the dialog of (c). Why: P3 already keeps the data locally, so a server undo adds state for little gain.

### 8.3 Choice 11: telling the account's other devices [open]

They get `401` and show "signed out"; a sign-in then fails with `invalid_credentials`.

| Option | For | Against |
|---|---|---|
| (a) As designed | nothing kept after deletion | the user may try to recover a deleted account |
| (b) Keep the account's refresh token hashes and the date for 30 days and answer `401 account_deleted`; the client says so and that the local copy is kept | clear; no email or data kept; a compatible new code (SP 13) | 30 days of minimal metadata |
| (c) The deleting device asks to sign the other devices out first | no server change | misses offline or lost devices |

Recommendation: (b). Why: a clear message for the price of token hashes.

### 8.4 Choice 12: the user's published connection log entries in team vaults [open]

| Option | For | Against |
|---|---|---|
| (a) Keep them, unmentioned | the team keeps its record | the user may not expect it |
| (b) Offer "Also delete my published log entries"; the client tombstones them (allowed for the creator) first | the user's choice | removes data others already received (Choice 8) |
| (c) Keep them and say so in the dialog | honest; team audit kept | no removal except by hand beforehand |

Recommendation: (c). Why: keeps the team's audit trail and removes nothing others received.

## 9. Server backups

### 9.1 Decided and designed

- SPEC asks for DB backup instructions; ARCH 10.2 puts them in `server/README.md` (`pg_dump`, an online SQLite backup, what a restore means for clients).
- A backup is a passive server (A5): verifiers, wraps, salts and metadata, open to offline guessing (RR7); old wraps open data up to the next personal rotation (RR6). Operators should encrypt backups and restrict access (SEC 14.4).
- Restore: a cursor above `last_seq` gets `410 cursor_expired` (`reason = ahead`); the client pulls a snapshot, pushes back what is newer or missing and warns "server state rolled back" (SP 6.9), but only while `last_seq` is below its cursor (D14).
- Backup retention is not specified.

### 9.2 Choice 13: tooling and encryption [open]

| Option | For | Against |
|---|---|---|
| (a) Instructions only (current design) | nothing to maintain | unencrypted dumps kept for years |
| (b) A built-in `shellpp-server admin backup` writing a consistent dump encrypted to an operator's public key | encrypted by default | a new dependency, an operator key to keep, a format to maintain |
| (c) Tested recipes: dump piped to an encrypting tool, a scheduled compose service, a recommended retention (for example 30 days) | encrypted, short-lived backups without server code | relies on the operator |

Recommendation: (c). Why: encrypted, short-lived backups at no code cost.

### 9.3 Choice 14: detecting a restore [open]

| Option | For | Against |
|---|---|---|
| (a) The cursor check only (current design) | nothing to build | missed once the restored server has taken more writes than it lost |
| (b) A random `server_generation` returned by `GET /v1/server`; every restore ends with `shellpp-server admin restored`, which replaces it; a client seeing a new value runs the SP 6.9 restore procedure for every vault | catches every honest restore; small | needs the operator's step; a hostile server can skip it (D14 holds) |
| (c) As (b), plus a copy of the value outside the database, compared at start | no operator step | depends on the volume layout |

Recommendation: (b). Why: honest restores are the common case, and the check is one field.

### 9.4 Choice 15: what a restore brings back [open]

| Option | For | Against |
|---|---|---|
| (a) Operator's matter (current design) | nothing to build | deleted accounts return with old verifiers; revoked devices and removed members return |
| (b) An append-only journal of irreversible events (ids, times) outside the database; `admin restored` (Choice 14) re-applies deletions and revocations; owners repeat removals as in Choice 3 (b) | one command restores the intended state | another store to protect; needs Choice 14 (b) |
| (c) Documented manual steps: server logs carry user ids (SEC 9.1); `admin user delete` for each account deleted after the backup | no new mechanism | easy to forget |

Recommendation: (c) now, (b) if Choice 14 (b) is taken. Why: the manual path works at once; the journal pays off only with a restore marker.

## 10. Threats

Actors are those of SEC 3.

| Threat | Designed mitigation | Residual | Choices |
|---|---|---|---|
| Active server reads data | E2E encryption; auth key separate from the KEK and bound to the origin; KDF floor and ceiling; email-bound salt | offline guessing from wraps (RR7) | 13 |
| Rollback, withholding, fork | lower revisions ignored; personal epochs never go back; roster epoch commitments; absence never deletes | not detected (D14, RR1) | 2, 3, 5, 14 |
| Forced full resync | missing records pushed back | deletions return (RR1) | 2 |
| Key substitution | TOFU pins, local fingerprints, rosters, chain check, no automatic recipients | first contact (RR5) | none |
| Stolen device | encryption at rest, lock, token revocation, no remote wipe | full access if unlocked or remembered with the OS session open (RR8) | 9 |
| Removed member | new epoch, re-encryption, removed senders refused, Secrets to change | knows every secret; with an active server reads or forges (RR4, RR21) | 5, 8 |
| Malicious editor | server roles, conflict copies, 90-day tombstones | deletes or redirects shared hosts (RR3) | 8 |
| Metadata leakage | padding, encrypted names and types, IPs in memory only, last seen by day | SEC 14.2 list (RR2) | 8 (c) adds history, 11 (b) keeps hashes, 13 protects backups |
| Replay | refresh rotation with reuse detection; used TOTP steps refused; AAD binds each envelope; old wraps refused by the epoch rule | none known | 5 extends the epoch rule to records |
| Leaked backup | verifiers of high-entropy keys; one Argon2id per password guess | old wraps (RR6), guessing (RR7) | 13, 15 |
| Honest restore | `410 ahead` while the cursor is ahead | missed after new writes (D14) | 3, 14, 15 |

## 11. Questions for the owner

1. Are the stage 0 design choices of SP and SEC (section 2.3) the accepted basis for stages 7 and 8, so that only the choices here are open?
2. Choice 1, quotas: (a), (b) or (c)?
3. Choice 2, records missing from a snapshot: (a), (b) or (c)?
4. Choice 3, server epoch below the device's: (a), (b) or (c)?
5. Choice 4, archives of an earlier account: (a), (b) or (c)?
6. Choice 5: refuse a pulled record whose key epoch goes down, yes or no?
7. Choice 6, duplicates from two devices: (a), (b) or (c)?
8. Choice 7, conflicts without a view: (a), (b) (a SPEC change) or (c)?
9. Concurrent SSH ID regeneration (section 2.4, item 5): should the device whose `ssh_id` version wins re-publish its public key when the server's differs, yes or no?
10. Choice 8, deletions by others: (a), (b) or (c)? With (b), is 90 days right?
11. Choice 9, lost or stolen device: (a), (b) or (c)?
12. Choice 10, account deletion: (a), (b) with which N, or (c)?
13. Choice 11: answer `account_deleted` to the account's other devices for 30 days, yes or no?
14. Choice 12, published log entries: (a), (b) or (c)?
15. Choice 13, backups: (a), (b) or (c), and which retention to recommend?
16. Choice 14: add `server_generation` and `admin restored`, yes or no?
17. Choice 15, what a restore brings back: (a), (b) or (c)?
18. A lost response to a token refresh signs the device out (SP 4.6). Keep that, or accept the previous refresh token once within a short window (for example 60 s) while its successor is unused?
