//! OS randomness, SHA-256 and constant-time comparison.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::CryptoError;

/// `N` random bytes from the OS CSPRNG (`getrandom`).
pub fn random_array<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|_| CryptoError::Rng)?;
    Ok(out)
}

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// SHA-256 of the concatenation of `parts`.
pub fn sha256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// Constant-time equality (`subtle`). Slices of different lengths are unequal;
/// the lengths themselves are not secret.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answer() {
        // FIPS 180-2, "abc".
        assert_eq!(
            data_encoding::HEXLOWER.encode(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_parts(&[b"a", b"", b"bc"]), sha256(b"abc"));
    }

    #[test]
    fn ct_eq_cases() {
        assert!(ct_eq(b"", b""));
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn random_arrays_differ() {
        let a: [u8; 32] = random_array().unwrap();
        let b: [u8; 32] = random_array().unwrap();
        assert_ne!(a, b);
    }
}
