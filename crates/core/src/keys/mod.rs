//! Keys of the Keychain and of the short "+ -> Key" path of the host panel
//! (UI 3.1, 7; docs/data-model.md, section 9.1): parsing OpenSSH private
//! keys (Ed25519, ECDSA P-256/P-384/P-521, RSA; encrypted with a passphrase
//! or not) with `ssh-key`, fingerprints, the derived public key, and `key`
//! records.
//!
//! The stored form is the OpenSSH format. PEM (PKCS#1, PKCS#8, SEC1) and
//! PuTTY `.ppk` are converted on import ([`convert`], [`prepare`]); new keys
//! come from [`generate`]. A key is matched by its SHA256 fingerprint
//! (natural key, docs/data-model.md, section 18.6): importing an existing
//! key returns the existing record.
//!
//! Decrypting an encrypted key runs bcrypt-pbkdf with the rounds stored in
//! the key: the facade calls [`passphrase_opens`] without holding the core
//! mutex.
//!
//! Owner: S4. The facade methods (`Core::keys_list`, `Core::key_import`,
//! `Core::save_key_passphrase`) are the contract.

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};
use ssh_key::{Algorithm, EcdsaCurve, Kdf, PrivateKey};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::{FieldError, ValidationReason};
use crate::known_hosts;
use crate::model::key::{self, MAX_LABEL_CHARS, MAX_PASSPHRASE_BYTES, MAX_PRIVATE_KEY_BYTES};
use crate::model::text;
use crate::model::time::{format_unix_ms, uuid_v7_timestamp};
use crate::service::dto::KeyView;

pub mod certificate;
pub mod convert;
pub mod generate;

/// The first line of an OpenSSH private key.
pub const OPENSSH_BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";

/// Smallest RSA modulus accepted, in bits.
pub const MIN_RSA_BITS: u32 = 2048;

/// Most bcrypt-pbkdf rounds accepted in an encrypted key. OpenSSH writes 16
/// by default; `ssh-keygen -a` values of a few hundred are common. The rounds
/// come from the key file, and decryption cannot be interrupted, so a crafted
/// key with billions of rounds would tie up a thread for days: such keys are
/// refused before anything is decrypted.
pub const MAX_KDF_ROUNDS: u32 = 1024;

/// Algorithm names of keys this app can use (docs/data-model.md, section
/// 9.1; the security keys since stage 3).
pub const SUPPORTED_ALGORITHMS: &[&str] = &[
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "ssh-rsa",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// A parsed private key: everything the import needs, without decrypting.
pub struct ParsedKey {
    /// The normalized text as it will be stored (still encrypted when it was).
    pub text: Zeroizing<String>,
    pub algorithm: String,
    /// `<algorithm> <base64> [comment]`.
    pub public_key: String,
    pub fingerprint: String,
    pub comment: String,
    pub encrypted: bool,
}

impl std::fmt::Debug for ParsedKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedKey")
            .field("algorithm", &self.algorithm)
            .field("encrypted", &self.encrypted)
            .finish_non_exhaustive()
    }
}

/// The stored form of pasted or file text: surrounding whitespace removed,
/// CRLF and CR turned into LF, trailing whitespace of each line removed,
/// except in a PuTTY `.ppk`: its `Comment: ` header may be empty and its MAC
/// covers the comment, so its lines are kept as they are.
pub fn normalize_text(raw: &str) -> Zeroizing<String> {
    // One pass into buffers of their final capacity: neither output is longer
    // than its input, so nothing is reallocated. `str::replace` would leave
    // unwiped copies of the key (its result and the buffers it outgrows).
    let mut unified = Zeroizing::new(String::with_capacity(raw.len()));
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            chars.next_if_eq(&'\n');
            unified.push('\n');
        } else {
            unified.push(c);
        }
    }
    let mut out = Zeroizing::new(String::with_capacity(unified.len()));
    let text = unified.trim();
    let ppk = text.starts_with("PuTTY-User-Key-File-");
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(if ppk { line } else { line.trim_end() });
    }
    out
}

/// `true` when decrypting the key costs at most [`MAX_KDF_ROUNDS`] rounds.
pub fn kdf_allowed(private: &PrivateKey) -> bool {
    match private.kdf() {
        Kdf::Bcrypt { rounds, .. } => *rounds <= MAX_KDF_ROUNDS,
        _ => true,
    }
}

