//! Key import formats besides OpenSSH (UI 7.1 "New key", 7.3; docs/data-model.md,
//! section 9.1; D5): PEM PKCS#1 (`RSA PRIVATE KEY`), PKCS#8 (`PRIVATE KEY`,
//! and `ENCRYPTED PRIVATE KEY` with PBES2), SEC1 (`EC PRIVATE KEY`), the
//! legacy OpenSSL encryption of PKCS#1 and SEC1 (`Proc-Type: 4,ENCRYPTED`
//! with `DEK-Info: AES-128-CBC`, `AES-192-CBC` or `AES-256-CBC`), and PuTTY
//! `.ppk` versions 2 and 3.
//!
//! Every format becomes an OpenSSH private key, the stored form. A source
//! that was encrypted is encrypted again with the same passphrase in the
//! standard OpenSSH encryption (`ssh-key`: aes256-ctr, bcrypt-pbkdf), so the
//! key is never stored unencrypted when it came encrypted. Nothing here logs
//! key material; decoded buffers are wiped on drop.
//!
//! Against hostile files the key derivations a source asks for are bounded
//! before anything is derived (PBKDF2 iterations, scrypt memory, the Argon2
//! memory and passes of `.ppk` v3), as [`super::MAX_KDF_ROUNDS`] bounds
//! bcrypt-pbkdf.

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use data_encoding::BASE64_MIME_PERMISSIVE;
use pkcs8::der::Decode as _;
use pkcs8::{AssociatedOid as _, ObjectIdentifier};
use secrecy::{ExposeSecret, SecretString};
use ssh_key::private::{EcdsaKeypair, Ed25519Keypair, Ed25519PrivateKey, KeypairData};
use ssh_key::{LineEnding, PrivateKey};
use zeroize::Zeroizing;

/// The formats a pasted or chosen text can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----`, the stored form.
    OpenSsh,
    /// `-----BEGIN RSA PRIVATE KEY-----`.
    Pkcs1,
    /// `-----BEGIN PRIVATE KEY-----` or `-----BEGIN ENCRYPTED PRIVATE KEY-----`.
    Pkcs8,
    /// `-----BEGIN EC PRIVATE KEY-----`.
    Sec1,
    /// `PuTTY-User-Key-File-2:` or `-3:`.
    Putty,
}

impl Format {
    /// The name the key panel shows for the source format.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH",
            Self::Pkcs1 => "PEM (PKCS#1)",
            Self::Pkcs8 => "PEM (PKCS#8)",
            Self::Sec1 => "PEM (SEC1)",
            Self::Putty => "PuTTY (.ppk)",
        }
    }
}

/// Why a text could not be imported as a key: the key panel shows it next to
/// the type "unknown" (UI 7.2).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyProblem {
    /// Not a private key in a known format (a certificate, another file).
    #[error("not a private key in a supported format")]
    NotAKey,
    /// A public key (an OpenSSH `.pub` line, an RFC 4716 or a PEM public
    /// key): the private key is the file next to it, without `.pub`.
    #[error("a public key, not a private key")]
    PublicKey,
    /// A known format whose content could not be decoded.
    #[error("the key is damaged or incomplete")]
    Damaged,
    /// A key type this app cannot use (DSA, Ed448, an unknown curve).
    #[error("unsupported key type {0}")]
    Algorithm(String),
    /// RSA under [`super::MIN_RSA_BITS`] bits.
    #[error("RSA keys under 2048 bits are not accepted")]
    RsaTooSmall,
    /// An encryption this app does not read (for example DES-EDE3-CBC).
    #[error("unsupported key encryption {0}")]
    Cipher(String),
    /// The key derivation the file asks for exceeds the limits.
    #[error("the key's passphrase derivation is too expensive")]
    KdfTooExpensive,
    /// The key is encrypted and no passphrase was given.
    #[error("the key is protected by a passphrase")]
    PassphraseNeeded,
    /// The passphrase does not open the key.
    #[error("wrong passphrase")]
    WrongPassphrase,
}

