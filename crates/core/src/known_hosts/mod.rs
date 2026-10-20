//! The app's own known hosts store (D21; docs/data-model.md, section 9.5;
//! UI 10.1, 10.2; docs/security.md, section 11.1): `known_host` records in the
//! personal vault, matching of plain entries, OpenSSH wildcard patterns and
//! hashed entries (`|1|<salt>|<hash>`, HMAC-SHA1 compared in constant time),
//! revoked keys, and the display rules of UI 10.1.
//!
//! The system `~/.ssh/known_hosts` is never read when connecting.
//!
//! Owner: S4. The facade methods (`Core::known_host_*`) are the contract;
//! internal APIs are S4's choice. The known_hosts file format (import and
//! export, stage 2b) is in [`openssh`].

pub mod openssh;

use data_encoding::{BASE64, BASE64_NOPAD};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::model::known_host::Entry;
use crate::service::host_api::HostKeyStatus;

/// Length of an HMAC-SHA1 hash in a hashed entry.
const HASH_LEN: usize = 20;

/// Longest salt accepted in a hashed entry (OpenSSH writes 20 bytes).
const MAX_SALT_LEN: usize = 64;

/// `SHA256:` fingerprint of a base64 key blob, as OpenSSH prints it
/// (unpadded base64 of SHA-256 of the blob). `None` for invalid base64.
pub fn fingerprint_sha256(public_key_base64: &str) -> Option<String> {
    decode_blob(public_key_base64).map(|blob| fingerprint_of_blob(&blob))
}

/// `SHA256:` fingerprint of a raw key blob.
pub fn fingerprint_of_blob(blob: &[u8]) -> String {
    let digest = Sha256::digest(blob);
    format!("SHA256:{}", BASE64_NOPAD.encode(&digest))
}

/// UI 10.1 display address: `host` for port 22, `[host]:port` otherwise.
pub fn display_address(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Decodes base64 (padded, as in known_hosts lines, or unpadded). `None` for
/// invalid or empty input.
pub fn decode_blob(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    let blob = BASE64
        .decode(text.as_bytes())
        .or_else(|_| BASE64_NOPAD.decode(text.as_bytes()))
        .ok()?;
    (!blob.is_empty()).then_some(blob)
}

/// Canonical padded base64 of a blob.
pub fn encode_blob(blob: &[u8]) -> String {
    BASE64.encode(blob)
}

/// The algorithm name of an SSH public key blob, when the blob decodes as a
/// public key (not a certificate) of that algorithm.
pub fn blob_key_type(blob: &[u8]) -> Option<String> {
    let key = ssh_key::PublicKey::from_bytes(blob).ok()?;
    if key.key_data().is_certificate() {
        return None;
    }
    Some(key.algorithm().as_str().to_owned())
}

/// What a stored `host` value is.
#[derive(Clone, PartialEq, Eq)]
pub enum HostKind {
    /// A host name or address.
    Plain,
    /// An OpenSSH pattern with `*` or `?`.
    Pattern,
    /// `|1|<base64 salt>|<base64 hash>`.
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
    /// Not a valid `host` value.
    Invalid,
}

impl std::fmt::Debug for HostKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Plain => "Plain",
            Self::Pattern => "Pattern",
            Self::Hashed { .. } => "Hashed",
            Self::Invalid => "Invalid",
        })
    }
}

/// Classifies a `host` value. Plain names and patterns may not contain
/// whitespace, control characters, commas or brackets, and may not start
/// with `!` (negated patterns are not stored) or `|` (only `|1|` hashes are).
pub fn classify(host: &str) -> HostKind {
    if let Some(rest) = host.strip_prefix("|1|") {
        let mut parts = rest.split('|');
        let (Some(salt), Some(hash), None) = (parts.next(), parts.next(), parts.next()) else {
            return HostKind::Invalid;
        };
        let (Some(salt), Some(hash)) = (decode_blob(salt), decode_blob(hash)) else {
            return HostKind::Invalid;
        };
        if hash.len() != HASH_LEN || salt.len() > MAX_SALT_LEN {
            return HostKind::Invalid;
        }
        return HostKind::Hashed { salt, hash };
    }
    if host.is_empty()
        || host.starts_with('|')
        || host.starts_with('!')
        || host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | '[' | ']'))
    {
        return HostKind::Invalid;
    }
    if host.contains('*') || host.contains('?') {
        HostKind::Pattern
    } else {
        HostKind::Plain
    }
}

