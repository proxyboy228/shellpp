//! FIDO2 security keys (`sk-ssh-ed25519@openssh.com`,
//! `sk-ecdsa-sha2-nistp256@openssh.com`; OpenSSH's PROTOCOL.u2f; D37, D50,
//! run 8 decision D143): signing through an [`Authenticator`].
//!
//! - The private key file holds no secret: the application (the WebAuthn
//!   relying party id, `ssh:` by default), the key handle (the credential
//!   id) and the flags. The authenticator signs `SHA-256(application) ||
//!   flags || counter || SHA-256(data)`: its authenticator data followed by
//!   the client data hash of the data to sign.
//! - The SSH signature is the authenticator's signature (Ed25519: 64 bytes;
//!   ECDSA: `mpint r, mpint s` from its DER form) followed by the flags and
//!   the counter it reported ([`signature`]).
//! - The host app provides the authenticator: the system's WebAuthn API on
//!   Windows, CTAP2 over HID elsewhere (D50). Without one a security key is
//!   unusable and the next method runs.
//!
//! Nothing here logs key handles, data or signatures.

use std::sync::Arc;

use russh::keys::agent::AgentIdentity;
use russh::keys::ssh_encoding::Encode as _;
use russh::keys::ssh_key::private::KeypairData;
use russh::keys::ssh_key::{Algorithm, HashAlg, PrivateKey, Signature};

/// The key flag "user presence required" (OpenSSH `SSH_SK_USER_PRESENCE_REQD`).
pub const USER_PRESENCE_REQUIRED: u8 = 0x01;
/// The key flag "user verification required" (a PIN or a fingerprint;
/// OpenSSH `SSH_SK_USER_VERIFICATION_REQD`).
pub const USER_VERIFICATION_REQUIRED: u8 = 0x04;

/// The algorithm of a security key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkAlgorithm {
    Ed25519,
    EcdsaP256,
}

/// One signature asked of the authenticator.
#[derive(Clone)]
pub struct SignRequest {
    pub algorithm: SkAlgorithm,
    /// The relying party id (`ssh:` unless the key was made with another).
    pub application: String,
    /// The credential id.
    pub key_handle: Vec<u8>,
    /// The key's flags ([`USER_PRESENCE_REQUIRED`],
    /// [`USER_VERIFICATION_REQUIRED`]).
    pub flags: u8,
    /// The data to sign; its SHA-256 is the client data hash.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for SignRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignRequest")
            .field("algorithm", &self.algorithm)
            .field("flags", &self.flags)
            .finish_non_exhaustive()
    }
}

/// What the authenticator returned.
#[derive(Clone, PartialEq, Eq)]
pub struct Assertion {
    /// The flags byte of its authenticator data (bit 0: the user was
    /// present; bit 2: the user was verified).
    pub flags: u8,
    /// Its signature counter.
    pub counter: u32,
    /// Ed25519: the 64-byte signature; ECDSA: the DER signature.
    pub signature: Vec<u8>,
}

impl std::fmt::Debug for Assertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assertion")
            .field("flags", &self.flags)
            .field("counter", &self.counter)
            .finish_non_exhaustive()
    }
}

impl Assertion {
    /// The assertion of WebAuthn or CTAP2 authenticator data (`rpIdHash`
    /// 32 bytes, flags, a big-endian counter, extensions) and a signature.
    pub fn from_authenticator_data(auth_data: &[u8], signature: Vec<u8>) -> Option<Self> {
        let flags = *auth_data.get(32)?;
        let counter = u32::from_be_bytes(auth_data.get(33..37)?.try_into().ok()?);
        Some(Self {
            flags,
            counter,
            signature,
        })
    }
}

