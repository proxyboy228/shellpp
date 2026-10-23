//! The minisign signature of the update manifest (D170; docs/release.md).
//!
//! The release workflow signs `latest.json` with the owner's minisign key
//! and publishes `latest.json.minisig` next to it; the public half of the
//! key is `release/minisign.pub`, built into the app. A manifest counts
//! only when its signature opens with that key, so a changed update source
//! (Settings ▸ Updates) or a taken-over release page cannot announce an
//! update of its own.
//!
//! Only the format is parsed here; the cryptography is ed25519-dalek and
//! BLAKE2b-512 (the RustCrypto `blake2`), as minisign itself does:
//! - a public key is `"Ed"`, an 8-byte key id and the 32-byte Ed25519 key;
//! - a signature is `"ED"`, the key id and the Ed25519 signature of the
//!   file's BLAKE2b-512 hash (the legacy `"Ed"`, a signature of the file
//!   itself, is refused: the workflow's minisign never makes it);
//! - the global signature signs the signature and the trusted comment.

use base64ct::{Base64, Encoding as _};
use blake2::{Blake2b512, Digest as _};
use ed25519_dalek::{Signature, VerifyingKey};

/// The release signing key of this build (`release/minisign.pub`); a file
/// without a key leaves the build unable to check updates.
const RELEASE_KEY: &str = include_str!("../../../../release/minisign.pub");

/// A signature file is a few hundred bytes; more is not one.
pub const MAX_SIGNATURE_BYTES: u64 = 4096;

const UNTRUSTED: &str = "untrusted comment:";
const TRUSTED: &str = "trusted comment: ";

/// A minisign public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    id: [u8; 8],
    key: VerifyingKey,
}

/// Why a manifest's signature does not count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SignatureError {
    /// Not a minisign signature file (or the legacy, unhashed kind).
    #[error("not a minisign signature")]
    Malformed,
    /// Made with another key than this build's.
    #[error("signed with another key")]
    OtherKey,
    /// The key's, but the file or the trusted comment changed.
    #[error("the signature does not match")]
    Mismatch,
}

impl PublicKey {
    /// The key of this build's `release/minisign.pub`, `None` while the
    /// file holds none.
    pub fn release() -> Option<Self> {
        Self::parse(RELEASE_KEY)
    }

    /// A minisign `.pub` file: the line after the untrusted comment.
    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines().skip_while(|line| !line.starts_with(UNTRUSTED));
        lines.next()?;
        let mut bytes = [0u8; 42];
        let decoded = Base64::decode(lines.next()?.trim(), &mut bytes).ok()?;
        if decoded.len() != 42 || &decoded[..2] != b"Ed" {
            return None;
        }
        let id = decoded[2..10].try_into().ok()?;
        let key = VerifyingKey::from_bytes(decoded[10..42].try_into().ok()?).ok()?;
        Some(Self { id, key })
    }

    /// Whether `signature` (a `.minisig` file) signs `data` with this key,
    /// its trusted comment included.
    pub fn verify(&self, data: &[u8], signature: &str) -> Result<(), SignatureError> {
        let mut lines = signature.lines();
        if !lines.next().is_some_and(|line| line.starts_with(UNTRUSTED)) {
            return Err(SignatureError::Malformed);
        }
        let mut sig = [0u8; 74];
        let sig = lines
            .next()
            .and_then(|line| Base64::decode(line.trim(), &mut sig).ok())
            .filter(|sig| sig.len() == 74)
            .ok_or(SignatureError::Malformed)?;
        let comment = lines
            .next()
            .and_then(|line| line.strip_prefix(TRUSTED))
            .ok_or(SignatureError::Malformed)?;
        let mut global = [0u8; 64];
        let global = lines
            .next()
            .and_then(|line| Base64::decode(line.trim(), &mut global).ok())
            .filter(|global| global.len() == 64)
            .ok_or(SignatureError::Malformed)?;
        if lines.any(|line| !line.trim().is_empty()) || &sig[..2] != b"ED" {
            return Err(SignatureError::Malformed);
        }
        if sig[2..10] != self.id {
            return Err(SignatureError::OtherKey);
        }
        let file_sig = Signature::from_bytes(sig[10..74].try_into().expect("64 bytes"));
        let hash = Blake2b512::digest(data);
        self.key
            .verify_strict(&hash, &file_sig)
            .map_err(|_| SignatureError::Mismatch)?;
        let global = Signature::from_bytes(global.try_into().expect("64 bytes"));
        let mut signed = Vec::with_capacity(64 + comment.len());
        signed.extend_from_slice(&sig[10..74]);
        signed.extend_from_slice(comment.as_bytes());
        self.key
            .verify_strict(&signed, &global)
            .map_err(|_| SignatureError::Mismatch)
    }
}

/// Where the signature of the manifest at `url` is: the same address with
/// `.minisig` after the path.
pub fn signature_url(url: &str) -> String {
    let url = url.trim();
    match url.find(['?', '#']) {
        Some(at) => format!("{}.minisig{}", &url[..at], &url[at..]),
        None => format!("{url}.minisig"),
    }
}
