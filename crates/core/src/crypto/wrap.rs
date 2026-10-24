//! Wrap blobs, version 1 (docs/storage-format.md, sections 6.1 and 6.2).
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 1 | `version` = `0x01` |
//! | 1 | 24 | `nonce`, random |
//! | 25 | n + 16 | ciphertext ‖ tag (XChaCha20-Poly1305) |
//!
//! A wrapped 32-byte key is 73 bytes.

use uuid::Uuid;
use zeroize::Zeroizing;

use super::keys::Key32;
use super::labels;
use super::xchacha;
use super::{CryptoError, random};

/// Version byte of a wrap blob.
pub const WRAP_VERSION: u8 = 0x01;
/// Nonce length inside a wrap blob.
pub const WRAP_NONCE_LEN: usize = xchacha::NONCE_LEN;
/// Version byte, nonce and tag.
pub const WRAP_OVERHEAD: usize = 1 + WRAP_NONCE_LEN + xchacha::TAG_LEN;
/// Length of a wrapped 32-byte key.
pub const WRAPPED_KEY_LEN: usize = WRAP_OVERHEAD + 32;

const _: () = assert!(WRAPPED_KEY_LEN == 73);

/// Seals `plaintext` under `key` with `aad` and a fresh random nonce.
pub fn wrap_seal(key: &Key32, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let nonce: [u8; WRAP_NONCE_LEN] = random::random_array()?;
    wrap_seal_with_nonce(key, aad, plaintext, &nonce)
}

pub(crate) fn wrap_seal_with_nonce(
    key: &Key32,
    aad: &[u8],
    plaintext: &[u8],
    nonce: &[u8; WRAP_NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    let mut out = Vec::with_capacity(WRAP_OVERHEAD + plaintext.len());
    out.push(WRAP_VERSION);
    out.extend_from_slice(nonce);
    xchacha::seal_append(key, nonce, aad, plaintext, &mut out)?;
    Ok(out)
}

/// Opens a wrap blob. Fails with [`CryptoError::Malformed`] for a wrong
/// version or a blob too short to hold a tag, and [`CryptoError::Aead`] for a
/// wrong key, AAD or a modified blob.
pub fn wrap_open(key: &Key32, aad: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let Some(&version) = blob.first() else {
        return Err(CryptoError::Malformed("empty wrap blob"));
    };
    if version != WRAP_VERSION {
        return Err(CryptoError::Malformed("unknown wrap blob version"));
    }
    if blob.len() < WRAP_OVERHEAD {
        return Err(CryptoError::Malformed("wrap blob too short"));
    }
    let nonce = &blob[1..1 + WRAP_NONCE_LEN];
    xchacha::open(key, nonce, aad, &blob[1 + WRAP_NONCE_LEN..])
}

/// Wraps a 32-byte key: a 73-byte blob.
pub fn wrap_key(wrapping_key: &Key32, aad: &[u8], key: &Key32) -> Result<Vec<u8>, CryptoError> {
    wrap_seal(wrapping_key, aad, key.expose())
}

/// Unwraps a 32-byte key from a 73-byte blob.
pub fn unwrap_key(wrapping_key: &Key32, aad: &[u8], blob: &[u8]) -> Result<Key32, CryptoError> {
    if blob.first().is_some_and(|&v| v == WRAP_VERSION) && blob.len() != WRAPPED_KEY_LEN {
        return Err(CryptoError::Malformed("wrapped key has the wrong length"));
    }
    let plain = wrap_open(wrapping_key, aad, blob)?;
    Key32::from_slice(&plain).ok_or(CryptoError::Malformed("wrapped key has the wrong length"))
}

/// The slot a vault key wrap belongs to (docs/storage-format.md, section 6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapPurpose {
    /// `kek` row: wrapped by the key derived from the master password.
    Kek,
    /// `recovery` row: wrapped by the key derived from the recovery key.
    Recovery,
    /// `device` row: wrapped by the key derived from the device key.
    Device,
    /// `chain` row: `vault_key[v, e - 1]` wrapped by `chain_key[v, e]`.
    EpochChain,
}

impl WrapPurpose {
    /// The AAD label of this slot.
    pub fn label(self) -> &'static [u8] {
        match self {
            Self::Kek => labels::WRAP_KEK,
            Self::Recovery => labels::WRAP_RECOVERY,
            Self::Device => labels::WRAP_DEVICE,
            Self::EpochChain => labels::WRAP_EPOCH_CHAIN,
        }
    }
}