/// Why a security key did not sign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkError {
    /// No authenticator is reachable (none plugged in, no API).
    #[error("no security key")]
    NoAuthenticator,
    /// The user cancelled, or did not touch the key in time.
    #[error("cancelled")]
    Cancelled,
    /// The authenticator does not hold this credential.
    #[error("unknown credential")]
    UnknownCredential,
    /// The authenticator answered with something that is not a valid
    /// signature of this key.
    #[error("invalid answer")]
    Invalid,
    /// Another failure of the authenticator or its API.
    #[error("failed")]
    Failed,
}

/// A new credential to make (key generation).
#[derive(Clone, Debug)]
pub struct CreateRequest {
    /// The relying party id of the new key (`ssh:`).
    pub application: String,
    /// The key asks for a PIN or a fingerprint at every signature.
    pub user_verification: bool,
}

/// What the authenticator answered for a new credential.
#[derive(Clone)]
pub struct Attested {
    /// The credential id.
    pub key_handle: Vec<u8>,
    /// Its authenticator data with the attested credential data (the
    /// AAGUID, the credential id and the COSE public key).
    pub auth_data: Vec<u8>,
}

impl std::fmt::Debug for Attested {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attested").finish_non_exhaustive()
    }
}

/// A FIDO2 authenticator: the system's WebAuthn API or CTAP2 over HID.
pub trait Authenticator: Send + Sync + 'static {
    /// Asks for one signature. Blocks until the user touches the key (or
    /// enters its PIN) or it fails: call it on a blocking thread.
    fn sign(&self, request: &SignRequest) -> Result<Assertion, SkError>;

    /// Whether a signature for a key with these flags can be asked now
    /// (a device is there, the authenticator handles the key's PIN). The
    /// SSH layer checks it before it offers the key: once the server
    /// accepted it, a failure ends the attempt, while a key that cannot
    /// sign is skipped and the next method runs.
    fn ready(&self, flags: u8) -> bool {
        let _ = flags;
        true
    }

    /// Makes a new credential, Ed25519 when the key can, else ECDSA P-256
    /// (blocking, as [`Authenticator::sign`]). Not every authenticator can.
    fn create(&self, request: &CreateRequest) -> Result<Attested, SkError> {
        let _ = request;
        Err(SkError::Failed)
    }
}

/// One CBOR item of a COSE key: an integer or a byte string (the only
/// kinds a COSE key of Ed25519 or P-256 holds).
#[derive(Debug, PartialEq, Eq)]
enum Cbor<'a> {
    Int(i64),
    Bytes(&'a [u8]),
}

/// The argument of a CBOR head (`major` type, value) and the rest.
fn cbor_head(input: &[u8]) -> Option<(u8, u64, &[u8])> {
    let (&first, rest) = input.split_first()?;
    let major = first >> 5;
    let (value, rest) = match first & 0x1f {
        small @ 0..=23 => (u64::from(small), rest),
        24 => (u64::from(*rest.first()?), &rest[1..]),
        25 => (
            u64::from(u16::from_be_bytes(rest.get(..2)?.try_into().ok()?)),
            &rest[2..],
        ),
        26 => (
            u64::from(u32::from_be_bytes(rest.get(..4)?.try_into().ok()?)),
            &rest[4..],
        ),
        _ => return None,
    };
    Some((major, value, rest))
}

fn cbor_item(input: &[u8]) -> Option<(Cbor<'_>, &[u8])> {
    let (major, value, rest) = cbor_head(input)?;
    match major {
        0 => Some((Cbor::Int(i64::try_from(value).ok()?), rest)),
        1 => Some((Cbor::Int(-1 - i64::try_from(value).ok()?), rest)),
        2 => {
            let len = usize::try_from(value).ok()?;
            Some((Cbor::Bytes(rest.get(..len)?), &rest[len..]))
        }
        _ => None,
    }
}