/// Most PBKDF2 iterations accepted in an encrypted PKCS#8 key (OpenSSL
/// writes 2048 by default; 10 million take seconds).
pub const MAX_PBKDF2_ITERATIONS: u32 = 10_000_000;
/// Most memory scrypt may use for an encrypted PKCS#8 key (128 · r · N · p,
/// counting the lanes as if they ran at once).
pub const MAX_SCRYPT_BYTES: u64 = 256 * 1024 * 1024;
/// Most memory (KiB) Argon2 may use for a `.ppk` v3 key (puttygen writes
/// 8192 KiB by default; 1 GiB leaves room for stronger settings).
pub const MAX_PPK_ARGON2_KIB: u32 = 1024 * 1024;
/// Most Argon2 passes for a `.ppk` v3 key (puttygen tunes the passes to
/// about 100 ms, typically under 30).
pub const MAX_PPK_ARGON2_PASSES: u32 = 64;
/// Most Argon2 lanes for a `.ppk` v3 key (puttygen writes 1).
pub const MAX_PPK_ARGON2_LANES: u32 = 64;

/// The PEM labels this module reads.
const PEM_RSA: &str = "RSA PRIVATE KEY";
const PEM_PKCS8: &str = "PRIVATE KEY";
const PEM_PKCS8_ENCRYPTED: &str = "ENCRYPTED PRIVATE KEY";
const PEM_SEC1: &str = "EC PRIVATE KEY";
const PEM_OPENSSH: &str = "OPENSSH PRIVATE KEY";
const PPK_PREFIX: &str = "PuTTY-User-Key-File-";

/// Whether `text` (already normalized) is a public key: an OpenSSH line
/// (`ssh-ed25519 AAAA… comment`, a `.pub` file, certificates included), an
/// RFC 4716 block or a PEM `PUBLIC KEY`. Pasted instead of the private key,
/// it gets its own message (UI 7.2).
pub fn is_public_key(text: &str) -> bool {
    let first = text.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(first) = first else {
        return false;
    };
    if first.starts_with("---- BEGIN SSH2 PUBLIC KEY ----") {
        return true;
    }
    if let Some(pem) = Pem::parse(text)
        && matches!(pem.label, "PUBLIC KEY" | "RSA PUBLIC KEY")
    {
        return true;
    }
    // `algorithm base64 [comment]`, after authorized_keys options when there
    // are any (they may hold quoted spaces: `command="echo hi"`), so every
    // pair of words is tried; the blob starts with the algorithm name's
    // length (`AAAA`).
    let words: Vec<&str> = first.split_whitespace().collect();
    words.windows(2).any(|pair| {
        let (algorithm, blob) = (pair[0], pair[1]);
        (algorithm.starts_with("ssh-")
            || algorithm.starts_with("ecdsa-sha2-")
            || algorithm.starts_with("sk-"))
            && blob.starts_with("AAAA")
            && blob
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
    })
}

/// The format of `text` (already normalized) and whether it needs a
/// passphrase to be read; `None` when it is not a private key this module
/// knows.
pub fn detect(text: &str) -> Option<(Format, bool)> {
    if let Some(rest) = text.strip_prefix(PPK_PREFIX) {
        if !(rest.starts_with("2:") || rest.starts_with("3:")) {
            return None;
        }
        let encrypted = text
            .lines()
            .find_map(|line| line.strip_prefix("Encryption:"))
            .is_some_and(|value| value.trim() != "none");
        return Some((Format::Putty, encrypted));
    }
    let pem = Pem::parse(text)?;
    match pem.label {
        PEM_OPENSSH => {
            let encrypted = super::text_is_encrypted(text)?;
            Some((Format::OpenSsh, encrypted))
        }
        PEM_RSA => Some((Format::Pkcs1, pem.legacy_encrypted())),
        PEM_SEC1 => Some((Format::Sec1, pem.legacy_encrypted())),
        PEM_PKCS8 => Some((Format::Pkcs8, false)),
        PEM_PKCS8_ENCRYPTED => Some((Format::Pkcs8, true)),
        _ => None,
    }
}

