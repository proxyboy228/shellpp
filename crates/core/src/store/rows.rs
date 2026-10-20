//! Typed rows of the local database (docs/storage-format.md, sections 4.2-4.6).
//!
//! UUIDs are 16-byte BLOBs, booleans INTEGER 0/1, timestamps RFC 3339 text
//! produced by the caller.

use std::fmt;

use uuid::Uuid;

use crate::service::dto::{Role, VaultKind, VaultStatus};

use super::StoreError;

/// The single `local_auth` row (id = 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalAuthRow {
    /// Id of the personal vault.
    pub personal_vault_id: Uuid,
    /// KDF JSON (`crate::crypto::KdfParams::to_json`).
    pub kdf_json: String,
    /// 16 random bytes; the Argon2id salt is derived from it.
    pub kdf_salt: [u8; 16],
    /// Normalized email bound into the Argon2id salt; `None` until first bound.
    pub kdf_email: Option<String>,
    /// RFC 3339 UTC timestamp of the last change.
    pub updated_at: String,
}

/// A `vaults` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultRow {
    pub vault_id: Uuid,
    pub kind: VaultKind,
    pub status: VaultStatus,
    /// `None` for the personal vault.
    pub role: Option<Role>,
    pub current_epoch: u32,
    /// True while the vault syncs with the bound account.
    pub bound: bool,
    pub created_at: String,
}

/// The `kind` column of `key_wraps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WrapKind {
    Kek,
    Recovery,
    Device,
    Hpke,
    Chain,
}

impl WrapKind {
    /// The text stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kek => "kek",
            Self::Recovery => "recovery",
            Self::Device => "device",
            Self::Hpke => "hpke",
            Self::Chain => "chain",
        }
    }

    /// Parses the database text.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "kek" => Self::Kek,
            "recovery" => Self::Recovery,
            "device" => Self::Device,
            "hpke" => Self::Hpke,
            "chain" => Self::Chain,
            _ => return None,
        })
    }
}

/// A `key_wraps` row. `Debug` prints the lengths of the blobs only: a `kek`
/// wrap together with `local_auth` is enough to guess the master password
/// offline.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyWrapRow {
    pub vault_id: Uuid,
    /// The epoch of the wrapped key; for `chain` the newer epoch `e` whose
    /// chain key wraps `e - 1` (docs/storage-format.md, section 6.2).
    pub epoch: u32,
    pub kind: WrapKind,
    /// `hpke` only.
    pub sender_user_id: Option<Uuid>,
    /// `hpke` only, 32 bytes.
    pub hpke_enc: Option<Vec<u8>>,
    /// Wrap blob, or the HPKE ciphertext for `hpke`.
    pub wrap: Vec<u8>,
}

impl fmt::Debug for KeyWrapRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyWrapRow")
            .field("vault_id", &self.vault_id)
            .field("epoch", &self.epoch)
            .field("kind", &self.kind)
            .field("sender_user_id", &self.sender_user_id)
            .field("hpke_enc_len", &self.hpke_enc.as_ref().map(Vec::len))
            .field("wrap_len", &self.wrap.len())
            .finish()
    }
}

/// A `records` row. The envelope is authoritative; the other columns are
/// copies of its header plus sync bookkeeping (docs/storage-format.md, section 4.6).
#[derive(Clone, PartialEq, Eq)]
pub struct RecordRow {
    pub vault_id: Uuid,
    pub record_id: Uuid,
    pub class: u8,
    pub revision: u64,
    pub synced_revision: u64,
    pub key_epoch: u32,
    pub tombstone: bool,
    pub tombstoned_at: Option<String>,
    pub server_seq: Option<i64>,
    pub envelope: Vec<u8>,
    pub base_envelope: Option<Vec<u8>>,
}

impl fmt::Debug for RecordRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordRow")
            .field("vault_id", &self.vault_id)
            .field("record_id", &self.record_id)
            .field("class", &self.class)
            .field("revision", &self.revision)
            .field("synced_revision", &self.synced_revision)
            .field("key_epoch", &self.key_epoch)
            .field("tombstone", &self.tombstone)
            .field("envelope_len", &self.envelope.len())
            .field(
                "base_envelope_len",
                &self.base_envelope.as_ref().map(Vec::len),
            )
            .finish_non_exhaustive()
    }
}

