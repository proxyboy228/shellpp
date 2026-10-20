//! The encrypted JSON export (docs/storage-format.md, section 13.3):
//! `export_key` = HKDF-SHA256(Argon2id(export password, 16-byte salt), info
//! `"shellpp/v1/export"`), XChaCha20-Poly1305 single shot with a random
//! 24-byte nonce, AAD = `"shellpp.export.encrypted.v1"` ‖ salt ‖ be32(m_kib)
//! ‖ be32(t) ‖ be32(p) (55 bytes). Section 14.7 opens the file without the
//! app.

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::kdf::{KdfParams, derive_with_salt};
use super::{CryptoError, PlatformClass, derive_export_key, random_array, xchacha};
use crate::ids::EXPORT_FORMAT_ENCRYPTED_V1;

/// Length of the export salt.
pub const EXPORT_SALT_LEN: usize = 16;
/// Length of the export nonce.
pub const EXPORT_NONCE_LEN: usize = xchacha::NONCE_LEN;
/// Length of the export AAD.
pub const EXPORT_AAD_LEN: usize = 55;

/// The sealed parts of an encrypted export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedExport {
    pub salt: [u8; EXPORT_SALT_LEN],
    pub nonce: [u8; EXPORT_NONCE_LEN],
    /// Ciphertext ‖ 16-byte tag.
    pub ciphertext: Vec<u8>,
}

/// The AAD of an encrypted export.
pub fn export_aad(salt: &[u8; EXPORT_SALT_LEN], params: &KdfParams) -> [u8; EXPORT_AAD_LEN] {
    let mut aad = [0u8; EXPORT_AAD_LEN];
    let format = EXPORT_FORMAT_ENCRYPTED_V1.as_bytes();
    aad[..27].copy_from_slice(format);
    aad[27..43].copy_from_slice(salt);
    aad[43..47].copy_from_slice(&params.m_kib.to_be_bytes());
    aad[47..51].copy_from_slice(&params.t.to_be_bytes());
    aad[51..55].copy_from_slice(&params.p.to_be_bytes());
    aad
}

/// Encrypts the bytes of a plain export with `password` and `params` (the
/// preset of the exporting platform). Slow (Argon2id): never under a lock
/// other threads wait on.
pub fn seal_export(
    password: &SecretString,
    params: &KdfParams,
    plaintext: &[u8],
) -> Result<SealedExport, CryptoError> {
    let salt: [u8; EXPORT_SALT_LEN] = random_array()?;
    let nonce: [u8; EXPORT_NONCE_LEN] = random_array()?;
    seal_with(password, params, plaintext, salt, nonce)
}

fn seal_with(
    password: &SecretString,
    params: &KdfParams,
    plaintext: &[u8],
    salt: [u8; EXPORT_SALT_LEN],
    nonce: [u8; EXPORT_NONCE_LEN],
) -> Result<SealedExport, CryptoError> {
    let ikm = derive_with_salt(password, params, &salt, PlatformClass::Desktop)?;
    let key = derive_export_key(&ikm);
    let mut ciphertext = Vec::new();
    xchacha::seal_append(
        &key,
        &nonce,
        &export_aad(&salt, params),
        plaintext,
        &mut ciphertext,
    )?;
    Ok(SealedExport {
        salt,
        nonce,
        ciphertext,
    })
}

/// Decrypts an encrypted export. `params` outside the bounds of
/// docs/storage-format.md, section 6.5 are refused before anything is
/// derived; a wrong password or a changed byte is [`CryptoError::Aead`].
pub fn open_export(
    password: &SecretString,
    params: &KdfParams,
    sealed: &SealedExport,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let ikm = derive_with_salt(password, params, &sealed.salt, PlatformClass::Desktop)?;
    let key = derive_export_key(&ikm);
    xchacha::open(
        &key,
        &sealed.nonce,
        &export_aad(&sealed.salt, params),
        &sealed.ciphertext,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aad_layout() {
        let params = KdfParams::preset(PlatformClass::Desktop);
        let salt = [7u8; EXPORT_SALT_LEN];
        let aad = export_aad(&salt, &params);
        assert_eq!(&aad[..27], b"shellpp.export.encrypted.v1");
        assert_eq!(&aad[27..43], &salt);
        assert_eq!(&aad[43..47], &65_536u32.to_be_bytes());
        assert_eq!(&aad[47..51], &3u32.to_be_bytes());
        assert_eq!(&aad[51..55], &4u32.to_be_bytes());
    }

    #[test]
    fn seal_and_open() {
        let params = KdfParams::preset(PlatformClass::Desktop);
        let password = SecretString::from("an export password".to_owned());
        let sealed = seal_export(&password, &params, b"{\"format\":1}").unwrap();
        let opened = open_export(&password, &params, &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"{\"format\":1}");
        // A wrong password, a changed parameter or byte: authentication fails.
        let wrong = SecretString::from("another password".to_owned());
        assert!(matches!(
            open_export(&wrong, &params, &sealed),
            Err(CryptoError::Aead)
        ));
        let mut changed = sealed.clone();
        changed.ciphertext[0] ^= 1;
        assert!(matches!(
            open_export(&password, &params, &changed),
            Err(CryptoError::Aead)
        ));
        let mut more = params.clone();
        more.t += 1;
        assert!(matches!(
            open_export(&password, &more, &sealed),
            Err(CryptoError::Aead)
        ));
        // Parameters below the floor are refused before deriving.
        let mut weak = params.clone();
        weak.m_kib = 1024;
        assert!(matches!(
            open_export(&password, &weak, &sealed),
            Err(CryptoError::KdfParams(_))
        ));
    }
}