/// `true` when the key's algorithm (and for RSA its size) is allowed.
fn allowed(private: &PrivateKey) -> bool {
    match private.algorithm() {
        Algorithm::Ed25519 => true,
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256 | EcdsaCurve::NistP384 | EcdsaCurve::NistP521,
        } => true,
        // Security keys (FIDO2, stage 3): imported and stored; signing goes
        // through the authenticator, which the SSH layer does not reach yet.
        Algorithm::SkEd25519 | Algorithm::SkEcdsaSha2NistP256 => true,
        Algorithm::Rsa { .. } => private
            .public_key()
            .key_data()
            .rsa()
            .is_some_and(|rsa| rsa.key_size() >= MIN_RSA_BITS),
        _ => false,
    }
}

/// Parses an OpenSSH private key (decision 9). Errors use the field
/// `private_key`: `required` (empty), `too_long` (over 32768 bytes after
/// normalization), `invalid` (not an OpenSSH private key, PEM included),
/// `not_allowed` (another algorithm, RSA under 2048 bits, or more than
/// [`MAX_KDF_ROUNDS`] bcrypt-pbkdf rounds).
pub fn parse(raw: &str) -> Result<ParsedKey, FieldError> {
    let refuse = |reason| FieldError::new("private_key", reason);
    let text = normalize_text(raw);
    if text.is_empty() {
        return Err(refuse(ValidationReason::Required));
    }
    if text.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(refuse(ValidationReason::TooLong));
    }
    if !text.starts_with(OPENSSH_BEGIN) || text.contains('\0') {
        return Err(refuse(ValidationReason::Invalid));
    }
    let private =
        PrivateKey::from_openssh(text.as_bytes()).map_err(|_| refuse(ValidationReason::Invalid))?;
    if !allowed(&private) || !kdf_allowed(&private) {
        return Err(refuse(ValidationReason::NotAllowed));
    }
    let public = private.public_key();
    let blob = public
        .to_bytes()
        .map_err(|_| refuse(ValidationReason::Invalid))?;
    let algorithm = private.algorithm().as_str().to_owned();
    let comment = public.comment().as_str_lossy().to_owned();
    let mut public_key = format!("{algorithm} {}", known_hosts::encode_blob(&blob));
    let comment_for_line: String = comment
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    if !comment_for_line.is_empty() {
        public_key.push(' ');
        public_key.push_str(&comment_for_line);
    }
    Ok(ParsedKey {
        fingerprint: known_hosts::fingerprint_of_blob(&blob),
        encrypted: private.is_encrypted(),
        algorithm,
        public_key,
        comment,
        text,
    })
}

/// The stored OpenSSH form of a pasted or chosen key in any supported format
/// ([`convert`]): an OpenSSH key as it is, a PEM or `.ppk` key converted
/// (and, when it came encrypted, encrypted again with `passphrase`). The
/// errors are those of [`parse`] and of the passphrase (`passphrase`
/// `required` or `invalid`). Slow for encrypted sources: call without the
/// core mutex.
pub fn prepare(
    raw: &str,
    passphrase: Option<&SecretString>,
) -> Result<Zeroizing<String>, FieldError> {
    let text = normalize_text(raw);
    if text.is_empty() {
        return Err(FieldError::new("private_key", ValidationReason::Required));
    }
    if text.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(FieldError::new("private_key", ValidationReason::TooLong));
    }
    convert::to_openssh(&text, passphrase).map_err(problem_field)
}

/// The field error of a [`convert::KeyProblem`].
pub fn problem_field(problem: convert::KeyProblem) -> FieldError {
    use convert::KeyProblem;
    match problem {
        KeyProblem::NotAKey | KeyProblem::PublicKey | KeyProblem::Damaged => {
            FieldError::new("private_key", ValidationReason::Invalid)
        }
        KeyProblem::Algorithm(_)
        | KeyProblem::RsaTooSmall
        | KeyProblem::Cipher(_)
        | KeyProblem::KdfTooExpensive => {
            FieldError::new("private_key", ValidationReason::NotAllowed)
        }
        KeyProblem::PassphraseNeeded => refuse_passphrase(ValidationReason::Required),
        KeyProblem::WrongPassphrase => refuse_passphrase(ValidationReason::Invalid),
    }
}

/// What the key panel shows for a pasted or chosen text before it is saved
/// (UI 7.2, 7.3): the source format and whether it needs a passphrase, or
/// why it is not a usable key (the type "unknown" with the reason).
pub fn inspect(raw: &str) -> Result<(convert::Format, bool), convert::KeyProblem> {
    let text = normalize_text(raw);
    let (format, encrypted) = convert::detect(&text).ok_or_else(|| {
        if convert::is_public_key(&text) {
            convert::KeyProblem::PublicKey
        } else {
            convert::KeyProblem::NotAKey
        }
    })?;
    if !encrypted {
        // Reads the whole key: damaged keys and refused algorithms show now.
        convert::to_openssh(&text, None).and_then(|openssh| {
            parse(&openssh)
                .map(drop)
                .map_err(|error| match error.reason {
                    ValidationReason::NotAllowed => match openssh_algorithm(&openssh).as_str() {
                        "ssh-rsa" => convert::KeyProblem::RsaTooSmall,
                        other => convert::KeyProblem::Algorithm(other.to_owned()),
                    },
                    _ => convert::KeyProblem::Damaged,
                })
        })?;
    }
    Ok((format, encrypted))
}

