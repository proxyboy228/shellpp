//! Cryptographic constructions: Argon2id, HKDF-SHA256, XChaCha20-Poly1305 wrap
//! blobs and record envelopes, padding, the recovery key text format and
//! zeroizing key types (docs/security.md, section 5; docs/storage-format.md,
//! sections 5, 6 and 14).
//!
//! Only the crates `argon2`, `hkdf`, `sha2`, `chacha20poly1305`, `zeroize`,
//! `secrecy`, `getrandom` and `subtle` are used. No homemade cryptography.
//!
//! Owner: W1. Public API: see the stage 1 plan, section "crypto".

mod envelope;
mod export;
mod hkdf;
mod kdf;
mod keys;
pub mod labels;
mod random;
mod recovery;
mod wrap;
mod xchacha;

pub use envelope::{
    MAX_PAYLOAD_LEN, RECORD_AAD_LEN, open_envelope, pad, padded_len, record_aad, seal_envelope,
    unpad,
};
pub use export::{
    EXPORT_AAD_LEN, EXPORT_NONCE_LEN, EXPORT_SALT_LEN, SealedExport, export_aad, open_export,
    seal_export,
};
pub use hkdf::{
    derive_auth_key, derive_chain_key, derive_device_wrap_key, derive_epoch_commit,
    derive_export_key, derive_kek, derive_local_secrets_key, derive_record_key,
    derive_recording_key, derive_recovery_auth_key, derive_recovery_kek, hkdf32,
};
pub use kdf::{
    ALG_ARGON2ID, KDF_SALT_LEN, KDF_VERSION_1, KdfParams, MIN_PASSWORD_CHARS, argon2_salt,
    derive_master_key, derive_with_salt, normalize_email, normalize_password, password_length_ok,
};
pub use keys::Key32;
pub use random::{ct_eq, random_array, sha256, sha256_parts};
pub use recovery::{
    CROCKFORD_ALPHABET, RECOVERY_KEY_CHARS, RECOVERY_KEY_PREFIX, RECOVERY_KEY_TEXT_LEN,
    decode_recovery_key, encode_recovery_key,
};
pub use wrap::{
    WRAP_NONCE_LEN, WRAP_OVERHEAD, WRAP_VERSION, WRAPPED_KEY_LEN, WrapPurpose, refresh_token_aad,
    unwrap_key, vault_wrap_aad, wrap_key, wrap_open, wrap_seal,
};
/// XChaCha20-Poly1305 for the recordings' chunks (`crate::recording`).
pub(crate) use xchacha::{open as xchacha_open, seal_append as xchacha_seal_append};

use shellpp_sync_api::envelope::EnvelopeError;

/// Platform class used for the KDF presets and ceilings (docs/security.md, section 5.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformClass {
    Desktop,
    Mobile,
}

/// Errors of the cryptographic layer. Never carries key material or plaintext.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// AEAD authentication failed: wrong key, or tampered nonce, AAD or ciphertext.
    #[error("authentication failed")]
    Aead,
    /// KDF parameters outside the floor or the platform ceiling, or an unknown `alg` or `v`.
    #[error("KDF parameters rejected: {0}")]
    KdfParams(&'static str),
    /// Argon2id failed (for example memory allocation).
    #[error("key derivation failed")]
    Kdf,
    /// The OS random number generator failed.
    #[error("random number generator failed")]
    Rng,
    /// A blob, envelope or padding has the wrong layout or version.
    #[error("malformed data: {0}")]
    Malformed(&'static str),
    /// The payload exceeds 262 143 bytes (docs/storage-format.md, section 5.3).
    #[error("payload too large")]
    TooLarge,
    /// The recovery key text has a wrong length, alphabet or checksum.
    #[error("malformed recovery key")]
    RecoveryKeyFormat,
    /// An envelope failed checks 1-3 of docs/storage-format.md, section 5.4.
    /// `UnknownVersion` means "written by a newer app": keep the row, never
    /// rewrite it, show the record as unsupported.
    #[error("envelope rejected: {0}")]
    Envelope(EnvelopeError),
}

impl CryptoError {
    /// Stable error code (docs/architecture.md, section 3.4).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Aead | Self::Malformed(_) | Self::Envelope(_) => "damaged",
            Self::TooLarge => "too_large",
            Self::RecoveryKeyFormat => "validation_failed",
            Self::KdfParams(_) | Self::Kdf | Self::Rng => "internal",
        }
    }
}

/// Deterministic variants of the sealing functions for known-answer tests.
///
/// **Never use these outside tests**: every real encryption must use a fresh
/// random nonce (docs/security.md, section 5.6). They exist only so the
/// integration tests in `crates/core/tests` can check exact output bytes.
#[doc(hidden)]
pub mod testing {
    use shellpp_sync_api::envelope::EnvelopeHeader;

    use super::{CryptoError, Key32};

    /// [`super::wrap_seal`] with a caller-chosen nonce. Tests only.
    pub fn wrap_seal_with_fixed_nonce(
        key: &Key32,
        aad: &[u8],
        plaintext: &[u8],
        nonce: &[u8; 24],
    ) -> Result<Vec<u8>, CryptoError> {
        super::wrap::wrap_seal_with_nonce(key, aad, plaintext, nonce)
    }

    /// [`super::seal_envelope`] with a caller-chosen nonce. Tests only.
    pub fn seal_envelope_with_fixed_nonce(
        record_key: &Key32,
        header: &EnvelopeHeader,
        payload_json: &[u8],
        nonce: &[u8; 24],
    ) -> Result<Vec<u8>, CryptoError> {
        super::envelope::seal_envelope_with_nonce(record_key, header, payload_json, nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_data_free() {
        assert_eq!(CryptoError::Aead.code(), "damaged");
        assert_eq!(
            CryptoError::Envelope(EnvelopeError::UnknownVersion(2)).code(),
            "damaged"
        );
        assert_eq!(CryptoError::TooLarge.code(), "too_large");
        assert_eq!(CryptoError::RecoveryKeyFormat.code(), "validation_failed");
        assert_eq!(CryptoError::Rng.code(), "internal");
        assert_eq!(
            CryptoError::Envelope(EnvelopeError::BadLength(5)).to_string(),
            "envelope rejected: bad envelope length 5"
        );
    }
}
