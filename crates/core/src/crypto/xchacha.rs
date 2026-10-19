//! Thin wrapper over XChaCha20-Poly1305 (`chacha20poly1305`): 32-byte key,
//! 24-byte nonce, 16-byte tag appended to the ciphertext. Plaintext buffers
//! are allocated once at their final size and zeroized.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Key, Tag, XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::CryptoError;
use super::keys::Key32;

/// Nonce length.
pub const NONCE_LEN: usize = 24;
/// Tag length.
pub const TAG_LEN: usize = 16;

fn cipher(key: &Key32) -> XChaCha20Poly1305 {
    let key: &Key = key
        .expose()
        .as_slice()
        .try_into()
        .expect("a Key32 is 32 bytes");
    XChaCha20Poly1305::new(key)
}

fn nonce_ref(nonce: &[u8]) -> Result<&XNonce, CryptoError> {
    nonce
        .try_into()
        .map_err(|_| CryptoError::Malformed("nonce has the wrong length"))
}

/// Encrypts `plaintext` and appends ciphertext ‖ tag to `out`.
pub(crate) fn seal_append(
    key: &Key32,
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), CryptoError> {
    let nonce = nonce_ref(nonce)?;
    // Reserve first so the buffer holding plaintext is never reallocated.
    out.reserve(plaintext.len() + TAG_LEN);
    let start = out.len();
    out.extend_from_slice(plaintext);
    let tag = cipher(key)
        .encrypt_inout_detached(nonce, aad, out[start..].as_mut().into())
        .map_err(|_| CryptoError::Aead)?;
    out.extend_from_slice(tag.as_slice());
    Ok(())
}

/// Decrypts ciphertext ‖ tag. Any authentication failure is [`CryptoError::Aead`].
pub(crate) fn open(
    key: &Key32,
    nonce: &[u8],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let nonce = nonce_ref(nonce)?;
    let Some(msg_len) = ciphertext_and_tag.len().checked_sub(TAG_LEN) else {
        return Err(CryptoError::Malformed("ciphertext shorter than the tag"));
    };
    let (ciphertext, tag) = ciphertext_and_tag.split_at(msg_len);
    let tag: &Tag = tag.try_into().expect("tag slice is 16 bytes");
    let mut buf = Zeroizing::new(Vec::with_capacity(msg_len));
    buf.extend_from_slice(ciphertext);
    cipher(key)
        .decrypt_inout_detached(nonce, aad, buf.as_mut_slice().into(), tag)
        .map_err(|_| CryptoError::Aead)?;
    Ok(buf)
}
