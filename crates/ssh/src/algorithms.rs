//! Algorithm preferences (stage 2 plan, decisions 21 and 22; docs/security.md,
//! section 11.2): no SHA-1 key exchange, no `ssh-rsa` (SHA-1) signatures, no
//! SHA-1 MACs, no host certificates; the host key algorithms already saved
//! in known hosts for the target come first, so that a server with several
//! host keys presents one the store knows.

use std::borrow::Cow;

use russh::keys::ssh_key::{Algorithm, EcdsaCurve, HashAlg};
use russh::{Preferred, cipher, compression, kex, mac};

/// Key exchange algorithms, most preferred first. russh's defaults (which
/// already leave out every SHA-1 exchange) plus the NIST ECDH exchanges for
/// servers without curve25519.
const KEX_ORDER: &[kex::Name] = &[
    kex::MLKEM768X25519_SHA256,
    kex::CURVE25519,
    kex::CURVE25519_PRE_RFC_8731,
    kex::ECDH_SHA2_NISTP256,
    kex::ECDH_SHA2_NISTP384,
    kex::ECDH_SHA2_NISTP521,
    kex::DH_GEX_SHA256,
    kex::DH_G18_SHA512,
    kex::DH_G17_SHA512,
    kex::DH_G16_SHA512,
    kex::DH_G15_SHA512,
    kex::DH_G14_SHA256,
    kex::EXTENSION_SUPPORT_AS_CLIENT,
    kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
];

const CIPHER_ORDER: &[cipher::Name] = &[
    cipher::CHACHA20_POLY1305,
    cipher::AES_256_GCM,
    cipher::AES_128_GCM,
    cipher::AES_256_CTR,
    cipher::AES_192_CTR,
    cipher::AES_128_CTR,
];

const MAC_ORDER: &[mac::Name] = &[
    mac::HMAC_SHA512_ETM,
    mac::HMAC_SHA256_ETM,
    mac::HMAC_SHA512,
    mac::HMAC_SHA256,
];

const COMPRESSION_ORDER: &[compression::Name] = &[
    compression::NONE,
    compression::ZLIB,
    compression::ZLIB_LEGACY,
];

/// Host key (and public key) algorithms without `ssh-rsa` (SHA-1).
fn default_host_key_algorithms() -> Vec<Algorithm> {
    vec![
        Algorithm::Ed25519,
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        },
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        },
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        },
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha512),
        },
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha256),
        },
    ]
}

/// The algorithms a saved key type stands for. A known_hosts entry names the
/// key blob type (`ssh-rsa` for every RSA key), which is used with the SHA-2
/// signature algorithms only.
fn algorithms_for_key_type(key_type: &str) -> Vec<Algorithm> {
    match key_type.trim() {
        "ssh-rsa" | "rsa-sha2-512" | "rsa-sha2-256" => vec![
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
        ],
        other => match Algorithm::new(other) {
            Ok(algorithm) if is_allowed_host_key_algorithm(&algorithm) => vec![algorithm],
            _ => Vec::new(),
        },
    }
}

fn is_allowed_host_key_algorithm(algorithm: &Algorithm) -> bool {
    default_host_key_algorithms().contains(algorithm)
}

/// Host key algorithms: the saved types first (in their order), then the rest
/// of the default order.
pub(crate) fn host_key_order(preferred_host_key_types: &[String]) -> Vec<Algorithm> {
    let mut order: Vec<Algorithm> = Vec::new();
    for key_type in preferred_host_key_types {
        for algorithm in algorithms_for_key_type(key_type) {
            if !order.contains(&algorithm) {
                order.push(algorithm);
            }
        }
    }
    for algorithm in default_host_key_algorithms() {
        if !order.contains(&algorithm) {
            order.push(algorithm);
        }
    }
    order
}

/// The russh preferences of one connection.
pub(crate) fn preferred(preferred_host_key_types: &[String]) -> Preferred {
    Preferred {
        kex: Cow::Borrowed(KEX_ORDER),
        key: Cow::Owned(host_key_order(preferred_host_key_types)),
        // Host certificates are not trusted in the MVP (docs/security.md, 11.1).
        host_key_certificates: Cow::Borrowed(&[]),
        cipher: Cow::Borrowed(CIPHER_ORDER),
        mac: Cow::Borrowed(MAC_ORDER),
        compression: Cow::Borrowed(COMPRESSION_ORDER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(preferred: &Preferred) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        all.extend(preferred.kex.iter().map(|n| n.as_ref().to_owned()));
        all.extend(preferred.key.iter().map(|a| a.as_str().to_owned()));
        all.extend(
            preferred
                .host_key_certificates
                .iter()
                .map(|a| a.as_str().to_owned()),
        );
        all.extend(preferred.cipher.iter().map(|n| n.as_ref().to_owned()));
        all.extend(preferred.mac.iter().map(|n| n.as_ref().to_owned()));
        all
    }

    #[test]
    fn no_sha1_algorithm_is_offered() {
        let preferred = preferred(&["ssh-rsa".to_owned()]);
        for name in names(&preferred) {
            assert!(!name.contains("sha1"), "{name} must not be offered");
            assert_ne!(name, "ssh-rsa", "SHA-1 RSA signatures must not be offered");
            assert!(!name.contains("cert-v01"), "{name}: no host certificates");
        }
        assert!(preferred.host_key_certificates.is_empty());
    }

    #[test]
    fn saved_host_key_types_come_first() {
        let order = host_key_order(&["ecdsa-sha2-nistp256".to_owned()]);
        assert_eq!(order[0].as_str(), "ecdsa-sha2-nistp256");
        assert_eq!(order[1].as_str(), "ssh-ed25519");

        let order = host_key_order(&["ssh-rsa".to_owned(), "ssh-ed25519".to_owned()]);
        let names: Vec<&str> = order.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            &names[..3],
            &["rsa-sha2-512", "rsa-sha2-256", "ssh-ed25519"]
        );
        // No duplicates, nothing lost.
        assert_eq!(order.len(), default_host_key_algorithms().len());
    }

    #[test]
    fn unknown_or_forbidden_saved_types_are_ignored() {
        let order = host_key_order(&["ssh-dss".to_owned(), "nonsense".to_owned()]);
        assert_eq!(order, default_host_key_algorithms());
    }

    #[test]
    fn compression_prefers_none() {
        let preferred = preferred(&[]);
        assert_eq!(preferred.compression[0].as_ref(), "none");
    }
}
