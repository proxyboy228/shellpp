//! The server's own cryptography (docs/security.md, sections 5.1, 5.5,
//! 14.1): Argon2id verifiers of the clients' auth keys, SHA-256 hashes of
//! tokens and codes, the keys derived from `SHELLPP_SERVER_SECRET`, the fake
//! pre-login salt, the TOTP secret at rest and TOTP checks. Only vetted
//! crates; nothing here is a construction of its own.

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Keys derived from the server secret (HKDF-SHA256, empty salt).
#[derive(Clone)]
pub struct ServerKeys {
    prelogin_salt_key: Zeroizing<[u8; 32]>,
    totp_at_rest_key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for ServerKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerKeys(..)")
    }
}

fn hkdf32(ikm: &[u8], label: &[u8]) -> Zeroizing<[u8; 32]> {
    let hk = hkdf::Hkdf::<Sha256>::new(None, ikm);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(label, out.as_mut())
        .expect("32 bytes is a valid HKDF output length");
    out
}

impl ServerKeys {
    pub fn derive(server_secret: &[u8]) -> Self {
        Self {
            prelogin_salt_key: hkdf32(server_secret, b"shellpp-server/v1/prelogin-salt"),
            totp_at_rest_key: hkdf32(server_secret, b"shellpp-server/v1/totp-at-rest"),
        }
    }

    /// The stable fake salt of an unknown email (D11):
    /// `HMAC-SHA256(prelogin_salt_key, normalized_email)[0..16]`.
    pub fn fake_salt(&self, normalized_email: &str) -> [u8; 16] {
        let mut mac =
            <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.prelogin_salt_key.as_ref())
                .expect("any key length");
        mac.update(normalized_email.as_bytes());
        let full = mac.finalize().into_bytes();
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&full[..16]);
        salt
    }

    /// Encrypts a TOTP secret for `users.totp_secret_enc`: a wrap blob
    /// (`0x01` ‖ nonce ‖ ciphertext ‖ tag) bound to the user.
    pub fn seal_totp(&self, user_id: Uuid, secret: &[u8]) -> Vec<u8> {
        let cipher = self.totp_cipher();
        let nonce_bytes = random_bytes::<24>();
        let nonce = XNonce::from(nonce_bytes);
        let aad = totp_aad(user_id);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: secret,
                    aad: &aad,
                },
            )
            .expect("XChaCha20-Poly1305 encryption cannot fail");
        let mut blob = Vec::with_capacity(1 + 24 + ciphertext.len());
        blob.push(0x01);
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ciphertext);
        blob
    }

    /// Opens a sealed TOTP secret; `None` for anything that does not open.
    pub fn open_totp(&self, user_id: Uuid, blob: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        if blob.len() < 1 + 24 + 16 || blob[0] != 0x01 {
            return None;
        }
        let cipher = self.totp_cipher();
        let nonce = XNonce::from(<[u8; 24]>::try_from(&blob[1..25]).ok()?);
        let aad = totp_aad(user_id);
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &blob[25..],
                    aad: &aad,
                },
            )
            .ok()
            .map(Zeroizing::new)
    }

    /// The cipher of TOTP secrets at rest, keyed by reference: no copy of
    /// the key is left behind (the cipher wipes its own on drop).
    fn totp_cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(<&Key>::from(&*self.totp_at_rest_key))
    }
}

fn totp_aad(user_id: Uuid) -> Vec<u8> {
    let mut aad = b"shellpp-server/v1/wrap/totp".to_vec();
    aad.extend_from_slice(user_id.as_bytes());
    aad
}

/// `n` bytes from the OS CSPRNG.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    bytes
}

/// Normalized email: trimmed, NFC, lowercase (docs/storage-format.md, section 3).
pub fn normalize_email(email: &str) -> String {
    email.trim().nfc().collect::<String>().to_lowercase()
}

/// SHA-256 of a token, ticket or code as stored.
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A new token: the prefix and base64url of 32 random bytes.
pub fn new_token(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        data_encoding::BASE64URL_NOPAD.encode(&random_bytes::<32>())
    )
}

/// Crockford base32 (no padding), the form of registration codes and
/// invitation tokens (docs/security.md, section 5.8).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A new registration code: 16 random bytes, 26 Crockford base32 characters.
pub fn new_code() -> String {
    crockford(&random_bytes::<16>())
}

fn crockford(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(CROCKFORD[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(
            CROCKFORD[((buffer << (5 - bits)) & 31) as usize],
        ));
    }
    out
}