/// The members of a COSE key (a CBOR map with integer labels) at the start
/// of `input`.
fn cose_key(input: &[u8]) -> Option<Vec<(i64, Cbor<'_>)>> {
    let (major, count, mut rest) = cbor_head(input)?;
    if major != 5 || count > 16 {
        return None;
    }
    let mut members = Vec::new();
    for _ in 0..count {
        let (Cbor::Int(label), after) = cbor_item(rest)? else {
            return None;
        };
        let (value, after) = cbor_item(after)?;
        members.push((label, value));
        rest = after;
    }
    Some(members)
}

/// The COSE public key of attested credential data: after the 37 bytes of
/// authenticator data, the AAGUID (16), the credential id's length (2)
/// and the credential id.
fn attested_public_key(auth_data: &[u8]) -> Option<&[u8]> {
    let flags = *auth_data.get(32)?;
    // Bit 6 (AT): attested credential data is included.
    if flags & 0x40 == 0 {
        return None;
    }
    let id_len = usize::from(u16::from_be_bytes(auth_data.get(53..55)?.try_into().ok()?));
    auth_data.get(55 + id_len..)
}

/// [`key_file`] as OpenSSH text.
pub fn key_file_text(
    request: &CreateRequest,
    attested: &Attested,
    comment: &str,
) -> Result<zeroize::Zeroizing<String>, SkError> {
    let text = key_file(request, attested, comment)?
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .map_err(|_| SkError::Invalid)?;
    Ok(zeroize::Zeroizing::new(text.to_string()))
}

/// The OpenSSH private key file (it holds no secret: the application, the
/// flags and the key handle) of a credential the authenticator made.
pub fn key_file(
    request: &CreateRequest,
    attested: &Attested,
    comment: &str,
) -> Result<PrivateKey, SkError> {
    use russh::keys::ssh_key::{private, public};
    let members = attested_public_key(&attested.auth_data)
        .and_then(cose_key)
        .ok_or(SkError::Invalid)?;
    let member = |label: i64| {
        members
            .iter()
            .find(|(found, _)| *found == label)
            .map(|(_, value)| value)
    };
    let bytes = |label: i64| match member(label) {
        Some(Cbor::Bytes(bytes)) => Some(*bytes),
        _ => None,
    };
    let mut flags = USER_PRESENCE_REQUIRED;
    if request.user_verification {
        flags |= USER_VERIFICATION_REQUIRED;
    }
    let application = request.application.clone();
    let key_handle = attested.key_handle.clone();
    // kty (1), alg (3), crv (-1), x (-2), y (-3).
    let data = match (member(1), member(3), member(-1)) {
        // OKP, EdDSA, Ed25519.
        (Some(Cbor::Int(1)), Some(Cbor::Int(-8)), Some(Cbor::Int(6))) => {
            let x: [u8; 32] = bytes(-2)
                .and_then(|x| x.try_into().ok())
                .ok_or(SkError::Invalid)?;
            let public = public::SkEd25519::new(public::Ed25519PublicKey(x), application);
            KeypairData::SkEd25519(
                private::SkEd25519::new(public, flags, key_handle).map_err(|_| SkError::Invalid)?,
            )
        }
        // EC2, ES256, P-256.
        (Some(Cbor::Int(2)), Some(Cbor::Int(-7)), Some(Cbor::Int(1))) => {
            let (x, y) = bytes(-2).zip(bytes(-3)).ok_or(SkError::Invalid)?;
            if x.len() != 32 || y.len() != 32 {
                return Err(SkError::Invalid);
            }
            let mut point = vec![0x04];
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            let public::EcdsaPublicKey::NistP256(point) =
                public::EcdsaPublicKey::from_sec1_bytes(&point).map_err(|_| SkError::Invalid)?
            else {
                return Err(SkError::Invalid);
            };
            let public = public::SkEcdsaSha2NistP256::new(point, application);
            KeypairData::SkEcdsaSha2NistP256(
                private::SkEcdsaSha2NistP256::new(public, flags, key_handle)
                    .map_err(|_| SkError::Invalid)?,
            )
        }
        _ => return Err(SkError::Invalid),
    };
    PrivateKey::new(data, comment).map_err(|_| SkError::Invalid)
}