/// The algorithm of an OpenSSH private key text (for the "unknown" reason).
fn openssh_algorithm(text: &str) -> String {
    PrivateKey::from_openssh(text.as_bytes())
        .map(|private| private.algorithm().as_str().to_owned())
        .unwrap_or_default()
}

/// Why a passphrase was refused (field `passphrase`).
fn refuse_passphrase(reason: ValidationReason) -> FieldError {
    FieldError::new("passphrase", reason)
}

/// `Ok` when `passphrase` opens the encrypted key `text`: `passphrase`
/// `not_allowed` for an unencrypted key, `invalid` for a wrong passphrase or
/// an undecodable key (also one with more than [`MAX_KDF_ROUNDS`] rounds,
/// which is never decrypted), `too_long` over 1024 bytes. Slow
/// (bcrypt-pbkdf): call without the core mutex.
pub fn passphrase_opens(text: &str, passphrase: &SecretString) -> Result<(), FieldError> {
    if passphrase.expose_secret().len() > MAX_PASSPHRASE_BYTES {
        return Err(refuse_passphrase(ValidationReason::TooLong));
    }
    let private = PrivateKey::from_openssh(text.as_bytes())
        .map_err(|_| refuse_passphrase(ValidationReason::Invalid))?;
    if !private.is_encrypted() {
        return Err(refuse_passphrase(ValidationReason::NotAllowed));
    }
    if !kdf_allowed(&private) {
        return Err(refuse_passphrase(ValidationReason::Invalid));
    }
    private
        .decrypt(passphrase.expose_secret().as_bytes())
        .map(drop)
        .map_err(|_| refuse_passphrase(ValidationReason::Invalid))
}

/// The passphrase rules of `key_import` for a parsed key: an encrypted key
/// needs one (`required`) that opens it (`invalid`); an unencrypted key
/// takes none (`not_allowed`). An empty passphrase counts as absent.
pub fn check_passphrase(
    parsed: &ParsedKey,
    passphrase: Option<&SecretString>,
) -> Result<(), FieldError> {
    let passphrase = passphrase.filter(|value| !value.expose_secret().is_empty());
    match (parsed.encrypted, passphrase) {
        (true, None) => Err(refuse_passphrase(ValidationReason::Required)),
        (true, Some(passphrase)) => passphrase_opens(&parsed.text, passphrase),
        (false, None) => Ok(()),
        (false, Some(_)) => Err(refuse_passphrase(ValidationReason::NotAllowed)),
    }
}

/// The label of an imported key: the given one (`string(256)`, errors on
/// `label`), else the key comment, else the algorithm name.
pub fn label_for(given: Option<&str>, parsed: &ParsedKey) -> Result<String, FieldError> {
    if let Some(given) = given {
        match text::string(given, MAX_LABEL_CHARS) {
            Ok(Some(label)) => return Ok(label),
            Ok(None) => {}
            Err(reason) => return Err(FieldError::new("label", reason)),
        }
    }
    let comment: String = parsed
        .comment
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    Ok(text::string(&comment, MAX_LABEL_CHARS)
        .ok()
        .flatten()
        .unwrap_or_else(|| parsed.algorithm.clone()))
}

/// `true` when the stored private key text is encrypted (`None` when it
/// cannot be parsed).
pub fn text_is_encrypted(text: &str) -> Option<bool> {
    PrivateKey::from_openssh(text.as_bytes())
        .ok()
        .map(|private| private.is_encrypted())
}

/// Algorithm and fingerprint from a stored one-line public key.
pub fn public_key_info(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let algorithm = parts.next()?;
    let blob = known_hosts::decode_blob(parts.next()?)?;
    let actual = known_hosts::blob_key_type(&blob)?;
    (actual == algorithm).then(|| (actual, known_hosts::fingerprint_of_blob(&blob)))
}

/// The SHA256 fingerprint of a stored key record (from its public key, else
/// from its private key).
pub fn record_fingerprint(data: &Map<String, Value>) -> Option<String> {
    if let Some((_, fingerprint)) = key::public_key(data).and_then(public_key_info) {
        return Some(fingerprint);
    }
    let private = PrivateKey::from_openssh(key::private_key(data)?.as_bytes()).ok()?;
    let blob = private.public_key().to_bytes().ok()?;
    Some(known_hosts::fingerprint_of_blob(&blob))
}