/// Converts a key in any supported format (normalized text) to the stored
/// OpenSSH form. `passphrase` opens an encrypted source, which is then
/// encrypted again with it; an unencrypted source stays unencrypted (a
/// passphrase given for it is ignored here, the caller refuses it). An
/// OpenSSH key is returned as it is. Slow for encrypted sources (their key
/// derivation and bcrypt-pbkdf): call without the core mutex.
pub fn to_openssh(
    text: &str,
    passphrase: Option<&SecretString>,
) -> Result<Zeroizing<String>, KeyProblem> {
    let passphrase = passphrase.filter(|value| !value.expose_secret().is_empty());
    let (format, encrypted) = detect(text).ok_or(KeyProblem::NotAKey)?;
    if format == Format::OpenSsh {
        return Ok(Zeroizing::new(text.to_owned()));
    }
    if encrypted && passphrase.is_none() {
        return Err(KeyProblem::PassphraseNeeded);
    }
    let private = match format {
        Format::Putty => from_putty(text, passphrase.filter(|_| encrypted))?,
        Format::OpenSsh => unreachable!("returned above"),
        Format::Pkcs1 | Format::Pkcs8 | Format::Sec1 => {
            let pem = Pem::parse(text).ok_or(KeyProblem::NotAKey)?;
            from_pem(&pem, passphrase.filter(|_| encrypted))?
        }
    };
    check_allowed(&private)?;
    let private = match passphrase.filter(|_| encrypted) {
        Some(passphrase) => private
            .encrypt(
                &mut getrandom::SysRng,
                passphrase.expose_secret().as_bytes(),
            )
            .map_err(|_| KeyProblem::Damaged)?,
        None => private,
    };
    private
        .to_openssh(LineEnding::LF)
        .map_err(|_| KeyProblem::Damaged)
}

/// RSA at least [`super::MIN_RSA_BITS`]; Ed25519 and the NIST curves.
fn check_allowed(private: &PrivateKey) -> Result<(), KeyProblem> {
    use ssh_key::{Algorithm, EcdsaCurve};
    match private.algorithm() {
        Algorithm::Ed25519
        | Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256 | EcdsaCurve::NistP384 | EcdsaCurve::NistP521,
        } => Ok(()),
        Algorithm::Rsa { .. } => {
            let bits = private
                .public_key()
                .key_data()
                .rsa()
                .map_or(0, |rsa| rsa.key_size());
            if bits >= super::MIN_RSA_BITS {
                Ok(())
            } else {
                Err(KeyProblem::RsaTooSmall)
            }
        }
        other => Err(KeyProblem::Algorithm(other.as_str().to_owned())),
    }
}

fn from_putty(text: &str, passphrase: Option<&SecretString>) -> Result<PrivateKey, KeyProblem> {
    check_ppk_kdf(text)?;
    let encrypted = passphrase.is_some();
    // ssh-key takes the passphrase as a `String`; this copy is wiped when
    // the call returns only as far as ssh-key wipes it.
    let passphrase = passphrase.map(|value| value.expose_secret().to_owned());
    PrivateKey::from_ppk(text, passphrase).map_err(|error| match error {
        ssh_key::Error::Crypto | ssh_key::Error::Ppk(_) if encrypted => KeyProblem::WrongPassphrase,
        ssh_key::Error::AlgorithmUnknown | ssh_key::Error::AlgorithmUnsupported { .. } => {
            KeyProblem::Algorithm(ppk_algorithm(text))
        }
        _ => KeyProblem::Damaged,
    })
}

/// The algorithm on the first line of a `.ppk` file.
fn ppk_algorithm(text: &str) -> String {
    text.lines()
        .next()
        .and_then(|line| line.split_once(':'))
        .map(|(_, algorithm)| algorithm.trim().chars().take(64).collect())
        .unwrap_or_default()
}