/// The sign request of a security key for `data`; `None` for another key.
pub fn request_for(key: &PrivateKey, data: &[u8]) -> Option<SignRequest> {
    let (algorithm, application, flags, key_handle) = match key.key_data() {
        KeypairData::SkEd25519(sk) => (
            SkAlgorithm::Ed25519,
            sk.public().application(),
            sk.flags(),
            sk.key_handle(),
        ),
        KeypairData::SkEcdsaSha2NistP256(sk) => (
            SkAlgorithm::EcdsaP256,
            sk.public().application(),
            sk.flags(),
            sk.key_handle(),
        ),
        _ => return None,
    };
    Some(SignRequest {
        algorithm,
        application: application.to_owned(),
        key_handle: key_handle.to_vec(),
        flags,
        data: data.to_vec(),
    })
}

/// An SSH `mpint` of a big-endian unsigned integer.
fn mpint(bytes: &[u8], out: &mut Vec<u8>) {
    let trimmed = match bytes.iter().position(|byte| *byte != 0) {
        Some(first) => &bytes[first..],
        None => &[],
    };
    let pad = trimmed.first().is_some_and(|byte| byte & 0x80 != 0);
    let len = trimmed.len() + usize::from(pad);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    if pad {
        out.push(0);
    }
    out.extend_from_slice(trimmed);
}

/// `r` and `s` of a DER ECDSA signature (`SEQUENCE { INTEGER, INTEGER }`,
/// short lengths: a P-256 signature is at most 72 bytes).
fn der_integers(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let body = match der {
        [0x30, len, body @ ..] if len & 0x80 == 0 && usize::from(*len) == body.len() => body,
        _ => return None,
    };
    let (r, rest) = der_integer(body)?;
    let (s, rest) = der_integer(rest)?;
    rest.is_empty().then_some((r, s))
}

/// One DER `INTEGER` (at most 33 bytes) and what follows it.
fn der_integer(input: &[u8]) -> Option<(&[u8], &[u8])> {
    match input {
        [0x02, len, rest @ ..]
            if len & 0x80 == 0 && (1..=33).contains(len) && rest.len() >= usize::from(*len) =>
        {
            Some(rest.split_at(usize::from(*len)))
        }
        _ => None,
    }
}

/// The SSH signature of a security key from its authenticator's answer.
pub fn signature(algorithm: SkAlgorithm, assertion: &Assertion) -> Result<Signature, SkError> {
    let mut data = Vec::new();
    let ssh_algorithm = match algorithm {
        SkAlgorithm::Ed25519 => {
            if assertion.signature.len() != 64 {
                return Err(SkError::Invalid);
            }
            data.extend_from_slice(&assertion.signature);
            Algorithm::SkEd25519
        }
        SkAlgorithm::EcdsaP256 => {
            let (r, s) = der_integers(&assertion.signature).ok_or(SkError::Invalid)?;
            mpint(r, &mut data);
            mpint(s, &mut data);
            Algorithm::SkEcdsaSha2NistP256
        }
    };
    data.push(assertion.flags);
    data.extend_from_slice(&assertion.counter.to_be_bytes());
    Signature::new(ssh_algorithm, data).map_err(|_| SkError::Invalid)
}

/// Signs `data` with a security key through `authenticator` and checks the
/// signature against the key's public key (a wrong credential or a broken
/// authenticator is caught here, not by the server). Blocking.
pub fn sign_blocking(
    authenticator: &dyn Authenticator,
    key: &PrivateKey,
    data: &[u8],
) -> Result<Signature, SkError> {
    use russh::keys::signature::Verifier as _;
    let request = request_for(key, data).ok_or(SkError::Invalid)?;
    let assertion = authenticator.sign(&request)?;
    let signature = signature(request.algorithm, &assertion)?;
    key.public_key()
        .key_data()
        .verify(data, &signature)
        .map_err(|_| SkError::Invalid)?;
    Ok(signature)
}

