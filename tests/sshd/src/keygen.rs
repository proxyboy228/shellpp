//! Key generation for tests: host keys and client keys (OpenSSH format,
//! optionally encrypted with a passphrase). Keys exist only at run time.
//!
//! Everything cryptographic comes from ssh-key (through `russh::keys`) and
//! the operating system CSPRNG behind `rand::rng()`; nothing is homemade.

use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
use russh::keys::ssh_key::{EcdsaCurve, LineEnding};
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

/// Bit size of [`KeyType::Rsa3072`] keys.
const RSA_BITS: usize = 3072;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyType {
    Ed25519,
    EcdsaP256,
    #[serde(rename = "rsa-3072")]
    Rsa3072,
}

impl KeyType {
    /// The CLI and JSON name: `ed25519`, `ecdsa-p256`, `rsa-3072`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::EcdsaP256 => "ecdsa-p256",
            Self::Rsa3072 => "rsa-3072",
        }
    }

    /// Parses the CLI and JSON name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ed25519" => Some(Self::Ed25519),
            "ecdsa-p256" => Some(Self::EcdsaP256),
            "rsa-3072" => Some(Self::Rsa3072),
            _ => None,
        }
    }

    /// The type of an existing key, when it is one of the three generated
    /// types (any RSA size maps to [`KeyType::Rsa3072`]).
    pub(crate) fn of(key: &PrivateKey) -> Option<Self> {
        match key.algorithm() {
            Algorithm::Ed25519 => Some(Self::Ed25519),
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            } => Some(Self::EcdsaP256),
            Algorithm::Rsa { .. } => Some(Self::Rsa3072),
            _ => None,
        }
    }
}

/// A generated key pair.
pub struct GeneratedKey {
    /// OpenSSH private key text (`-----BEGIN OPENSSH PRIVATE KEY-----`),
    /// encrypted when a passphrase was given.
    pub private_openssh: SecretString,
    /// One OpenSSH public key line, `<algorithm> <base64> <comment>`.
    pub public_openssh: String,
    /// `SHA256:` fingerprint as OpenSSH prints it.
    pub fingerprint: String,
}

impl std::fmt::Debug for GeneratedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedKey")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

/// Generates a key pair. `passphrase` encrypts the private key with the
/// standard OpenSSH key encryption (bcrypt-pbkdf + aes256-ctr). An empty
/// passphrase leaves the key unencrypted.
pub fn generate_key(
    key_type: KeyType,
    comment: &str,
    passphrase: Option<&SecretString>,
) -> Result<GeneratedKey, crate::Error> {
    let mut key = random_private_key(key_type)?;
    key.set_comment(comment);
    let public_openssh = key.public_key().to_openssh().map_err(key_error)?;
    let fingerprint = fingerprint(key.public_key());
    let passphrase = passphrase
        .map(|secret| secret.expose_secret())
        .filter(|text| !text.is_empty());
    let private = match passphrase {
        Some(passphrase) => key
            .encrypt(&mut rand::rng(), passphrase.as_bytes())
            .map_err(key_error)?
            .to_openssh(LineEnding::LF)
            .map_err(key_error)?,
        None => key.to_openssh(LineEnding::LF).map_err(key_error)?,
    };
    Ok(GeneratedKey {
        private_openssh: SecretString::from(private.as_str()),
        public_openssh,
        fingerprint,
    })
}

/// A new random private key of `key_type` (no comment).
pub(crate) fn random_private_key(key_type: KeyType) -> Result<PrivateKey, crate::Error> {
    let mut rng = rand::rng();
    match key_type {
        KeyType::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(key_error),
        KeyType::EcdsaP256 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )
        .map_err(key_error),
        KeyType::Rsa3072 => {
            let keypair = RsaKeypair::random(&mut rng, RSA_BITS).map_err(key_error)?;
            PrivateKey::new(KeypairData::from(keypair), "").map_err(key_error)
        }
    }
}

/// `SHA256:<base64>` fingerprint of a public key.
pub(crate) fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// The algorithm name and the base64 blob of a public key, as in a
/// known_hosts line.
pub(crate) fn openssh_parts(key: &PublicKey) -> (String, String) {
    let algorithm = key.algorithm().as_str().to_owned();
    // `to_openssh` is `<algorithm> <base64>[ <comment>]`; the comment of a
    // host key is empty.
    let line = key.to_openssh().unwrap_or_default();
    let blob = line.split(' ').nth(1).unwrap_or_default().to_owned();
    (algorithm, blob)
}

fn key_error(error: impl std::fmt::Display) -> crate::Error {
    crate::Error::Key(error.to_string())
}
