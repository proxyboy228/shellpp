//! Every HKDF, hash and AAD label of the client (docs/security.md, sections 5.5
//! and 5.8; docs/storage-format.md, sections 5.2, 6.2 and 6.4). Labels are
//! ASCII bytes without a terminator and are frozen once users have data.

/// Prefix of the Argon2id salt hash (`argon2_salt`).
pub const KDF_SALT: &[u8] = b"shellpp/v1/kdf-salt";
/// `kek` = HKDF(`master_key`, this).
pub const KEK: &[u8] = b"shellpp/v1/kek";
/// `auth_key` = HKDF(`master_key`, this ‖ be16(len(origin)) ‖ origin).
pub const AUTH: &[u8] = b"shellpp/v1/auth";
/// `recovery_kek` = HKDF(`recovery_key`, this).
pub const RECOVERY_KEK: &[u8] = b"shellpp/v1/recovery-kek";
/// `recovery_auth_key` = HKDF(`recovery_key`, this ‖ be16(len(origin)) ‖ origin).
pub const RECOVERY_AUTH: &[u8] = b"shellpp/v1/recovery-auth";
/// `device_wrap_key` = HKDF(`device_key`, this ‖ db_id).
pub const DEVICE_WRAP: &[u8] = b"shellpp/v1/device-wrap";
/// `record_key[v, e]` = HKDF(`vault_key[v, e]`, this ‖ vault_id ‖ be32(e)); also the
/// envelope AAD prefix.
pub const RECORD: &[u8] = b"shellpp/v1/record";
/// `chain_key[v, e]` = HKDF(`vault_key[v, e]`, this ‖ vault_id ‖ be32(e)).
pub const EPOCH_CHAIN: &[u8] = b"shellpp/v1/epoch-chain";
/// `epoch_commit[v, e]` = first 16 bytes of HKDF(`vault_key[v, e]`, this ‖ vault_id ‖ be32(e)).
pub const ROSTER_EPOCH: &[u8] = b"shellpp/v1/roster-epoch";
/// `local_secrets_key` = HKDF(`vault_key[personal, e]`, this ‖ device_id ‖ be32(e)).
pub const LOCAL_SECRETS: &[u8] = b"shellpp/v1/local-secrets";
/// `recording_key[file]` = HKDF(`vault_key[personal, e]`, this ‖ file_id).
pub const RECORDING: &[u8] = b"shellpp/v1/recording";
/// `export_key` = HKDF(Argon2id(export password, salt), this).
pub const EXPORT: &[u8] = b"shellpp/v1/export";
/// AAD prefix of the `kek` wrap.
pub const WRAP_KEK: &[u8] = b"shellpp/v1/wrap/kek";
/// AAD prefix of the `recovery` wrap.
pub const WRAP_RECOVERY: &[u8] = b"shellpp/v1/wrap/recovery";
/// AAD prefix of the `device` wrap.
pub const WRAP_DEVICE: &[u8] = b"shellpp/v1/wrap/device";
/// AAD prefix of the `chain` wrap.
pub const WRAP_EPOCH_CHAIN: &[u8] = b"shellpp/v1/wrap/epoch-chain";
/// AAD prefix of the wrapped refresh token.
pub const WRAP_REFRESH_TOKEN: &[u8] = b"shellpp/v1/wrap/refresh-token";
/// HPKE `info` prefix of a team vault key wrap.
pub const HPKE_VAULT_KEY: &[u8] = b"shellpp/v1/hpke/vault-key";
/// Prefix of the identity fingerprint hash.
pub const IDENTITY_FP: &[u8] = b"shellpp/v1/identity-fp";

/// Every label, for tests that check the shared prefix and uniqueness.
pub const ALL: &[&[u8]] = &[
    KDF_SALT,
    KEK,
    AUTH,
    RECOVERY_KEK,
    RECOVERY_AUTH,
    DEVICE_WRAP,
    RECORD,
    EPOCH_CHAIN,
    ROSTER_EPOCH,
    LOCAL_SECRETS,
    RECORDING,
    EXPORT,
    WRAP_KEK,
    WRAP_RECOVERY,
    WRAP_DEVICE,
    WRAP_EPOCH_CHAIN,
    WRAP_REFRESH_TOKEN,
    HPKE_VAULT_KEY,
    IDENTITY_FP,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_share_the_prefix_and_are_unique() {
        for (i, label) in ALL.iter().enumerate() {
            assert!(label.starts_with(crate::ids::LABEL_PREFIX.as_bytes()));
            assert!(label.is_ascii());
            for other in &ALL[i + 1..] {
                assert_ne!(label, other);
            }
        }
        assert_eq!(RECORD.len(), 17);
        assert_eq!(HPKE_VAULT_KEY.len(), 25);
    }
}