/// Refuses `.ppk` v3 files whose Argon2 settings exceed the limits. The
/// headers are read as ssh-key reads them (`Name: value` lines, the base64
/// lines of `*-Lines` skipped), and a header given twice is refused: ssh-key
/// keeps the last one, so a check of the first could be fooled.
fn check_ppk_kdf(text: &str) -> Result<(), KeyProblem> {
    let mut seen: Vec<&str> = Vec::new();
    let mut memory = None;
    let mut passes = None;
    let mut lanes = None;
    let mut lines = text.lines().skip(1);
    while let Some(line) = lines.next() {
        let Some((name, value)) = line.split_once(": ") else {
            return Err(KeyProblem::Damaged);
        };
        if seen.contains(&name) {
            return Err(KeyProblem::Damaged);
        }
        seen.push(name);
        let number = || value.parse::<u32>().map_err(|_| KeyProblem::Damaged);
        match name {
            "Argon2-Memory" => memory = Some(number()?),
            "Argon2-Passes" => passes = Some(number()?),
            "Argon2-Parallelism" => lanes = Some(number()?),
            _ if name.ends_with("-Lines") => {
                let count = usize::try_from(number()?).map_err(|_| KeyProblem::Damaged)?;
                for _ in 0..count {
                    lines.next().ok_or(KeyProblem::Damaged)?;
                }
            }
            _ => {}
        }
    }
    if memory.is_some_and(|kib| kib > MAX_PPK_ARGON2_KIB)
        || passes.is_some_and(|passes| passes > MAX_PPK_ARGON2_PASSES)
        || lanes.is_some_and(|lanes| lanes > MAX_PPK_ARGON2_LANES)
    {
        return Err(KeyProblem::KdfTooExpensive);
    }
    Ok(())
}

/// A PEM block: its label, the RFC 1421 headers of the legacy encryption
/// (`Proc-Type`, `DEK-Info`) and the decoded body.
struct Pem<'a> {
    label: &'a str,
    dek_info: Option<&'a str>,
    proc_type_encrypted: bool,
    body: Zeroizing<Vec<u8>>,
}

impl<'a> Pem<'a> {
    /// The first PEM block of `text` (normalized: LF line endings, no
    /// surrounding whitespace). `None` when there is none or its body is not
    /// base64.
    fn parse(text: &'a str) -> Option<Self> {
        let mut lines = text.lines();
        let label = lines
            .next()?
            .strip_prefix("-----BEGIN ")?
            .strip_suffix("-----")?;
        let end = format!("-----END {label}-----");
        let mut dek_info = None;
        let mut proc_type_encrypted = false;
        let mut body = Zeroizing::new(String::with_capacity(text.len()));
        let mut ended = false;
        for line in lines {
            let line = line.trim();
            if line == end {
                ended = true;
                break;
            }
            if let Some(value) = line.strip_prefix("Proc-Type:") {
                proc_type_encrypted = value.trim().ends_with("ENCRYPTED");
            } else if let Some(value) = line.strip_prefix("DEK-Info:") {
                dek_info = Some(value.trim());
            } else if !line.is_empty() {
                body.push_str(line);
            }
        }
        if !ended {
            return None;
        }
        let body = Zeroizing::new(BASE64_MIME_PERMISSIVE.decode(body.as_bytes()).ok()?);
        Some(Self {
            label,
            dek_info,
            proc_type_encrypted,
            body,
        })
    }

    fn legacy_encrypted(&self) -> bool {
        self.proc_type_encrypted || self.dek_info.is_some()
    }

    /// The DER body, decrypted when the block uses the legacy encryption.
    fn der(&self, passphrase: Option<&SecretString>) -> Result<Zeroizing<Vec<u8>>, KeyProblem> {
        if !self.legacy_encrypted() {
            return Ok(self.body.clone());
        }
        let passphrase = passphrase.ok_or(KeyProblem::PassphraseNeeded)?;
        let dek_info = self.dek_info.ok_or(KeyProblem::Damaged)?;
        legacy_decrypt(dek_info, &self.body, passphrase)
    }
}

