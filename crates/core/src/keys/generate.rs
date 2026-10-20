//! Generate key (UI 7.1; docs/data-model.md, section 9.1; night decision
//! D32): Ed25519 (the default), ECDSA on P-256, P-384 or P-521, RSA with
//! 3072 (the default size) or 4096 bits. RSA 2048 is not offered. The new
//! key is an OpenSSH private key, encrypted when a passphrase is given
//! (aes256-ctr, bcrypt-pbkdf with the `ssh-key` default of 16 rounds).
//!
//! Randomness comes from the operating system (`getrandom`). RSA takes
//! seconds; call without the core mutex.

use getrandom::SysRng;
use getrandom::rand_core::UnwrapErr;
use secrecy::{ExposeSecret, SecretString};
use ssh_key::private::{KeypairData, RsaKeypair};
use ssh_key::{Algorithm, EcdsaCurve, LineEnding, PrivateKey};
use zeroize::Zeroizing;

use crate::crypto::CryptoError;

/// The key types the Generate key form offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    #[default]
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    Rsa3072,
    Rsa4096,
}

impl KeyKind {
    /// Every offered kind, in the order of the form.
    pub const ALL: [Self; 6] = [
        Self::Ed25519,
        Self::EcdsaP256,
        Self::EcdsaP384,
        Self::EcdsaP521,
        Self::Rsa3072,
        Self::Rsa4096,
    ];

    /// The OpenSSH algorithm name of keys of this kind.
    pub fn algorithm_name(self) -> &'static str {
        match self {
            Self::Ed25519 => "ssh-ed25519",
            Self::EcdsaP256 => "ecdsa-sha2-nistp256",
            Self::EcdsaP384 => "ecdsa-sha2-nistp384",
            Self::EcdsaP521 => "ecdsa-sha2-nistp521",
            Self::Rsa3072 | Self::Rsa4096 => "ssh-rsa",
        }
    }
}

/// A new key pair of `kind` with `comment` (the label), as OpenSSH private
/// key text; encrypted with `passphrase` when it is given and not empty.
pub fn generate(
    kind: KeyKind,
    comment: &str,
    passphrase: Option<&SecretString>,
) -> Result<Zeroizing<String>, CryptoError> {
    // A failing system RNG is reported here instead of panicking below
    // (`UnwrapErr` panics on an RNG error, which cannot happen once the
    // source works).
    let mut probe = [0u8; 16];
    getrandom::fill(&mut probe).map_err(|_| CryptoError::Rng)?;
    let mut rng = UnwrapErr(SysRng);
    let mut private = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519),
        KeyKind::EcdsaP256 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        ),
        KeyKind::EcdsaP384 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
        ),
        KeyKind::EcdsaP521 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP521,
            },
        ),
        KeyKind::Rsa3072 | KeyKind::Rsa4096 => {
            let bits = if kind == KeyKind::Rsa3072 { 3072 } else { 4096 };
            RsaKeypair::random(&mut rng, bits)
                .and_then(|keypair| PrivateKey::new(KeypairData::Rsa(keypair), ""))
        }
    }
    .map_err(|_| CryptoError::Rng)?;
    private.set_comment(comment);
    let private = match passphrase.filter(|value| !value.expose_secret().is_empty()) {
        Some(passphrase) => private
            .encrypt(&mut SysRng, passphrase.expose_secret().as_bytes())
            .map_err(|_| CryptoError::Malformed("generated key"))?,
        None => private,
    };
    private
        .to_openssh(LineEnding::LF)
        .map_err(|_| CryptoError::Malformed("generated key"))
}
