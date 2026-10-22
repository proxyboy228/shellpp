//! Algorithm preferences (stage 2 plan, decisions 21 and 22; docs/security.md,
//! section 11.2): no SHA-1 key exchange, no `ssh-rsa` (SHA-1) signatures, no
//! SHA-1 MACs, no host certificates; the host key algorithms already saved
//! in known hosts for the target come first, so that a server with several
//! host keys presents one the store knows.
//!
//! The host setting "Allow legacy algorithms" (off by default; run 4,
//! plan 9.1, D80) adds the SHA-1 and CBC ones after all modern ones, for old
//! servers and appliances that have nothing else.

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

/// The algorithm names a mismatch error can show (what a server offered,
/// [`crate::AlgorithmMismatch`]): ours, the legacy ones and those old or
/// other servers name. At most 128 (the bits of the mismatch).
pub(crate) const KNOWN: &[&str] = &[
    // Key exchange.
    "mlkem768x25519-sha256",
    "sntrup761x25519-sha512",
    "sntrup761x25519-sha512@openssh.com",
    "curve25519-sha256",
    "curve25519-sha256@libssh.org",
    "ecdh-sha2-nistp256",
    "ecdh-sha2-nistp384",
    "ecdh-sha2-nistp521",
    "diffie-hellman-group-exchange-sha256",
    "diffie-hellman-group18-sha512",
    "diffie-hellman-group17-sha512",
    "diffie-hellman-group16-sha512",
    "diffie-hellman-group15-sha512",
    "diffie-hellman-group14-sha256",
    "diffie-hellman-group14-sha1",
    "diffie-hellman-group-exchange-sha1",
    "diffie-hellman-group1-sha1",
    // Host keys.
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "rsa-sha2-512",
    "rsa-sha2-256",
    "ssh-rsa",
    "ssh-dss",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
    "ssh-ed25519-cert-v01@openssh.com",
    "ecdsa-sha2-nistp256-cert-v01@openssh.com",
    "rsa-sha2-512-cert-v01@openssh.com",
    "rsa-sha2-256-cert-v01@openssh.com",
    "ssh-rsa-cert-v01@openssh.com",
    // Ciphers.
    "chacha20-poly1305@openssh.com",
    "aes256-gcm@openssh.com",
    "aes128-gcm@openssh.com",
    "aes256-ctr",
    "aes192-ctr",
    "aes128-ctr",
    "aes256-cbc",
    "aes192-cbc",
    "aes128-cbc",
    "3des-cbc",
    "blowfish-cbc",
    "cast128-cbc",
    "arcfour",
    "arcfour128",
    "arcfour256",
    // MACs.
    "hmac-sha2-512-etm@openssh.com",
    "hmac-sha2-256-etm@openssh.com",
    "hmac-sha2-512",
    "hmac-sha2-256",
    "hmac-sha1-etm@openssh.com",
    "hmac-sha1",
    "hmac-sha1-96",
    "hmac-md5",
    "hmac-md5-96",
    "hmac-md5-etm@openssh.com",
    "umac-64@openssh.com",
    "umac-128@openssh.com",
    "umac-64-etm@openssh.com",
    "umac-128-etm@openssh.com",
    // Compression.
    "none",
    "zlib",
    "zlib@openssh.com",
];

/// Legacy algorithms, offered after the modern ones only when the host
/// allows them (D80).
const LEGACY_KEX: &[kex::Name] = &[kex::DH_G14_SHA1, kex::DH_GEX_SHA1];
const LEGACY_CIPHER: &[cipher::Name] = &[
    cipher::AES_256_CBC,
    cipher::AES_192_CBC,
    cipher::AES_128_CBC,
];
const LEGACY_MAC: &[mac::Name] = &[mac::HMAC_SHA1_ETM, mac::HMAC_SHA1];
/// `ssh-rsa`: RSA signatures with SHA-1.
const LEGACY_HOST_KEY: Algorithm = Algorithm::Rsa { hash: None };

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
pub(crate) fn preferred(preferred_host_key_types: &[String], legacy: bool) -> Preferred {
    let mut key = host_key_order(preferred_host_key_types);
    if !legacy {
        return Preferred {
            kex: Cow::Borrowed(KEX_ORDER),
            key: Cow::Owned(key),
            // Host certificates are not trusted in the MVP (docs/security.md, 11.1).
            host_key_certificates: Cow::Borrowed(&[]),
            cipher: Cow::Borrowed(CIPHER_ORDER),
            mac: Cow::Borrowed(MAC_ORDER),
            compression: Cow::Borrowed(COMPRESSION_ORDER),
        };
    }
    // The legacy key exchanges go before the pseudo-algorithms at the end
    // (ext-info-c, strict kex), which are never chosen.
    let pseudo = KEX_ORDER
        .iter()
        .position(|name| *name == kex::EXTENSION_SUPPORT_AS_CLIENT)
        .unwrap_or(KEX_ORDER.len());
    let mut kex_order: Vec<kex::Name> = KEX_ORDER[..pseudo].to_vec();
    kex_order.extend_from_slice(LEGACY_KEX);
    kex_order.extend_from_slice(&KEX_ORDER[pseudo..]);
    key.push(LEGACY_HOST_KEY);
    Preferred {
        kex: Cow::Owned(kex_order),
        key: Cow::Owned(key),
        host_key_certificates: Cow::Borrowed(&[]),
        cipher: Cow::Owned([CIPHER_ORDER, LEGACY_CIPHER].concat()),
        mac: Cow::Owned([MAC_ORDER, LEGACY_MAC].concat()),
        compression: Cow::Borrowed(COMPRESSION_ORDER),
    }
}

