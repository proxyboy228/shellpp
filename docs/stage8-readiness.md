# Stage 8 readiness: what stage 7 leaves for team vaults

Written at the end of run 7 (2026-10-23, plan section 6.3). No stage 8 code
exists; this note lists what the stage 7 code already does for team vaults
(docs/sync-protocol.md, section 7; D13, D14) and what stage 8 still needs.

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

## Open questions for the owner (from run 7)

- Argon2 verification inside the database transaction makes a burst of
  unauthenticated logins a denial of service on SQLite (one connection);
  moving the verification out of the transaction is a server refactor of
  its own (auth review, finding 2).
- `B64` and `Secret` of `crates/sync-api` are not wiped on drop; wrapping
  them in zeroizing types touches every request type on both sides (crypto
  review, finding 2).
