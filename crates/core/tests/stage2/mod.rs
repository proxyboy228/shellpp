//! Helpers shared by the stage 2 integration tests (keys, known hosts,
//! history, sessions, canary). Keys are generated at run time from the OS
//! random generator (CONTRIBUTING); nothing here is a real credential.

#![allow(dead_code)]

use std::convert::Infallible;

use secrecy::SecretString;
use ssh_key::private::{KeypairData, RsaKeypair};
use ssh_key::rand_core::{TryCryptoRng, TryRng};
use ssh_key::{Algorithm, EcdsaCurve, LineEnding, PrivateKey};

/// The OS random generator as a `rand_core` RNG for `ssh-key`.
pub struct OsRng;

impl TryRng for OsRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(getrandom::u32().expect("os random"))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(getrandom::u64().expect("os random"))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        getrandom::fill(dst).expect("os random");
        Ok(())
    }
}

impl TryCryptoRng for OsRng {}

/// Key types used by the tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    Rsa(usize),
}

/// A new private key with a comment.
pub fn generate(kind: KeyKind, comment: &str) -> PrivateKey {
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("ed25519"),
        KeyKind::EcdsaP256 => PrivateKey::random(
            &mut OsRng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )
        .expect("p256"),
        KeyKind::EcdsaP384 => PrivateKey::random(
            &mut OsRng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
        )
        .expect("p384"),
        KeyKind::Rsa(bits) => {
            let keypair = RsaKeypair::random(&mut OsRng, bits).expect("rsa");
            PrivateKey::new(KeypairData::from(keypair), "").expect("rsa key")
        }
    };
    key.set_comment(comment);
    key
}

/// OpenSSH text of a key, encrypted with `passphrase` (the default
/// bcrypt-pbkdf rounds of `ssh-key`, never weakened) when given.
pub fn openssh(key: &PrivateKey, passphrase: Option<&str>) -> String {
    let key = match passphrase {
        Some(passphrase) => key.encrypt(&mut OsRng, passphrase).expect("encrypt"),
        None => key.clone(),
    };
    key.to_openssh(LineEnding::LF).expect("openssh").to_string()
}

/// SHA256 fingerprint as OpenSSH prints it.
pub fn fingerprint(key: &PrivateKey) -> String {
    key.fingerprint(ssh_key::HashAlg::Sha256).to_string()
}

/// A passphrase generated at run time.
pub fn passphrase() -> SecretString {
    let bytes: [u8; 12] = shellpp_core::crypto::random_array().expect("random");
    SecretString::from(format!(
        "pp-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// The OpenSSH text of an encrypted key with its bcrypt-pbkdf rounds field
/// set to `rounds` (a crafted key file; nothing is re-encrypted).
pub fn with_kdf_rounds(text: &str, rounds: u32) -> String {
    use data_encoding::BASE64;
    let body: String = text
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let mut bytes = BASE64.decode(body.as_bytes()).expect("base64 body");
    let read_u32 = |bytes: &[u8], at: usize| {
        u32::from_be_bytes(bytes[at..at + 4].try_into().expect("u32")) as usize
    };
    // "openssh-key-v1\0", string cipher, string kdf name, string kdf options
    // (string salt, uint32 rounds).
    let mut at = b"openssh-key-v1\0".len();
    at += 4 + read_u32(&bytes, at);
    at += 4 + read_u32(&bytes, at);
    at += 4;
    at += 4 + read_u32(&bytes, at);
    bytes[at..at + 4].copy_from_slice(&rounds.to_be_bytes());
    let encoded = BASE64.encode(&bytes);
    let mut out = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(70) {
        out.push_str(std::str::from_utf8(chunk).expect("ascii"));
        out.push('\n');
    }
    out.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    out
}
