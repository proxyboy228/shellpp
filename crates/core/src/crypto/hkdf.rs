//! HKDF-SHA256 derivations (docs/security.md, section 5.5).
//!
//! Every call uses a zero-length salt (equivalent to 32 zero bytes), a 32-byte
//! output and `info` = ASCII label ‖ binary context.

use hkdf::Hkdf;
use sha2::Sha256;
use uuid::Uuid;

use super::keys::Key32;
use super::labels;

/// HKDF-SHA256 with an empty salt and a 32-byte output; `info` is `label`
/// followed by every element of `context`.
pub fn hkdf32(ikm: &Key32, label: &[u8], context: &[&[u8]]) -> Key32 {
    let hk = Hkdf::<Sha256>::new(None, ikm.expose());
    let mut info: Vec<&[u8]> = Vec::with_capacity(context.len() + 1);
    info.push(label);
    info.extend_from_slice(context);
    let mut out = Key32::zeroed();
    hk.expand_multi_info(&info, out.expose_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

/// `be16(len(origin)) ‖ origin` for the origin-bound derivations.
fn origin_context(origin: &str) -> Vec<u8> {
    let bytes = origin.as_bytes();
    // Origins are URLs of a few dozen bytes; a longer one is not a valid
    // origin and is truncated to the length the prefix can describe.
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    let mut out = Vec::with_capacity(2 + usize::from(len));
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&bytes[..usize::from(len)]);
    out
}

/// `kek` = HKDF(`master_key`, `"shellpp/v1/kek"`).
pub fn derive_kek(master_key: &Key32) -> Key32 {
    hkdf32(master_key, labels::KEK, &[])
}

/// `auth_key` = HKDF(`master_key`, `"shellpp/v1/auth"` ‖ be16(len(o)) ‖ o), where
/// `o` is the ASCII of the normalized origin of the sync server (stage 7).
pub fn derive_auth_key(master_key: &Key32, origin: &str) -> Key32 {
    hkdf32(master_key, labels::AUTH, &[&origin_context(origin)])
}

/// `recovery_kek` = HKDF(`recovery_key`, `"shellpp/v1/recovery-kek"`).
pub fn derive_recovery_kek(recovery_key: &Key32) -> Key32 {
    hkdf32(recovery_key, labels::RECOVERY_KEK, &[])
}

/// `recovery_auth_key` = HKDF(`recovery_key`, `"shellpp/v1/recovery-auth"` ‖ be16(len(o)) ‖ o)
/// (stage 7).
pub fn derive_recovery_auth_key(recovery_key: &Key32, origin: &str) -> Key32 {
    hkdf32(
        recovery_key,
        labels::RECOVERY_AUTH,
        &[&origin_context(origin)],
    )
}

/// `device_wrap_key` = HKDF(`device_key`, `"shellpp/v1/device-wrap"` ‖ db_id).
pub fn derive_device_wrap_key(device_key: &Key32, db_id: &Uuid) -> Key32 {
    hkdf32(device_key, labels::DEVICE_WRAP, &[db_id.as_bytes()])
}

/// `record_key[v, e]` = HKDF(`vault_key[v, e]`, `"shellpp/v1/record"` ‖ vault_id ‖ be32(e)).
pub fn derive_record_key(vault_key: &Key32, vault_id: &Uuid, epoch: u32) -> Key32 {
    hkdf32(
        vault_key,
        labels::RECORD,
        &[vault_id.as_bytes(), &epoch.to_be_bytes()],
    )
}

/// `chain_key[v, e]` = HKDF(`vault_key[v, e]`, `"shellpp/v1/epoch-chain"` ‖ vault_id ‖ be32(e)).
pub fn derive_chain_key(vault_key: &Key32, vault_id: &Uuid, epoch: u32) -> Key32 {
    hkdf32(
        vault_key,
        labels::EPOCH_CHAIN,
        &[vault_id.as_bytes(), &epoch.to_be_bytes()],
    )
}

/// `local_secrets_key` = HKDF(`vault_key[personal, e]`, `"shellpp/v1/local-secrets"` ‖
/// device_id ‖ be32(e)).
pub fn derive_local_secrets_key(vault_key: &Key32, device_id: &Uuid, epoch: u32) -> Key32 {
    hkdf32(
        vault_key,
        labels::LOCAL_SECRETS,
        &[device_id.as_bytes(), &epoch.to_be_bytes()],
    )
}

/// `recording_key[file]` = HKDF(`vault_key[personal, e]`, `"shellpp/v1/recording"` ‖ file_id)
/// (stage 6).
pub fn derive_recording_key(vault_key: &Key32, file_id: &Uuid) -> Key32 {
    hkdf32(vault_key, labels::RECORDING, &[file_id.as_bytes()])
}

/// `export_key` = HKDF(Argon2id output, `"shellpp/v1/export"`) (stage 3).
pub fn derive_export_key(argon2_output: &Key32) -> Key32 {
    hkdf32(argon2_output, labels::EXPORT, &[])
}

/// `epoch_commit[v, e]`: the first 16 bytes of HKDF(`vault_key[v, e]`,
/// `"shellpp/v1/roster-epoch"` ‖ vault_id ‖ be32(e)) (stage 8).
pub fn derive_epoch_commit(vault_key: &Key32, vault_id: &Uuid, epoch: u32) -> [u8; 16] {
    let full = hkdf32(
        vault_key,
        labels::ROSTER_EPOCH,
        &[vault_id.as_bytes(), &epoch.to_be_bytes()],
    );
    let mut out = [0u8; 16];
    out.copy_from_slice(&full.expose()[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_concatenated_after_the_label() {
        let ikm = Key32::from_bytes([7u8; 32]);
        let a = hkdf32(&ikm, b"shellpp/v1/x", &[b"ab", b"cd"]);
        let b = hkdf32(&ikm, b"shellpp/v1/xab", &[b"cd"]);
        let c = hkdf32(&ikm, b"shellpp/v1/xabcd", &[]);
        assert!(a.ct_eq(&b));
        assert!(a.ct_eq(&c));
    }

    #[test]
    fn origin_context_layout() {
        assert_eq!(origin_context(""), vec![0, 0]);
        assert_eq!(
            origin_context("https://sync.example.com"),
            [&[0u8, 24][..], b"https://sync.example.com"].concat()
        );
    }

    #[test]
    fn derivations_are_independent() {
        let ikm = Key32::from_bytes([9u8; 32]);
        let id = Uuid::from_u128(1);
        let keys = [
            derive_kek(&ikm),
            derive_auth_key(&ikm, "https://sync.example.com"),
            derive_recovery_kek(&ikm),
            derive_recovery_auth_key(&ikm, "https://sync.example.com"),
            derive_device_wrap_key(&ikm, &id),
            derive_record_key(&ikm, &id, 1),
            derive_record_key(&ikm, &id, 2),
            derive_chain_key(&ikm, &id, 1),
            derive_local_secrets_key(&ikm, &id, 1),
            derive_recording_key(&ikm, &id),
            derive_export_key(&ikm),
        ];
        for (i, a) in keys.iter().enumerate() {
            for b in &keys[i + 1..] {
                assert!(!a.ct_eq(b));
            }
        }
    }
}
