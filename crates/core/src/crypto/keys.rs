//! Zeroizing key types (docs/security.md, section 5.3: all key material is held
//! in `zeroize` / `secrecy` types and wiped on drop).

use std::fmt;

use secrecy::{ExposeSecret, SecretBox};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::CryptoError;

/// A 32-byte secret key.
///
/// The bytes live on the heap (so moves never leave copies on the stack) and
/// are zeroized on drop. `Debug` prints `Key32(..)`. The type is deliberately
/// not `Clone`; use [`Key32::duplicate`] when a second owner is really needed.
pub struct Key32 {
    bytes: Box<[u8; 32]>,
}

impl Key32 {
    /// A key of 32 zero bytes, to be filled in place by the crate.
    pub(crate) fn zeroed() -> Self {
        Self {
            bytes: Box::new([0u8; 32]),
        }
    }

    /// Takes the bytes of an existing key and wipes the array passed in.
    /// Prefer [`Key32::from_slice`] on a zeroizing buffer: a `[u8; 32]`
    /// value is copied whenever it moves, and only this copy is wiped here.
    pub fn from_bytes(mut bytes: [u8; 32]) -> Self {
        let mut key = Self::zeroed();
        key.bytes.copy_from_slice(&bytes);
        bytes.zeroize();
        key
    }

    /// Copies a 32-byte slice into a new key; `None` for any other length.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 32 {
            return None;
        }
        let mut key = Self::zeroed();
        key.bytes.copy_from_slice(bytes);
        Some(key)
    }

    /// A new random key from the OS CSPRNG.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut key = Self::zeroed();
        getrandom::fill(key.expose_mut()).map_err(|_| CryptoError::Rng)?;
        Ok(key)
    }

    /// The key bytes. Do not copy them into types that are not zeroized.
    pub fn expose(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Mutable access for in-place derivation (KDF, HKDF, unwrap).
    pub(crate) fn expose_mut(&mut self) -> &mut [u8; 32] {
        &mut self.bytes
    }

    /// A second, independent copy of the key.
    pub fn duplicate(&self) -> Self {
        let mut key = Self::zeroed();
        key.bytes.copy_from_slice(&self.bytes[..]);
        key
    }

    /// Constant-time comparison of two keys.
    pub fn ct_eq(&self, other: &Self) -> bool {
        super::random::ct_eq(&self.bytes[..], &other.bytes[..])
    }

    /// Copies the key into a `secrecy` box (for example for a `SecretStore`).
    pub fn to_secret_box(&self) -> SecretBox<[u8; 32]> {
        SecretBox::init_with_mut(|buf: &mut [u8; 32]| buf.copy_from_slice(&self.bytes[..]))
    }

    /// Copies the key out of a `secrecy` box.
    pub fn from_secret_box(secret: &SecretBox<[u8; 32]>) -> Self {
        let mut key = Self::zeroed();
        key.bytes.copy_from_slice(&secret.expose_secret()[..]);
        key
    }
}

impl Drop for Key32 {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl ZeroizeOnDrop for Key32 {}

impl fmt::Debug for Key32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key32(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let key = Key32::from_bytes([0x5a; 32]);
        assert_eq!(format!("{key:?}"), "Key32(..)");
        assert_eq!(format!("{:?}", Some(key)), "Some(Key32(..))");
    }

    #[test]
    fn duplicate_and_secret_box_round_trip() {
        let key = Key32::generate().unwrap();
        let copy = key.duplicate();
        assert!(key.ct_eq(&copy));
        assert_eq!(key.expose(), copy.expose());
        let boxed = key.to_secret_box();
        assert_eq!(boxed.expose_secret(), key.expose());
        let back = Key32::from_secret_box(&boxed);
        assert!(back.ct_eq(&key));
        let other = Key32::generate().unwrap();
        assert!(!other.ct_eq(&key));
    }

    #[test]
    fn from_slice_checks_length() {
        assert!(Key32::from_slice(&[1u8; 31]).is_none());
        assert!(Key32::from_slice(&[1u8; 33]).is_none());
        assert_eq!(Key32::from_slice(&[1u8; 32]).unwrap().expose(), &[1u8; 32]);
    }
}