pub(crate) fn vault_kind_str(kind: VaultKind) -> &'static str {
    match kind {
        VaultKind::Personal => "personal",
        VaultKind::Team => "team",
    }
}

pub(crate) fn parse_vault_kind(text: &str) -> Result<VaultKind, StoreError> {
    match text {
        "personal" => Ok(VaultKind::Personal),
        "team" => Ok(VaultKind::Team),
        _ => Err(StoreError::Corrupt("unknown vault kind")),
    }
}

pub(crate) fn vault_status_str(status: VaultStatus) -> &'static str {
    match status {
        VaultStatus::Active => "active",
        VaultStatus::Archived => "archived",
    }
}

pub(crate) fn parse_vault_status(text: &str) -> Result<VaultStatus, StoreError> {
    match text {
        "active" => Ok(VaultStatus::Active),
        "archived" => Ok(VaultStatus::Archived),
        _ => Err(StoreError::Corrupt("unknown vault status")),
    }
}

pub(crate) fn role_str(role: Role) -> &'static str {
    match role {
        Role::Owner => "owner",
        Role::Editor => "editor",
        Role::Viewer => "viewer",
    }
}

pub(crate) fn parse_role(text: &str) -> Result<Role, StoreError> {
    match text {
        "owner" => Ok(Role::Owner),
        "editor" => Ok(Role::Editor),
        "viewer" => Ok(Role::Viewer),
        _ => Err(StoreError::Corrupt("unknown role")),
    }
}

pub(crate) fn uuid_from_blob(blob: &[u8]) -> Result<Uuid, StoreError> {
    Uuid::from_slice(blob).map_err(|_| StoreError::Corrupt("UUID column is not 16 bytes"))
}

pub(crate) fn u32_from_db(value: i64, what: &'static str) -> Result<u32, StoreError> {
    u32::try_from(value).map_err(|_| StoreError::Corrupt(what))
}

pub(crate) fn u64_from_db(value: i64, what: &'static str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(what))
}

pub(crate) fn u64_to_db(value: u64, what: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Corrupt(what))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_texts_round_trip() {
        for kind in [
            WrapKind::Kek,
            WrapKind::Recovery,
            WrapKind::Device,
            WrapKind::Hpke,
            WrapKind::Chain,
        ] {
            assert_eq!(WrapKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(WrapKind::parse("KEK"), None);
        for kind in [VaultKind::Personal, VaultKind::Team] {
            assert_eq!(parse_vault_kind(vault_kind_str(kind)).unwrap(), kind);
        }
        for status in [VaultStatus::Active, VaultStatus::Archived] {
            assert_eq!(
                parse_vault_status(vault_status_str(status)).unwrap(),
                status
            );
        }
        for role in [Role::Owner, Role::Editor, Role::Viewer] {
            assert_eq!(parse_role(role_str(role)).unwrap(), role);
        }
        assert!(parse_role("admin").is_err());
    }

    #[test]
    fn record_debug_has_no_envelope_bytes() {
        let row = RecordRow {
            vault_id: Uuid::from_u128(1),
            record_id: Uuid::from_u128(2),
            class: 2,
            revision: 3,
            synced_revision: 0,
            key_epoch: 1,
            tombstone: false,
            tombstoned_at: None,
            server_seq: None,
            envelope: vec![0xEE; 344],
            base_envelope: None,
        };
        let text = format!("{row:?}");
        assert!(text.contains("envelope_len: 344"));
        assert!(!text.contains("238")); // 0xEE
    }

    #[test]
    fn key_wrap_debug_has_no_blob_bytes() {
        let row = KeyWrapRow {
            vault_id: Uuid::from_u128(1),
            epoch: 4,
            kind: WrapKind::Hpke,
            sender_user_id: Some(Uuid::from_u128(3)),
            hpke_enc: Some(vec![0xDD; 32]),
            wrap: vec![0xEE; 73],
        };
        let text = format!("{row:?}");
        assert!(text.contains("epoch: 4"));
        assert!(text.contains("kind: Hpke"));
        assert!(text.contains("hpke_enc_len: Some(32)"));
        assert!(text.contains("wrap_len: 73"));
        assert!(!text.contains("221")); // 0xDD
        assert!(!text.contains("238")); // 0xEE
    }
}