/// The legacy OpenSSL PEM encryption: `DEK-Info: <cipher>,<hex IV>`, the key
/// from `EVP_BytesToKey` with MD5 and the first 8 bytes of the IV as the
/// salt, CBC with PKCS#7 padding.
fn legacy_decrypt(
    dek_info: &str,
    body: &[u8],
    passphrase: &SecretString,
) -> Result<Zeroizing<Vec<u8>>, KeyProblem> {
    let (cipher, iv_hex) = dek_info.split_once(',').ok_or(KeyProblem::Damaged)?;
    let cipher = cipher.trim();
    let key_len = match cipher {
        "AES-128-CBC" => 16,
        "AES-192-CBC" => 24,
        "AES-256-CBC" => 32,
        other => return Err(KeyProblem::Cipher(other.chars().take(32).collect())),
    };
    let iv = data_encoding::HEXUPPER_PERMISSIVE
        .decode(iv_hex.trim().as_bytes())
        .map_err(|_| KeyProblem::Damaged)?;
    let iv: [u8; 16] = iv.try_into().map_err(|_| KeyProblem::Damaged)?;
    let key = evp_bytes_to_key(passphrase.expose_secret().as_bytes(), &iv[..8], key_len);
    let mut buffer = Zeroizing::new(body.to_vec());
    let plain_len = match key_len {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
            .map_err(|_| KeyProblem::Damaged)?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .map(<[u8]>::len),
        24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(&key, &iv)
            .map_err(|_| KeyProblem::Damaged)?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .map(<[u8]>::len),
        _ => cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
            .map_err(|_| KeyProblem::Damaged)?
            .decrypt_padded::<Pkcs7>(&mut buffer)
            .map(<[u8]>::len),
    }
    // A wrong passphrase almost always breaks the padding.
    .map_err(|_| KeyProblem::WrongPassphrase)?;
    buffer.truncate(plain_len);
    Ok(buffer)
}

/// `EVP_BytesToKey(MD5, salt, passphrase, count = 1)`: D1 = MD5(P ‖ S),
/// Di = MD5(Di-1 ‖ P ‖ S), the key is D1 ‖ D2 ‖ … cut to `len`.
pub(crate) fn evp_bytes_to_key(passphrase: &[u8], salt: &[u8], len: usize) -> Zeroizing<Vec<u8>> {
    let mut key = Zeroizing::new(Vec::with_capacity(len + 16));
    let mut previous: Option<[u8; 16]> = None;
    while key.len() < len {
        let mut context = md5::Context::new();
        if let Some(previous) = &previous {
            context.consume(previous);
        }
        context.consume(passphrase);
        context.consume(salt);
        let digest = context.finalize().0;
        key.extend_from_slice(&digest);
        previous = Some(digest);
    }
    key.truncate(len);
    key
}

fn from_pem(pem: &Pem<'_>, passphrase: Option<&SecretString>) -> Result<PrivateKey, KeyProblem> {
    let der = pem.der(passphrase)?;
    let data = match pem.label {
        PEM_RSA => rsa_keypair(&pkcs1::RsaPrivateKey::from_der(&der).map_err(|_| {
            // A legacy-encrypted body that decrypts to garbage: a wrong
            // passphrase whose padding happened to be valid.
            if pem.legacy_encrypted() {
                KeyProblem::WrongPassphrase
            } else {
                KeyProblem::Damaged
            }
        })?)?,
        PEM_SEC1 => {
            let key = sec1::EcPrivateKey::from_der(&der).map_err(|_| {
                if pem.legacy_encrypted() {
                    KeyProblem::WrongPassphrase
                } else {
                    KeyProblem::Damaged
                }
            })?;
            let curve = key
                .parameters
                .and_then(|parameters| parameters.named_curve())
                .ok_or_else(|| KeyProblem::Algorithm("EC (explicit curve)".to_owned()))?;
            KeypairData::Ecdsa(ec_keypair_sec1(curve, &key)?)
        }
        PEM_PKCS8 => pkcs8_keypair(&der)?,
        PEM_PKCS8_ENCRYPTED => {
            let passphrase = passphrase.ok_or(KeyProblem::PassphraseNeeded)?;
            let encrypted = pkcs8::EncryptedPrivateKeyInfoRef::from_der(&der)
                .map_err(|_| KeyProblem::Damaged)?;
            check_pbes2(&encrypted.encryption_algorithm)?;
            let document = encrypted
                .decrypt(passphrase.expose_secret().as_bytes())
                .map_err(|_| KeyProblem::WrongPassphrase)?;
            pkcs8_keypair(document.as_bytes())?
        }
        _ => return Err(KeyProblem::NotAKey),
    };
    PrivateKey::new(data, "").map_err(|_| KeyProblem::Damaged)
}