/// Signs for a security key once the server accepted its public key.
pub(crate) struct SkSigner {
    pub(crate) key: Arc<PrivateKey>,
    pub(crate) authenticator: Arc<dyn Authenticator>,
}

/// Why a security key did not sign in an authentication.
#[derive(Debug)]
pub(crate) enum SkSignError {
    Send,
    Sk(SkError),
}

impl From<russh::SendError> for SkSignError {
    fn from(_: russh::SendError) -> Self {
        Self::Send
    }
}

impl russh::Signer for SkSigner {
    type Error = SkSignError;

    async fn auth_sign(
        &mut self,
        _key: &AgentIdentity,
        _hash_alg: Option<HashAlg>,
        mut to_sign: Vec<u8>,
    ) -> Result<Vec<u8>, Self::Error> {
        let key = self.key.clone();
        let authenticator = self.authenticator.clone();
        let data = to_sign.clone();
        let signature =
            tokio::task::spawn_blocking(move || sign_blocking(authenticator.as_ref(), &key, &data))
                .await
                .map_err(|_| SkSignError::Sk(SkError::Failed))?
                .map_err(SkSignError::Sk)?;
        let mut blob = Vec::new();
        signature
            .encode(&mut blob)
            .map_err(|_| SkSignError::Sk(SkError::Invalid))?;
        blob.encode(&mut to_sign)
            .map_err(|_| SkSignError::Sk(SkError::Invalid))?;
        Ok(to_sign)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpints_are_minimal_and_positive() {
        let mut out = Vec::new();
        mpint(&[0x00, 0x00, 0x7f, 0x01], &mut out);
        assert_eq!(out, [0, 0, 0, 2, 0x7f, 0x01]);
        let mut out = Vec::new();
        mpint(&[0x80], &mut out);
        assert_eq!(out, [0, 0, 0, 2, 0x00, 0x80]);
        let mut out = Vec::new();
        mpint(&[0, 0], &mut out);
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn der_signatures_are_split_and_bad_ones_refused() {
        let der = [0x30, 0x07, 0x02, 0x02, 0x00, 0x81, 0x02, 0x01, 0x05];
        assert_eq!(
            der_integers(&der),
            Some((&[0x00u8, 0x81][..], &[0x05u8][..]))
        );
        // Trailing bytes, a wrong tag, a long length.
        assert_eq!(der_integers(&[0x30, 0x03, 0x02, 0x01, 0x05]), None);
        assert_eq!(der_integers(&[0x31, 0x03, 0x02, 0x01, 0x05]), None);
        assert_eq!(der_integers(&[0x30, 0x81, 0x03, 0x02, 0x01, 0x05]), None);
        assert_eq!(der_integers(&[]), None);
    }

    #[test]
    fn authenticator_data_gives_flags_and_counter() {
        let mut auth_data = vec![0u8; 32];
        auth_data.push(0x05);
        auth_data.extend_from_slice(&7u32.to_be_bytes());
        let assertion = Assertion::from_authenticator_data(&auth_data, vec![1; 64]).unwrap();
        assert_eq!((assertion.flags, assertion.counter), (0x05, 7));
        assert!(Assertion::from_authenticator_data(&auth_data[..36], Vec::new()).is_none());
        // The Debug form shows no signature bytes.
        assert!(!format!("{assertion:?}").contains("[1"));
    }

    #[test]
    fn a_wrong_length_ed25519_signature_is_refused() {
        let assertion = Assertion {
            flags: 1,
            counter: 1,
            signature: vec![0; 63],
        };
        assert_eq!(
            signature(SkAlgorithm::Ed25519, &assertion).unwrap_err(),
            SkError::Invalid
        );
    }
}
