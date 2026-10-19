//! Key generation round trips: every type, plain and encrypted, decoded with
//! russh (the client's decoder) and checked against the reported
//! fingerprint and public key line. Passphrases are generated per run.

use russh::keys::{HashAlg, PrivateKey, PublicKey, decode_secret_key};
use secrecy::{ExposeSecret, SecretString};
use shellpp_test_sshd::{KeyType, generate_key};

fn random_passphrase() -> SecretString {
    let bytes: [u8; 12] = rand::random();
    SecretString::from(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

const TYPES: [(KeyType, &str); 3] = [
    (KeyType::Ed25519, "ssh-ed25519"),
    (KeyType::EcdsaP256, "ecdsa-sha2-nistp256"),
    (KeyType::Rsa3072, "ssh-rsa"),
];

fn check_public_parts(
    key: &shellpp_test_sshd::GeneratedKey,
    algorithm: &str,
    comment: &str,
) -> PublicKey {
    let mut fields = key.public_openssh.split(' ');
    assert_eq!(fields.next(), Some(algorithm));
    assert!(fields.next().is_some_and(|blob| blob.starts_with("AAAA")));
    assert_eq!(fields.next(), Some(comment));
    let public = PublicKey::from_openssh(&key.public_openssh).expect("public key line parses");
    assert_eq!(
        public.fingerprint(HashAlg::Sha256).to_string(),
        key.fingerprint
    );
    assert!(key.fingerprint.starts_with("SHA256:"));
    public
}

#[test]
fn plain_keys_of_every_type_round_trip() {
    for (key_type, algorithm) in TYPES {
        let key = generate_key(key_type, "test-comment", None).expect("generates");
        let public = check_public_parts(&key, algorithm, "test-comment");
        let text = key.private_openssh.expose_secret();
        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
        assert!(!text.contains('\r'));
        let parsed = PrivateKey::from_openssh(text).expect("parses");
        assert!(!parsed.is_encrypted());
        let decoded = decode_secret_key(text, None).expect("russh decodes it");
        assert_eq!(decoded.public_key().key_data(), public.key_data());
        assert_eq!(
            decoded.fingerprint(HashAlg::Sha256).to_string(),
            key.fingerprint
        );
    }
}

#[test]
fn rsa_keys_have_3072_bits() {
    let key = generate_key(KeyType::Rsa3072, "rsa", None).expect("generates");
    let decoded = decode_secret_key(key.private_openssh.expose_secret(), None).expect("decodes");
    let rsa = decoded.key_data().rsa().expect("an RSA key");
    assert_eq!(rsa.key_size(), 3072);
}

#[test]
fn encrypted_keys_need_their_passphrase() {
    for (key_type, algorithm) in TYPES {
        let passphrase = random_passphrase();
        let key = generate_key(key_type, "enc", Some(&passphrase)).expect("generates");
        check_public_parts(&key, algorithm, "enc");
        let text = key.private_openssh.expose_secret();
        let parsed = PrivateKey::from_openssh(text).expect("parses");
        assert!(parsed.is_encrypted(), "{algorithm} key is encrypted");
        assert!(!text.contains(passphrase.expose_secret()));

        assert!(decode_secret_key(text, None).is_err(), "no passphrase");
        assert!(
            decode_secret_key(text, Some("not-the-passphrase")).is_err(),
            "wrong passphrase"
        );
        let decoded =
            decode_secret_key(text, Some(passphrase.expose_secret())).expect("right passphrase");
        assert_eq!(
            decoded.fingerprint(HashAlg::Sha256).to_string(),
            key.fingerprint
        );
    }
}

#[test]
fn an_empty_passphrase_leaves_the_key_unencrypted() {
    let empty = SecretString::from("");
    let key = generate_key(KeyType::Ed25519, "", Some(&empty)).expect("generates");
    let parsed = PrivateKey::from_openssh(key.private_openssh.expose_secret()).expect("parses");
    assert!(!parsed.is_encrypted());
}

#[test]
fn keys_are_random_and_debug_hides_the_private_part() {
    let first = generate_key(KeyType::Ed25519, "a", None).expect("generates");
    let second = generate_key(KeyType::Ed25519, "a", None).expect("generates");
    assert_ne!(first.fingerprint, second.fingerprint);
    let debug = format!("{first:?}");
    assert!(debug.contains(&first.fingerprint));
    assert!(!debug.contains("PRIVATE KEY"));
}

#[test]
fn key_type_names() {
    for (key_type, name) in [
        (KeyType::Ed25519, "ed25519"),
        (KeyType::EcdsaP256, "ecdsa-p256"),
        (KeyType::Rsa3072, "rsa-3072"),
    ] {
        assert_eq!(key_type.as_str(), name);
        assert_eq!(KeyType::parse(name), Some(key_type));
        let json: KeyType = serde_json::from_str(&format!("\"{name}\"")).expect("json name");
        assert_eq!(json, key_type);
    }
    assert_eq!(KeyType::parse("dsa"), None);
}