/// Refuses PBES2 key derivations above the limits, and other schemes.
fn check_pbes2(scheme: &pkcs8::pkcs5::EncryptionScheme) -> Result<(), KeyProblem> {
    use pkcs8::pkcs5::pbes2::Kdf;
    let pkcs8::pkcs5::EncryptionScheme::Pbes2(parameters) = scheme else {
        return Err(KeyProblem::Cipher("PBES1".to_owned()));
    };
    match &parameters.kdf {
        Kdf::Pbkdf2(pbkdf2) => {
            if pbkdf2.iteration_count > MAX_PBKDF2_ITERATIONS {
                return Err(KeyProblem::KdfTooExpensive);
            }
        }
        Kdf::Scrypt(scrypt) => {
            let bytes = 128u64
                .saturating_mul(u64::from(scrypt.block_size))
                .saturating_mul(scrypt.cost_parameter)
                .saturating_mul(u64::from(scrypt.parallelization).max(1));
            if bytes > MAX_SCRYPT_BYTES {
                return Err(KeyProblem::KdfTooExpensive);
            }
        }
        _ => {
            return Err(KeyProblem::Cipher(
                "PBES2 (unknown key derivation)".to_owned(),
            ));
        }
    }
    Ok(())
}

/// An RSA key from its PKCS#1 structure.
fn rsa_keypair(key: &pkcs1::RsaPrivateKey<'_>) -> Result<KeypairData, KeyProblem> {
    let uint = |value: pkcs1::UintRef<'_>| rsa::BoxedUint::from_be_slice_vartime(value.as_bytes());
    let private = rsa::RsaPrivateKey::from_components(
        uint(key.modulus),
        uint(key.public_exponent),
        uint(key.private_exponent),
        vec![uint(key.prime1), uint(key.prime2)],
    )
    .map_err(|_| KeyProblem::Damaged)?;
    let bits = rsa::traits::PublicKeyParts::size(&private) * 8;
    if bits < super::MIN_RSA_BITS as usize {
        return Err(KeyProblem::RsaTooSmall);
    }
    let keypair =
        ssh_key::private::RsaKeypair::try_from(private).map_err(|_| KeyProblem::Damaged)?;
    Ok(KeypairData::Rsa(keypair))
}

/// A key from a PKCS#8 `PrivateKeyInfo` (RSA, EC on the NIST curves, Ed25519).
fn pkcs8_keypair(der: &[u8]) -> Result<KeypairData, KeyProblem> {
    let info = pkcs8::PrivateKeyInfoRef::from_der(der).map_err(|_| KeyProblem::Damaged)?;
    let oid = info.algorithm.oid;
    if oid == pkcs1::ALGORITHM_OID {
        let key = pkcs1::RsaPrivateKey::from_der(info.private_key.as_bytes())
            .map_err(|_| KeyProblem::Damaged)?;
        return rsa_keypair(&key);
    }
    if oid == ED25519_OID {
        let bytes =
            ed25519_dalek::pkcs8::KeypairBytes::try_from(info).map_err(|_| KeyProblem::Damaged)?;
        let private = Ed25519PrivateKey::from_bytes(&bytes.secret_key);
        return Ok(KeypairData::Ed25519(Ed25519Keypair {
            public: private.clone().into(),
            private,
        }));
    }
    if oid == sec1::ALGORITHM_OID {
        let curve = info
            .algorithm
            .parameters_oid()
            .map_err(|_| KeyProblem::Algorithm("EC (explicit curve)".to_owned()))?;
        let keypair = match curve {
            curve if curve == p256::NistP256::OID => {
                let secret = p256::SecretKey::try_from(info).map_err(|_| KeyProblem::Damaged)?;
                EcdsaKeypair::NistP256 {
                    public: secret.public_key().into(),
                    private: secret.into(),
                }
            }
            curve if curve == p384::NistP384::OID => {
                let secret = p384::SecretKey::try_from(info).map_err(|_| KeyProblem::Damaged)?;
                EcdsaKeypair::NistP384 {
                    public: secret.public_key().into(),
                    private: secret.into(),
                }
            }
            curve if curve == p521::NistP521::OID => {
                let secret = p521::SecretKey::try_from(info).map_err(|_| KeyProblem::Damaged)?;
                EcdsaKeypair::NistP521 {
                    public: secret.public_key().into(),
                    private: secret.into(),
                }
            }
            other => return Err(KeyProblem::Algorithm(format!("EC ({other})"))),
        };
        return Ok(KeypairData::Ecdsa(keypair));
    }
    Err(KeyProblem::Algorithm(algorithm_name(oid)))
}