/// The canonical text of a code typed by a user: no spaces or hyphens,
/// uppercase, `I`/`L` as `1`, `O` as `0`.
pub fn canonical_code(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        })
        .collect()
}

/// The Argon2id instance of server verifiers: m = 19456 KiB, t = 2, p = 1
/// over the 32-byte keys (docs/security.md, section 5.4).
fn verifier_argon2() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19_456, 2, 1, None).expect("valid Argon2 parameters"),
    )
}

/// The PHC verifier of an auth key or recovery auth key.
pub fn make_verifier(key: &[u8]) -> String {
    verifier_argon2()
        .hash_password_with_salt(key, &random_bytes::<16>())
        .expect("hashing a 32-byte key cannot fail")
        .to_string()
}

/// Whether `key` matches the PHC verifier.
pub fn check_verifier(verifier: &str, key: &[u8]) -> bool {
    PasswordVerifier::<str>::verify_password(&verifier_argon2(), key, verifier).is_ok()
}

/// Constant-time equality of two byte strings.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// A TOTP secret: 160 random bits (docs/sync-protocol.md, section 5.5).
pub fn new_totp_secret() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(random_bytes::<20>().to_vec())
}

/// RFC 4648 base32 without padding, the form authenticator apps take.
pub fn totp_secret_text(secret: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(secret)
}

/// The `otpauth://` URI of a TOTP secret.
pub fn otpauth_uri(email: &str, secret: &[u8]) -> String {
    let label = percent_encode(&format!("Shell++:{email}"));
    format!(
        "otpauth://totp/{label}?secret={}&issuer=Shell%2B%2B",
        totp_secret_text(secret)
    )
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b':' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Checks a 6-digit TOTP code (RFC 6238, HMAC-SHA1, 30 s, ±1 step) at
/// `unix_seconds`; returns the matching step when it is above `last_step`
/// (a used step is refused again).
pub fn check_totp(
    secret: &[u8],
    code: &str,
    unix_seconds: u64,
    last_step: Option<u64>,
) -> Option<u64> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // The copies of the secret are wiped on drop (here and, with its
    // `zeroize` feature, inside totp-rs), and so are the expected codes.
    let secret = Zeroizing::new(<[u8; 20]>::try_from(secret).ok()?);
    let totp = totp_rs::Builder::new()
        .with_secret(*secret)
        .build_noncompliant();
    let current = unix_seconds / 30;
    let mut found = None;
    for step in [current.saturating_sub(1), current, current + 1] {
        let expected = Zeroizing::new(totp.generate(step * 30).to_string());
        if ct_eq(expected.as_bytes(), code.as_bytes()) && found.is_none() {
            found = Some(step);
        }
    }
    found.filter(|step| last_step.is_none_or(|last| *step > last))
}