/// OpenSSH wildcard matching: `*` matches any sequence (also empty), `?`
/// exactly one character; case-insensitive.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// The name OpenSSH hashes for an address and port: the lowercase address
/// for port 22, `[address]:port` otherwise.
pub fn hashed_name(address: &str, port: u16) -> String {
    display_address(&address.to_lowercase(), port)
}

/// HMAC-SHA1 of `name` keyed with `salt` (the OpenSSH known_hosts hashing).
pub fn hmac_sha1(salt: &[u8], name: &str) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha1> as KeyInit>::new_from_slice(salt).expect("HMAC accepts keys of any length");
    mac.update(name.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// A hashed `host` value for `address` and `port` with the given salt (the
/// format OpenSSH writes with `HashKnownHosts yes`).
pub fn hash_host(salt: &[u8], address: &str, port: u16) -> String {
    let hash = hmac_sha1(salt, &hashed_name(address, port));
    format!("|1|{}|{}", BASE64.encode(salt), BASE64.encode(&hash))
}

/// `true` when the entry's host and port match the address and port of a
/// connection (docs/data-model.md, section 9.5). The address is the one
/// connected to, without brackets.
pub fn entry_matches(host: &str, port: Option<u16>, address: &str, connect_port: u16) -> bool {
    let address = address.trim().to_lowercase();
    if address.is_empty() {
        return false;
    }
    match classify(host) {
        HostKind::Plain => port.unwrap_or(22) == connect_port && host.to_lowercase() == address,
        HostKind::Pattern => port.unwrap_or(22) == connect_port && wildcard_match(host, &address),
        HostKind::Hashed { salt, hash } => {
            let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(&salt)
                .expect("HMAC accepts keys of any length");
            mac.update(hashed_name(&address, connect_port).as_bytes());
            // Constant-time comparison (docs/data-model.md, section 9.5).
            mac.verify_slice(&hash).is_ok()
        }
        HostKind::Invalid => false,
    }
}

/// `true` when the entry holds this key (same type and blob).
fn has_key(entry: &Entry, key_type: &str, blob: &[u8]) -> bool {
    entry.key_type == key_type && decode_blob(&entry.public_key).as_deref() == Some(blob)
}

/// Looks up a presented key (stage 2 plan, section 4.4, decision 21).
/// `entries` are the live, readable records, ordered by record id.
///
/// 1. Any `revoked` entry with the presented key (whatever its host): `Revoked`.
/// 2. A non-revoked entry for this host with the key: `Match`.
/// 3. Non-revoked entries for this host, none with the key: `Changed`
///    (the first one with the same key type, else the first one).
/// 4. Otherwise `Unknown`.
pub fn check(
    entries: &[(Uuid, Entry)],
    address: &str,
    port: u16,
    key_type: &str,
    blob: &[u8],
) -> HostKeyStatus {
    if let Some((record_id, _)) = entries
        .iter()
        .find(|(_, entry)| entry.revoked && has_key(entry, key_type, blob))
    {
        return HostKeyStatus::Revoked {
            record_id: *record_id,
        };
    }
    let matching: Vec<&(Uuid, Entry)> = entries
        .iter()
        .filter(|(_, entry)| {
            !entry.revoked && entry_matches(&entry.host, entry.port, address, port)
        })
        .collect();
    if let Some((record_id, _)) = matching
        .iter()
        .find(|(_, entry)| has_key(entry, key_type, blob))
    {
        return HostKeyStatus::Match {
            record_id: *record_id,
        };
    }
    let old = matching
        .iter()
        .find(|(_, entry)| entry.key_type == key_type)
        .or_else(|| matching.first());
    match old {
        Some((record_id, entry)) => HostKeyStatus::Changed {
            record_id: *record_id,
            old_key_type: entry.key_type.clone(),
            old_fingerprint: fingerprint_sha256(&entry.public_key).unwrap_or_default(),
        },
        None => HostKeyStatus::Unknown,
    }
}

/// Key types of the non-revoked entries for (address, port), in record
/// order without duplicates: the preferred host key algorithms of a
/// connection (decision 21).
pub fn known_key_types(entries: &[(Uuid, Entry)], address: &str, port: u16) -> Vec<String> {
    let mut types: Vec<String> = Vec::new();
    for (_, entry) in entries {
        if !entry.revoked
            && entry_matches(&entry.host, entry.port, address, port)
            && !types.contains(&entry.key_type)
        {
            types.push(entry.key_type.clone());
        }
    }
    types
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::dto::KnownHostSource;

    fn blob(tag: u8) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[tag; 32]);
        blob
    }

    fn entry(host: &str, port: Option<u16>, tag: u8, revoked: bool) -> Entry {
        Entry {
            host: host.to_owned(),
            port,
            key_type: "ssh-ed25519".to_owned(),
            public_key: encode_blob(&blob(tag)),
            revoked,
            source: KnownHostSource::Import,
            comment: None,
        }
    }

    #[test]
    fn fingerprints_match_openssh() {
        // `ssh-keygen -lf` prints SHA256 of the blob as unpadded base64.
        let fingerprint = fingerprint_of_blob(b"");
        assert_eq!(
            fingerprint,
            "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU"
        );
        assert_eq!(fingerprint_sha256("!!"), None);
        assert_eq!(fingerprint_sha256(""), None);
        let padded = encode_blob(&blob(1));
        assert_eq!(
            fingerprint_sha256(&padded),
            fingerprint_sha256(padded.trim_end_matches('='))
        );
    }

    #[test]
    fn display_addresses() {
        assert_eq!(display_address("h.example.com", 22), "h.example.com");
        assert_eq!(
            display_address("h.example.com", 2222),
            "[h.example.com]:2222"
        );
        assert_eq!(display_address("2001:db8::1", 2222), "[2001:db8::1]:2222");
    }

    #[test]
    fn classification() {
        assert_eq!(classify("h.example.com"), HostKind::Plain);
        assert_eq!(classify("192.0.2.10"), HostKind::Plain);
        assert_eq!(classify("2001:db8::1"), HostKind::Plain);
        assert_eq!(classify("*.example.com"), HostKind::Pattern);
        assert_eq!(classify("web?.example.com"), HostKind::Pattern);
        assert!(matches!(
            classify(&hash_host(&[1u8; 20], "h.example.com", 22)),
            HostKind::Hashed { .. }
        ));
        for invalid in [
            "",
            "a b",
            "a,b",
            "[a]:22",
            "!a",
            "|2|x|y",
            "|1|x",
            "|1|AAAA|AAAA",
            "a\u{1}b",
        ] {
            assert_eq!(classify(invalid), HostKind::Invalid, "{invalid}");
        }
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*.example.com", "web1.example.com"));
        assert!(wildcard_match("*.EXAMPLE.com", "Web1.Example.COM"));
        assert!(!wildcard_match("*.example.com", "example.com"));
        assert!(wildcard_match("*example.com", "example.com"));
        assert!(wildcard_match("web?.example.com", "web1.example.com"));
        assert!(!wildcard_match("web?.example.com", "web12.example.com"));
        assert!(wildcard_match("192.0.2.*", "192.0.2.77"));
        assert!(!wildcard_match("192.0.2.*", "192.0.20.1"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("a*b*c", "aXXbYYc"));
        assert!(!wildcard_match("a*b*c", "aXXbYY"));
        assert!(wildcard_match("**.example.com", "x.example.com"));
    }

    #[test]
    fn hashed_entries_match_openssh_hashing() {
        let salt = [9u8; 20];
        let host = hash_host(&salt, "H.Example.com", 22);
        assert!(entry_matches(&host, None, "h.example.com", 22));
        assert!(entry_matches(&host, None, "H.EXAMPLE.COM", 22));
        assert!(!entry_matches(&host, None, "h.example.com", 2222));
        assert!(!entry_matches(&host, None, "g.example.com", 22));
        let on_port = hash_host(&salt, "198.51.100.4", 2222);
        assert!(entry_matches(&on_port, None, "198.51.100.4", 2222));
        assert!(!entry_matches(&on_port, None, "198.51.100.4", 22));
        // The hashed name of a non-standard port is `[host]:port`.
        assert_eq!(hashed_name("2001:DB8::1", 2222), "[2001:db8::1]:2222");
        assert_eq!(
            hmac_sha1(&salt, "[198.51.100.4]:2222"),
            match classify(&on_port) {
                HostKind::Hashed { hash, .. } => hash,
                other => panic!("{other:?}"),
            }
        );
    }

    #[test]
    fn plain_and_pattern_matching_uses_ports() {
        assert!(entry_matches("h.example.com", None, "H.example.com", 22));
        assert!(entry_matches(
            "h.example.com",
            Some(22),
            "h.example.com",
            22
        ));
        assert!(!entry_matches("h.example.com", None, "h.example.com", 2222));
        assert!(entry_matches(
            "h.example.com",
            Some(2222),
            "h.example.com",
            2222
        ));
        assert!(entry_matches("2001:db8::1", None, "2001:DB8::1", 22));
        assert!(entry_matches("*.example.com", None, "a.example.com", 22));
        assert!(!entry_matches("*.example.com", None, "a.example.com", 2200));
        assert!(entry_matches(
            "*.example.com",
            Some(2200),
            "a.example.com",
            2200
        ));
        assert!(!entry_matches("h.example.com", None, "", 22));
        assert!(!entry_matches("a b", None, "a b", 22));
    }

    #[test]
    fn check_rules() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        let entries = vec![
            (a, entry("h.example.com", None, 1, false)),
            (b, entry("*.example.com", None, 2, false)),
            (c, entry("*.example.org", None, 3, true)),
        ];
        // Match on the plain entry.
        assert_eq!(
            check(&entries, "h.example.com", 22, "ssh-ed25519", &blob(1)),
            HostKeyStatus::Match { record_id: a }
        );
        // The pattern entry also matches h.example.com, with its own key.
        assert_eq!(
            check(&entries, "h.example.com", 22, "ssh-ed25519", &blob(2)),
            HostKeyStatus::Match { record_id: b }
        );
        // Matching entries, none with the key: changed (first with the type).
        assert_eq!(
            check(&entries, "h.example.com", 22, "ssh-ed25519", &blob(9)),
            HostKeyStatus::Changed {
                record_id: a,
                old_key_type: "ssh-ed25519".into(),
                old_fingerprint: fingerprint_of_blob(&blob(1)),
            }
        );
        // Another port: unknown.
        assert_eq!(
            check(&entries, "h.example.com", 2222, "ssh-ed25519", &blob(1)),
            HostKeyStatus::Unknown
        );
        // A revoked key blocks on any host.
        assert_eq!(
            check(&entries, "h.example.com", 22, "ssh-ed25519", &blob(3)),
            HostKeyStatus::Revoked { record_id: c }
        );
        assert_eq!(
            check(
                &entries,
                "unrelated.example.net",
                2222,
                "ssh-ed25519",
                &blob(3)
            ),
            HostKeyStatus::Revoked { record_id: c }
        );
        // A revoked entry for a host does not make another key "changed".
        assert_eq!(
            check(&entries, "x.example.org", 22, "ssh-ed25519", &blob(4)),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn changed_prefers_an_entry_of_the_presented_type() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut rsa = entry("h.example.com", None, 1, false);
        rsa.key_type = "ssh-rsa".into();
        let entries = vec![(a, rsa), (b, entry("h.example.com", None, 2, false))];
        match check(&entries, "h.example.com", 22, "ssh-ed25519", &blob(5)) {
            HostKeyStatus::Changed {
                record_id,
                old_key_type,
                ..
            } => {
                assert_eq!(record_id, b);
                assert_eq!(old_key_type, "ssh-ed25519");
            }
            other => panic!("{other:?}"),
        }
        match check(
            &entries,
            "h.example.com",
            22,
            "ecdsa-sha2-nistp256",
            &blob(5),
        ) {
            HostKeyStatus::Changed {
                record_id,
                old_key_type,
                ..
            } => {
                assert_eq!(record_id, a);
                assert_eq!(old_key_type, "ssh-rsa");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            known_key_types(&entries, "h.example.com", 22),
            vec!["ssh-rsa".to_owned(), "ssh-ed25519".to_owned()]
        );
        assert!(known_key_types(&entries, "h.example.com", 2222).is_empty());
    }
}