/// id-Ed25519 (RFC 8410).
const ED25519_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.112");

/// A readable name for a PKCS#8 algorithm this app does not use.
fn algorithm_name(oid: ObjectIdentifier) -> String {
    match oid.to_string().as_str() {
        "1.2.840.10040.4.1" => "DSA".to_owned(),
        "1.3.101.113" => "Ed448".to_owned(),
        "1.3.101.110" | "1.3.101.111" => "X25519/X448 (not a signing key)".to_owned(),
        other => other.to_owned(),
    }
}

/// An EC key from its SEC1 structure on a named NIST curve.
fn ec_keypair_sec1(
    curve: ObjectIdentifier,
    key: &sec1::EcPrivateKey<'_>,
) -> Result<EcdsaKeypair, KeyProblem> {
    Ok(match curve {
        curve if curve == p256::NistP256::OID => {
            let secret =
                p256::SecretKey::from_slice(key.private_key).map_err(|_| KeyProblem::Damaged)?;
            EcdsaKeypair::NistP256 {
                public: secret.public_key().into(),
                private: secret.into(),
            }
        }
        curve if curve == p384::NistP384::OID => {
            let secret =
                p384::SecretKey::from_slice(key.private_key).map_err(|_| KeyProblem::Damaged)?;
            EcdsaKeypair::NistP384 {
                public: secret.public_key().into(),
                private: secret.into(),
            }
        }
        curve if curve == p521::NistP521::OID => {
            let secret =
                p521::SecretKey::from_slice(key.private_key).map_err(|_| KeyProblem::Damaged)?;
            EcdsaKeypair::NistP521 {
                public: secret.public_key().into(),
                private: secret.into(),
            }
        }
        other => return Err(KeyProblem::Algorithm(format!("EC ({other})"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The headers of a `.ppk` v3 file with these Argon2 lines (the key
    /// lines are not read by the check).
    fn ppk(argon: &[&str]) -> String {
        let mut lines = vec![
            "PuTTY-User-Key-File-3: ssh-ed25519",
            "Encryption: aes256-cbc",
            "Comment: test",
            "Public-Lines: 2",
            "AAAAC3NzaC1lZDI1NTE5AAAAI",
            "Argon2-Passes: 1",
            "Key-Derivation: Argon2id",
        ];
        lines.extend_from_slice(argon);
        lines.extend([
            "Argon2-Salt: 00",
            "Private-Lines: 1",
            "AAAA",
            "Private-MAC: 00",
        ]);
        lines.join("\n")
    }

    #[test]
    fn the_argon2_limits_of_a_ppk_cannot_be_dodged() {
        let ok = [
            "Argon2-Memory: 8192",
            "Argon2-Passes: 13",
            "Argon2-Parallelism: 1",
        ];
        assert_eq!(check_ppk_kdf(&ppk(&ok)), Ok(()));
        // A line inside a base64 block (Public-Lines above) is not a header.
        for (line, problem) in [
            ("Argon2-Memory: 4194304", KeyProblem::KdfTooExpensive),
            ("Argon2-Passes: 4294967295", KeyProblem::KdfTooExpensive),
            ("Argon2-Parallelism: 1000", KeyProblem::KdfTooExpensive),
            ("Argon2-Passes: -1", KeyProblem::Damaged),
            ("Argon2-Passes:  13", KeyProblem::Damaged),
        ] {
            let mut argon = ok.to_vec();
            argon.retain(|known| known.split(':').next() != line.split(':').next());
            argon.push(line);
            assert_eq!(check_ppk_kdf(&ppk(&argon)), Err(problem), "{line}");
        }
        // ssh-key keeps the last of two lines: a low first value fools no one.
        let twice = [
            "Argon2-Memory: 8192",
            "Argon2-Passes: 8",
            "Argon2-Passes: 4294967295",
            "Argon2-Parallelism: 1",
        ];
        assert_eq!(check_ppk_kdf(&ppk(&twice)), Err(KeyProblem::Damaged));
    }
}