/// The names Shell++ offers for one negotiation, most preferred first
/// (for the error of a mismatch: what we support). The pseudo-algorithms of
/// the key exchange list are left out.
pub fn supported(kind: crate::AlgorithmKind, legacy: bool) -> Vec<&'static str> {
    use crate::AlgorithmKind as K;
    fn names<N: AsRef<str> + 'static>(
        modern: &'static [N],
        old: &'static [N],
        legacy: bool,
    ) -> Vec<&'static str> {
        let old: &'static [N] = if legacy { old } else { &[] };
        modern.iter().chain(old).map(AsRef::as_ref).collect()
    }
    match kind {
        K::KeyExchange => names(KEX_ORDER, LEGACY_KEX, legacy)
            .into_iter()
            .filter(|name| !is_pseudo_kex(name))
            .collect(),
        K::HostKey => {
            let mut keys: Vec<&'static str> = default_host_key_algorithms()
                .iter()
                .map(host_key_name)
                .collect();
            if legacy {
                keys.push(host_key_name(&LEGACY_HOST_KEY));
            }
            keys
        }
        K::Cipher => names(CIPHER_ORDER, LEGACY_CIPHER, legacy),
        K::Mac => names(MAC_ORDER, LEGACY_MAC, legacy),
        K::Compression => names(COMPRESSION_ORDER, &[], false),
    }
}

/// Whether a name is legacy: offered only when the host allows it.
pub(crate) fn is_legacy(kind: crate::AlgorithmKind, name: &str) -> bool {
    use crate::AlgorithmKind as K;
    match kind {
        K::KeyExchange => LEGACY_KEX.iter().any(|legacy| legacy.as_ref() == name),
        K::HostKey => name == "ssh-rsa",
        K::Cipher => LEGACY_CIPHER.iter().any(|legacy| legacy.as_ref() == name),
        K::Mac => LEGACY_MAC.iter().any(|legacy| legacy.as_ref() == name),
        K::Compression => false,
    }
}

fn is_pseudo_kex(name: &str) -> bool {
    name.starts_with("ext-info-") || name.starts_with("kex-strict-")
}

/// The static wire name of a host key algorithm we offer.
fn host_key_name(algorithm: &Algorithm) -> &'static str {
    match algorithm {
        Algorithm::Ed25519 => "ssh-ed25519",
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP256,
        } => "ecdsa-sha2-nistp256",
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP384,
        } => "ecdsa-sha2-nistp384",
        Algorithm::Ecdsa {
            curve: EcdsaCurve::NistP521,
        } => "ecdsa-sha2-nistp521",
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha512),
        } => "rsa-sha2-512",
        Algorithm::Rsa {
            hash: Some(HashAlg::Sha256),
        } => "rsa-sha2-256",
        Algorithm::Rsa { .. } => "ssh-rsa",
        _ => "other",
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
        let preferred = preferred(&["ssh-rsa".to_owned()], false);
        for name in names(&preferred) {
            assert!(!name.contains("sha1"), "{name} must not be offered");
            assert_ne!(name, "ssh-rsa", "SHA-1 RSA signatures must not be offered");
            assert!(!name.contains("cert-v01"), "{name}: no host certificates");
        }
        assert!(preferred.host_key_certificates.is_empty());
    }

    #[test]
    fn legacy_algorithms_come_last_and_only_when_allowed() {
        use crate::AlgorithmKind as K;
        assert!(KNOWN.len() <= 128);
        for kind in [
            K::KeyExchange,
            K::HostKey,
            K::Cipher,
            K::Mac,
            K::Compression,
        ] {
            let modern = supported(kind, false);
            let all = supported(kind, true);
            assert!(modern.iter().all(|name| !is_legacy(kind, name)), "{kind:?}");
            assert_eq!(&all[..modern.len()], &modern[..], "{kind:?}: legacy after");
            // Every name we can offer is in the table a mismatch shows.
            for name in &all {
                assert!(KNOWN.contains(name), "{name} is not in KNOWN");
            }
        }
        assert!(supported(K::KeyExchange, true).contains(&"diffie-hellman-group14-sha1"));
        assert!(supported(K::HostKey, true).ends_with(&["ssh-rsa"]));
        assert!(supported(K::Mac, true).contains(&"hmac-sha1"));
        let legacy = preferred(&[], true);
        let kex: Vec<&str> = legacy.kex.iter().map(|name| name.as_ref()).collect();
        assert!(
            kex.ends_with(&["ext-info-c", "kex-strict-c-v00@openssh.com"]),
            "{kex:?}"
        );
        assert_eq!(legacy.key.last(), Some(&Algorithm::Rsa { hash: None }));
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
        let preferred = preferred(&[], false);
        assert_eq!(preferred.compression[0].as_ref(), "none");
    }
}