/// The code of a 20-byte `secret` at `unix_seconds` (tests).
pub fn totp_code(secret: &[u8; 20], unix_seconds: u64) -> String {
    totp_rs::Builder::new()
        .with_secret(*secret)
        .build_noncompliant()
        .generate(unix_seconds)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifiers_accept_only_their_key() {
        let key = [7u8; 32];
        let verifier = make_verifier(&key);
        assert!(verifier.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(check_verifier(&verifier, &key));
        assert!(!check_verifier(&verifier, &[8u8; 32]));
        assert!(!check_verifier("not a phc string", &key));
    }

    #[test]
    fn fake_salts_are_stable_and_per_email() {
        let keys = ServerKeys::derive(&[1u8; 32]);
        let a = keys.fake_salt("alice@example.com");
        assert_eq!(a, keys.fake_salt("alice@example.com"));
        assert_ne!(a, keys.fake_salt("bob@example.com"));
        // Another secret gives other salts.
        assert_ne!(
            a,
            ServerKeys::derive(&[2u8; 32]).fake_salt("alice@example.com")
        );
    }

    #[test]
    fn hkdf_labels_match_known_values() {
        // RFC 5869-style check of our use: HKDF-SHA256, empty salt.
        let out = hkdf32(&[0x0b; 22], b"shellpp-server/v1/prelogin-salt");
        let again = hkdf32(&[0x0b; 22], b"shellpp-server/v1/prelogin-salt");
        assert_eq!(*out, *again);
        assert_ne!(
            *out,
            *hkdf32(&[0x0b; 22], b"shellpp-server/v1/totp-at-rest")
        );
        // The reference vector of RFC 5869, test case 3 (empty salt and info).
        let hk = hkdf::Hkdf::<Sha256>::new(None, &[0x0b; 22]);
        let mut okm = [0u8; 42];
        hk.expand(&[], &mut okm).unwrap();
        assert_eq!(
            data_encoding::HEXLOWER.encode(&okm),
            "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
        );
    }

    #[test]
    fn totp_secret_at_rest_opens_only_for_its_user() {
        let keys = ServerKeys::derive(&[3u8; 32]);
        let user = Uuid::now_v7();
        let blob = keys.seal_totp(user, b"12345678901234567890");
        assert_eq!(
            keys.open_totp(user, &blob).as_deref().map(Vec::as_slice),
            Some(&b"12345678901234567890"[..])
        );
        assert!(keys.open_totp(Uuid::now_v7(), &blob).is_none());
        let mut tampered = blob.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        assert!(keys.open_totp(user, &tampered).is_none());
        assert!(
            ServerKeys::derive(&[4u8; 32])
                .open_totp(user, &blob)
                .is_none()
        );
    }

    #[test]
    fn totp_matches_rfc_6238_and_refuses_reuse() {
        // RFC 6238, appendix B: SHA-1, secret "12345678901234567890",
        // T = 59 s -> 94287082 (8 digits); the 6-digit code is its tail.
        let secret = b"12345678901234567890";
        assert_eq!(totp_code(secret, 59), "287082");
        assert_eq!(totp_code(secret, 1_111_111_109), "081804");
        // ±1 step.
        let now = 1_111_111_109;
        let step = now / 30;
        assert_eq!(check_totp(secret, "081804", now, None), Some(step));
        assert_eq!(check_totp(secret, "081804", now + 30, None), Some(step));
        assert_eq!(check_totp(secret, "081804", now + 90, None), None);
        // A used step is refused.
        assert_eq!(check_totp(secret, "081804", now, Some(step)), None);
        assert_eq!(check_totp(secret, "08180", now, None), None);
        assert_eq!(check_totp(secret, "abcdef", now, None), None);
    }

    /// Every copy of a TOTP secret and of the at-rest key is wiped on drop,
    /// and the server keys never show in `Debug`.
    #[test]
    fn key_material_is_wiped_and_never_shown() {
        fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        // totp-rs holds the secret while a code is checked; the cipher holds
        // the at-rest key.
        wiped_on_drop::<totp_rs::Totp>();
        wiped_on_drop::<XChaCha20Poly1305>();
        wiped_on_drop::<Zeroizing<[u8; 32]>>();
        let keys = ServerKeys::derive(&random_bytes::<32>());
        let shown = format!("{keys:?}");
        for key in [&*keys.prelogin_salt_key, &*keys.totp_at_rest_key] {
            assert!(!shown.contains(&data_encoding::HEXLOWER.encode(key)));
            assert!(!shown.contains(&format!("{:?}", &key[..4])));
        }
        // A generated secret still seals, opens and checks.
        let user = Uuid::now_v7();
        let secret = new_totp_secret();
        let opened = keys
            .open_totp(user, &keys.seal_totp(user, &secret))
            .expect("opens");
        assert_eq!(opened.as_slice(), secret.as_slice());
        let now = 1_700_000_000;
        let fixed: [u8; 20] = secret.as_slice().try_into().unwrap();
        let code = totp_code(&fixed, now);
        assert_eq!(check_totp(&opened, &code, now, None), Some(now / 30));
    }

    #[test]
    fn codes_and_tokens() {
        let code = new_code();
        assert_eq!(code.len(), 26);
        assert_eq!(canonical_code(&code.to_lowercase()), code);
        assert_eq!(canonical_code("abil-o 0"), "AB1100");
        let token = new_token("shellpp_rt_");
        assert!(token.starts_with("shellpp_rt_"));
        assert_eq!(token.len(), "shellpp_rt_".len() + 43);
        assert_eq!(hash_token("x").len(), 32);
        assert_eq!(crockford(&[0xff; 5]), "ZZZZZZZZ");
    }

    #[test]
    fn emails_and_uris() {
        assert_eq!(normalize_email("  Alice@Example.COM "), "alice@example.com");
        assert_eq!(
            otpauth_uri("alice@example.com", b"12345678901234567890"),
            "otpauth://totp/Shell%2B%2B:alice%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Shell%2B%2B"
        );
    }
}
