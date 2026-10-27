# Stage 8 readiness: what stage 7 leaves for team vaults

Written at the end of run 7 (2026-10-23, plan section 6.3); the section
"Release 1.0: room left for what comes after it" was added in run 17
(2026-10-27). No stage 8 code exists; this note lists what the stage 7 code
already does for team vaults (docs/sync-protocol.md, section 7; D13, D14)
and what stage 8 still needs.

## Release 1.0: room left for what comes after it

Checked in run 17 against the code, for team vaults and for the archive of
a previous account, which comes first after 1.0 (its keys then wrapped under
the personal vault key, storage-format 6.2; D197). In short: a 1.0 client
that meets data of a later version refuses it with a clear message or keeps
it untouched; it never damages it.

- **The local database** has a schema version (`store::SCHEMA_VERSION`, 2
  today) and forward migrations. A database written by a later version
  (the archive's tables, team vaults' sessions) is refused before anything
  is written, with the code `newer_schema`, which the app shows as "This
  data was written by a newer version of Shell++. Update Shell++ to open
  it." Tests: `newer_schema_is_refused_and_the_file_is_unchanged` and
  `newer_schema_in_wal_mode_is_refused` (crates/core/src/store/tests.rs).
- **Records.** The envelope has a version byte; a record with a later
  envelope version, an unknown type in a known class, or a payload version
  above the one the client knows opens as `Opened::Unsupported`: it is kept
  as it is, left out of the index and never rewritten (`vault::open_row`;
  test `checks_detect_tampering_and_unsupported_records` in
  crates/core/src/vault/mod.rs; across a lock and an unlock,
  `records_of_a_later_version_are_kept_and_never_rewritten`). Sync applies
  such a row without opening it further and keeps it hidden
  (`engine::apply_page`); a conflict over one keeps the other version
  (`KeepOther::Unsupported`, tested in service/sync_ops.rs). A pending row
  that a rotation left under an older epoch is re-sealed for the push with
  exactly the plaintext it had (`vault::reseal_row`, since run 17; before,
  it went through the payload model, which drops top-level members it does
  not know; test `a_pending_record_of_a_later_version_is_resealed_byte_for_byte`).
  Writers keep the `data` members they do not know
  (`unknown_members_are_kept` and the like in crates/core/src/model). One
  path still goes through the payload model: when this device's pending
  copy goes over a newer server version (rule 0 of sync-protocol 6.4,
  `engine::apply_page`), it is re-sealed from its parsed payload, so a
  top-level member of a later version would be lost there; it needs a
  record of a later version pending on this device and a fork, and is
  recorded as debt (ROADMAP 6).
- **Classes and vaults.** Class codes `0x0B`..`0xFF` are reserved
  (crates/sync-api/src/classes.rs); the vault list's `kind` is a string, so
  a team vault in `GET /v1/vaults` does not break a 1.0 client, which syncs
  its personal vault only (one session, `records::check_vault`).
- **The protocol.** The server lists its API versions in `GET /v1/server`;
  a client refuses a server without `v1` as `unsupported_server`, which the
  app explains (sync_ops.rs, `server_check_with`). Later changes either stay
  within `v1` (new optional fields, new classes and types, which a 1.0
  client keeps) or come as a new API version beside it. Unknown JSON fields
  are ignored on both sides; a push status a client does not know reads as
  `ItemStatus::Unknown`, and since run 17 a registration mode it does not
  know reads as `Registration::Unknown` (treated as needing an invitation)
  instead of failing the whole server answer.
- **A gap against the specification:** storage-format 5.6 says an
  unsupported record is "shown as 'requires a newer version'"; the desktop
  keeps such records but shows nothing for them (the core's
  `Index::unsupported` list is not used by the interface). No data is at
  risk; a user of 1.0 just does not see that some records wait for a newer
  version. For the archive (new records in the personal vault) this is
  worth a line in the interface before the archive ships, or a change of
  the specification; the owner decides (run 17 report).

## Ready in stage 7

**Server (`server/`).**
- The schema of section 14 is complete, team tables included: `vaults`
  with `kind`, `vault_members` with roles, `vault_key_epochs`, `vault_key_wraps`
  (HPKE wraps per member and epoch) and `invitations`.
  Migrations create them on SQLite and PostgreSQL.
- Push acceptance (section 6.6) already checks membership, the role matrix
  of section 7.5 by class (`records::role_allows`, tested by
  `roles_by_class`), `class_not_allowed` for history in team vaults, the
  current epoch, and the quota rule "another account's quota never stops a
  member".
- `GET /v1/vaults`, `GET /v1/vaults/{id}/keys` and
  `POST /v1/vaults/{id}/key-epochs` exist; the last one is used today for the
  personal re-post and already takes the epoch-chain step `current + 1`.
- Writes lock the vault row on PostgreSQL (`lock_vault`), so concurrent
  pushes to one vault serialize; `seq` per vault is tested under concurrency.

**Client (`crates/core`).**
- The sync engine works on any vault: `apply_page`, `build_push`,
  `apply_push_results`, `resolve`, `push_back_missing` and `restore_mark`
  take the vault from the session's key ring; the `member_pin` type rule of
  section 6.7 is implemented and tested.
- Envelopes, epoch chains (`KeyRing::open_chain`, `ring_from_chain`) and
  the refusal of lower epochs (D93, D102) are vault-independent.
- `vaults.account_user_id` and `server_url` (schema 2) record the binding of
  every vault, as the archive of section 7.11 needs.
- `Selection::all()` is the selection of team vaults (they sync every class
  they may hold).

## Needed in stage 8

1. **HPKE wraps** (security.md 5.7): the `hpke` crate (exact version and audit
   status are still open, security.md section 15), the 77-byte `info`, wraps
   to each member's X25519 key; the client stores them as `hpke` key rows.
   The account's `user_keypair` record (D13) is written at registration
   already; reading it back into a key is the first step.
2. **One session per vault.** Today `Inner` holds one `Session` (the personal
   vault). Team vaults need a session (key ring, index, HLC) each, and the
   record APIs a vault id that is not always the personal one
   (`records::check_vault` refuses others today).
3. **Member keys and pins** (section 7.1): fingerprints, `member_pin`
   records, the verified flag, the epoch warning for an unverified sender.
4. **Invitations** (sections 7.3-7.4): the state machine on the server
   (tables exist, endpoints do not), the client's accept flow, and the
   interface of UI 6.x.
5. **Roster and rotations** (sections 7.6-7.10): `roster_entry` records and
   their type rule (missing in `engine::type_rule`, review finding of run 7),
   automatic rotation on member removal, background re-encryption across
   devices, `epoch_conflict` handling with refreshed epochs.
6. **Archives** (section 7.11): a removed member's read-only copy; the vault
   status `archived` exists in the local schema.
7. **The restore procedure for every bound vault**: `cycle` restores the
   personal vault only; section 6.9 asks for every bound vault, and the
   generation is stored only after all of them finished.
8. **Quota with team vaults**: the PostgreSQL lock is per vault, so two
   concurrent pushes to two vaults of one creator can overshoot a quota
   (server review of run 7); a per-user lock in the quota step closes it.
9. **Interface**: vault switcher, sharing dialog, member list, invitation
   inbox, the "server state rolled back" re-post for team vaults (asks the
   owner; a non-owner is told to ask one).

## Questions of run 7, closed

- Argon2 verification inside the database transaction (a burst of
  unauthenticated logins as a denial of service on SQLite): the checks run
  outside the transaction since run 9 (D174).
- `B64` and `Secret` of `crates/sync-api` not wiped on drop: they are wiped
  on drop since run 9 (D192).