/// The view of a key record (no secrets, docs/security.md, section 9.5).
pub fn view(vault_id: Uuid, record_id: Uuid, hlc_ms: u64, data: &Map<String, Value>) -> KeyView {
    let info = key::public_key(data).and_then(public_key_info);
    let private = key::private_key(data);
    let parsed_private = private.and_then(|text| PrivateKey::from_openssh(text.as_bytes()).ok());
    let (algorithm, fingerprint) = match (&info, &parsed_private) {
        (Some((algorithm, fingerprint)), _) => (algorithm.clone(), Some(fingerprint.clone())),
        (None, Some(private)) => (
            private.algorithm().as_str().to_owned(),
            private
                .public_key()
                .to_bytes()
                .ok()
                .map(|blob| known_hosts::fingerprint_of_blob(&blob)),
        ),
        (None, None) => ("unknown".to_owned(), None),
    };
    let algorithm = if SUPPORTED_ALGORITHMS.contains(&algorithm.as_str()) {
        algorithm
    } else {
        "unknown".to_owned()
    };
    KeyView {
        vault_id,
        record_id,
        created_at: uuid_v7_timestamp(&record_id),
        modified_at: format_unix_ms(hlc_ms),
        label: key::label(data).to_owned(),
        algorithm,
        fingerprint,
        encrypted: parsed_private
            .as_ref()
            .is_some_and(PrivateKey::is_encrypted),
        passphrase_saved: key::passphrase(data).is_some(),
    }
}

/// The `fido2` member of a security key record (docs/data-model.md, 9.3):
/// `{"resident": bool}` from the key's flags (OpenSSH's `SSH_SK_RESIDENT_KEY`
/// = 0x20); `None` for other keys.
pub fn fido2_member(text: &str) -> Option<Value> {
    const RESIDENT: u8 = 0x20;
    let private = PrivateKey::from_openssh(text.as_bytes()).ok()?;
    let flags = match private.key_data() {
        ssh_key::private::KeypairData::SkEd25519(sk) => sk.flags(),
        ssh_key::private::KeypairData::SkEcdsaSha2NistP256(sk) => sk.flags(),
        _ => return None,
    };
    Some(serde_json::json!({"resident": flags & RESIDENT != 0}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_normalization() {
        assert_eq!(
            normalize_text("\r\n  -----BEGIN X-----  \r\nabc \r\n-----END X-----\r\n\r\n").as_str(),
            "-----BEGIN X-----\nabc\n-----END X-----"
        );
        assert_eq!(normalize_text("a\rb").as_str(), "a\nb");
        // A .ppk keeps its lines: an empty comment, trailing spaces.
        assert_eq!(
            normalize_text("PuTTY-User-Key-File-3: ssh-ed25519\r\nComment: \r\nX: a  \r\n")
                .as_str(),
            "PuTTY-User-Key-File-3: ssh-ed25519\nComment: \nX: a"
        );
        assert_eq!(normalize_text("   ").as_str(), "");
        // Mixed endings, a lone CR before CRLF, non-ASCII text.
        assert_eq!(
            normalize_text("a\r\r\nb\n\rc\u{00e9}\r\nd").as_str(),
            "a\n\nb\n\nc\u{00e9}\nd"
        );
    }

    #[test]
    fn non_openssh_text_is_refused() {
        let reason = |raw: &str| parse(raw).unwrap_err().reason;
        assert_eq!(reason(""), ValidationReason::Required);
        assert_eq!(reason(" \n "), ValidationReason::Required);
        assert_eq!(
            reason("-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----"),
            ValidationReason::Invalid
        );
        assert_eq!(
            reason(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nnot base64\n-----END OPENSSH PRIVATE KEY-----"
            ),
            ValidationReason::Invalid
        );
        assert_eq!(
            reason("ssh-ed25519 AAAA comment"),
            ValidationReason::Invalid
        );
        let huge = format!("{OPENSSH_BEGIN}\n{}", "A".repeat(MAX_PRIVATE_KEY_BYTES));
        assert_eq!(reason(&huge), ValidationReason::TooLong);
        assert_eq!(parse("x").unwrap_err().field, "private_key");
    }

    #[test]
    fn public_key_lines() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[3u8; 32]);
        let line = format!(
            "ssh-ed25519 {} me@example.com",
            known_hosts::encode_blob(&blob)
        );
        let (algorithm, fingerprint) = public_key_info(&line).unwrap();
        assert_eq!(algorithm, "ssh-ed25519");
        assert_eq!(fingerprint, known_hosts::fingerprint_of_blob(&blob));
        let wrong = format!("ssh-rsa {}", known_hosts::encode_blob(&blob));
        assert_eq!(public_key_info(&wrong), None);
        assert_eq!(public_key_info("ssh-ed25519"), None);
    }
}