/// AAD of a vault key wrap: label ‖ vault_id (16) ‖ be32(epoch).
///
/// `epoch` is the epoch of the wrapped key: for [`WrapPurpose::EpochChain`]
/// pass `e - 1`, where `e` is the row's `epoch` column.
pub fn vault_wrap_aad(purpose: WrapPurpose, vault_id: &Uuid, epoch: u32) -> Vec<u8> {
    let label = purpose.label();
    let mut aad = Vec::with_capacity(label.len() + 16 + 4);
    aad.extend_from_slice(label);
    aad.extend_from_slice(vault_id.as_bytes());
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad
}

/// AAD of the wrapped refresh token: `"shellpp/v1/wrap/refresh-token"` ‖
/// device_id ‖ be16(len(origin)) ‖ origin (stage 7; the origin since run 9,
/// D188): the token opens only for the normalized server origin that issued
/// it, so a changed server address never receives it.
pub fn refresh_token_aad(device_id: &Uuid, origin: &str) -> Vec<u8> {
    let origin = super::hkdf::origin_context(origin);
    let mut aad = Vec::with_capacity(labels::WRAP_REFRESH_TOKEN.len() + 16 + origin.len());
    aad.extend_from_slice(labels::WRAP_REFRESH_TOKEN);
    aad.extend_from_slice(device_id.as_bytes());
    aad.extend_from_slice(&origin);
    aad
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_round_trip_and_layout() {
        let kek = Key32::generate().unwrap();
        let key = Key32::generate().unwrap();
        let id = Uuid::from_u128(42);
        let aad = vault_wrap_aad(WrapPurpose::Kek, &id, 1);
        let blob = wrap_key(&kek, &aad, &key).unwrap();
        assert_eq!(blob.len(), WRAPPED_KEY_LEN);
        assert_eq!(blob[0], WRAP_VERSION);
        assert!(unwrap_key(&kek, &aad, &blob).unwrap().ct_eq(&key));

        // Fresh nonce every time.
        let again = wrap_key(&kek, &aad, &key).unwrap();
        assert_ne!(blob[1..25], again[1..25]);
    }

    #[test]
    fn wrong_key_aad_or_tampering_fails() {
        let kek = Key32::generate().unwrap();
        let key = Key32::generate().unwrap();
        let id = Uuid::from_u128(42);
        let aad = vault_wrap_aad(WrapPurpose::Recovery, &id, 3);
        let blob = wrap_key(&kek, &aad, &key).unwrap();

        let other = Key32::generate().unwrap();
        assert!(matches!(
            unwrap_key(&other, &aad, &blob),
            Err(CryptoError::Aead)
        ));
        for wrong in [
            vault_wrap_aad(WrapPurpose::Kek, &id, 3),
            vault_wrap_aad(WrapPurpose::Recovery, &id, 2),
            vault_wrap_aad(WrapPurpose::Recovery, &Uuid::from_u128(43), 3),
        ] {
            assert!(matches!(
                unwrap_key(&kek, &wrong, &blob),
                Err(CryptoError::Aead)
            ));
        }
        for i in 1..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            assert!(matches!(
                unwrap_key(&kek, &aad, &bad),
                Err(CryptoError::Aead)
            ));
        }
        let mut bad = blob.clone();
        bad[0] = 0x02;
        assert!(matches!(
            unwrap_key(&kek, &aad, &bad),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            unwrap_key(&kek, &aad, &blob[..72]),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            wrap_open(&kek, &aad, &blob[..40]),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            wrap_open(&kek, &aad, &[]),
            Err(CryptoError::Malformed(_))
        ));
    }

    #[test]
    fn small_secrets() {
        let k = Key32::generate().unwrap();
        let aad = refresh_token_aad(&Uuid::from_u128(7), "https://sync.example.com");
        let token = b"shellpp_rt_0123456789abcdefghijABCDEFGHIJ0123456789-_x";
        assert_eq!(token.len(), 54);
        let blob = wrap_seal(&k, &aad, token).unwrap();
        assert_eq!(blob.len(), 95);
        assert_eq!(&wrap_open(&k, &aad, &blob).unwrap()[..], &token[..]);
        let empty = wrap_seal(&k, &aad, b"").unwrap();
        assert_eq!(empty.len(), WRAP_OVERHEAD);
        assert!(wrap_open(&k, &aad, &empty).unwrap().is_empty());
    }

    #[test]
    fn aad_layout() {
        let id = Uuid::from_bytes([0xAB; 16]);
        let aad = vault_wrap_aad(WrapPurpose::EpochChain, &id, 0x0102_0304);
        let mut expected = b"shellpp/v1/wrap/epoch-chain".to_vec();
        expected.extend_from_slice(&[0xAB; 16]);
        expected.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(aad, expected);
        assert_eq!(WrapPurpose::Device.label(), b"shellpp/v1/wrap/device");
    }
}
