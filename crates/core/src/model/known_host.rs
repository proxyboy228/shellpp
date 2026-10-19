//! The `known_host` record (docs/data-model.md, section 9.5; D21; stage 2
//! plan, section 4.4): validation of an entry and its stored data.
//!
//! A record holds one host (a plain name or address, an OpenSSH pattern with
//! `*` or `?`, or a hashed `|1|<salt>|<hash>` entry), an optional port, one
//! key (type and base64 blob), the `revoked` marker, its source and an
//! optional comment. Matching and display rules are in [`crate::known_hosts`].

use serde_json::{Map, Value, json};

use crate::error::{FieldError, ValidationReason};
use crate::known_hosts::{self, HostKind};
use crate::model::fields::Errors;
use crate::model::text;
use crate::service::dto::KnownHostSource;

/// Longest `host`, in Unicode scalar values (`string(1024)`).
pub const MAX_HOST_CHARS: usize = 1024;

/// Longest `key_type` (`string(64)`).
pub const MAX_KEY_TYPE_CHARS: usize = 64;

/// Longest `public_key`, in characters (`string(16384)`).
pub const MAX_PUBLIC_KEY_CHARS: usize = 16_384;

/// Longest `comment` (`string(256)`).
pub const MAX_COMMENT_CHARS: usize = 256;

/// A validated known host entry, ready to be stored.
#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    /// Lowercase name, address or pattern; a hashed entry exactly as given.
    pub host: String,
    /// `None` = 22 (always `None` for hashed entries).
    pub port: Option<u16>,
    pub key_type: String,
    /// Canonical (padded) base64 of the key blob.
    pub public_key: String,
    pub revoked: bool,
    pub source: KnownHostSource,
    pub comment: Option<String>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("key_type", &self.key_type)
            .field("revoked", &self.revoked)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// The raw members of an entry before validation.
#[derive(Clone, Copy)]
pub struct EntryInput<'a> {
    pub host: &'a str,
    pub port: Option<u16>,
    pub key_type: &'a str,
    pub public_key: &'a str,
    pub revoked: bool,
    pub source: KnownHostSource,
    pub comment: Option<&'a str>,
}

impl std::fmt::Debug for EntryInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryInput")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

/// `true` for a valid `key_type`: `[a-z0-9@.-]+`, at most 64 characters.
pub fn key_type_ok(key_type: &str) -> bool {
    !key_type.is_empty()
        && key_type.len() <= MAX_KEY_TYPE_CHARS
        && key_type
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@.-".contains(&b))
}

/// The stored form of a host: hashed entries as given, others lowercased.
/// `Err` with the reason when the value is not a valid `host`.
pub fn normalize_host(raw: &str) -> Result<String, ValidationReason> {
    let host = text::string(raw, MAX_HOST_CHARS)?.ok_or(ValidationReason::Required)?;
    match known_hosts::classify(&host) {
        HostKind::Hashed { .. } => Ok(host),
        HostKind::Invalid => Err(ValidationReason::Invalid),
        HostKind::Plain | HostKind::Pattern => Ok(host.to_lowercase()),
    }
}

/// Validates an entry. Every refused member adds one error with its field
/// name (`host`, `port`, `key_type`, `public_key`, `comment`).
pub fn validate(input: EntryInput<'_>) -> Result<Entry, Vec<FieldError>> {
    let mut errors = Errors::new();
    let host = match normalize_host(input.host) {
        Ok(host) => Some(host),
        Err(reason) => {
            errors.push("host", reason);
            None
        }
    };
    let hashed = host
        .as_deref()
        .is_some_and(|host| matches!(known_hosts::classify(host), HostKind::Hashed { .. }));
    let port = match input.port {
        Some(0) => {
            errors.push("port", ValidationReason::OutOfRange);
            None
        }
        Some(_) if hashed => {
            errors.push("port", ValidationReason::NotAllowed);
            None
        }
        Some(22) | None => None,
        Some(port) => Some(port),
    };
    let key_type = input.key_type.trim();
    let key_type = if key_type.is_empty() {
        errors.push("key_type", ValidationReason::Required);
        None
    } else if key_type.chars().count() > MAX_KEY_TYPE_CHARS {
        errors.push("key_type", ValidationReason::TooLong);
        None
    } else if !key_type_ok(key_type) {
        errors.push("key_type", ValidationReason::Invalid);
        None
    } else {
        Some(key_type.to_owned())
    };
    let public_key = input.public_key.trim();
    let public_key = if public_key.is_empty() {
        errors.push("public_key", ValidationReason::Required);
        None
    } else if public_key.chars().count() > MAX_PUBLIC_KEY_CHARS {
        errors.push("public_key", ValidationReason::TooLong);
        None
    } else {
        match known_hosts::decode_blob(public_key) {
            Some(blob) => match (&key_type, known_hosts::blob_key_type(&blob)) {
                (Some(expected), Some(actual)) if *expected == actual => {
                    Some(known_hosts::encode_blob(&blob))
                }
                // The key type is refused already; report only the type.
                (None, Some(_)) => None,
                _ => {
                    errors.push("public_key", ValidationReason::Invalid);
                    None
                }
            },
            None => {
                errors.push("public_key", ValidationReason::Invalid);
                None
            }
        }
    };
    let comment = match input
        .comment
        .map(|raw| text::string(raw, MAX_COMMENT_CHARS))
    {
        None | Some(Ok(None)) => None,
        Some(Ok(Some(comment))) => Some(comment),
        Some(Err(reason)) => {
            errors.push("comment", reason);
            None
        }
    };
    match (host, key_type, public_key) {
        (Some(host), Some(key_type), Some(public_key)) if errors.is_empty() => Ok(Entry {
            host,
            port,
            key_type,
            public_key,
            revoked: input.revoked,
            source: input.source,
            comment,
        }),
        _ => Err(errors.into_vec()),
    }
}

/// The source as stored.
pub fn source_name(source: KnownHostSource) -> &'static str {
    match source {
        KnownHostSource::Tofu => "tofu",
        KnownHostSource::Replaced => "replaced",
        KnownHostSource::Import => "import",
    }
}

fn parse_source(name: &str) -> Option<KnownHostSource> {
    match name {
        "tofu" => Some(KnownHostSource::Tofu),
        "replaced" => Some(KnownHostSource::Replaced),
        "import" => Some(KnownHostSource::Import),
        _ => None,
    }
}

/// The record data of a new entry.
pub fn to_data(entry: &Entry) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("host".into(), Value::String(entry.host.clone()));
    if let Some(port) = entry.port {
        data.insert("port".into(), json!(port));
    }
    data.insert("key_type".into(), Value::String(entry.key_type.clone()));
    data.insert("public_key".into(), Value::String(entry.public_key.clone()));
    if entry.revoked {
        data.insert("marker".into(), json!("revoked"));
    }
    data.insert("source".into(), json!(source_name(entry.source)));
    if let Some(comment) = &entry.comment {
        data.insert("comment".into(), Value::String(comment.clone()));
    }
    data
}

/// Reads stored data (written by this or another client) leniently. `None`
/// when a required member is missing or malformed: such a record is kept
/// but never matched or listed.
pub fn read(data: &Map<String, Value>) -> Option<Entry> {
    let host = data
        .get("host")
        .and_then(Value::as_str)
        .filter(|host| !host.is_empty())?;
    let hashed = matches!(known_hosts::classify(host), HostKind::Hashed { .. });
    if matches!(known_hosts::classify(host), HostKind::Invalid) {
        return None;
    }
    let port = match data.get("port") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64().and_then(|port| u16::try_from(port).ok()) {
            Some(0) => return None,
            Some(22) => None,
            Some(port) => Some(port),
            None => return None,
        },
    };
    let key_type = data
        .get("key_type")
        .and_then(Value::as_str)
        .filter(|key_type| key_type_ok(key_type))?;
    let public_key = data.get("public_key").and_then(Value::as_str)?;
    known_hosts::decode_blob(public_key)?;
    let revoked = data.get("marker").and_then(Value::as_str) == Some("revoked");
    let source = data
        .get("source")
        .and_then(Value::as_str)
        .and_then(parse_source)
        .unwrap_or(KnownHostSource::Import);
    let comment = data
        .get("comment")
        .and_then(Value::as_str)
        .filter(|comment| !comment.is_empty())
        .map(str::to_owned);
    Some(Entry {
        host: if hashed {
            host.to_owned()
        } else {
            host.to_lowercase()
        },
        port: if hashed { None } else { port },
        key_type: key_type.to_owned(),
        public_key: public_key.to_owned(),
        revoked,
        source,
        comment,
    })
}

/// The natural key of docs/data-model.md, section 18.6: (`host`, `port`,
/// `key_type`, `public_key`, `marker`), with the blob compared decoded.
pub fn same_natural_key(a: &Entry, b: &Entry) -> bool {
    a.host == b.host
        && a.port.unwrap_or(22) == b.port.unwrap_or(22)
        && a.key_type == b.key_type
        && a.revoked == b.revoked
        && known_hosts::decode_blob(&a.public_key) == known_hosts::decode_blob(&b.public_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    /// A syntactically valid Ed25519 public key blob (32 zero bytes as the key).
    fn ed25519_blob() -> String {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[7u8; 32]);
        known_hosts::encode_blob(&blob)
    }

    fn input<'a>(host: &'a str, key_type: &'a str, public_key: &'a str) -> EntryInput<'a> {
        EntryInput {
            host,
            port: None,
            key_type,
            public_key,
            revoked: false,
            source: KnownHostSource::Import,
            comment: None,
        }
    }

    fn errors(result: Result<Entry, Vec<FieldError>>) -> Vec<(String, ValidationReason)> {
        result
            .unwrap_err()
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    #[test]
    fn plain_entries_are_lowercased_and_port_22_is_absent() {
        let blob = ed25519_blob();
        let entry = validate(EntryInput {
            port: Some(22),
            ..input(" Web1.Example.COM ", "ssh-ed25519", &blob)
        })
        .unwrap();
        assert_eq!(entry.host, "web1.example.com");
        assert_eq!(entry.port, None);
        let data = to_data(&entry);
        assert_eq!(data.get("port"), None);
        assert_eq!(data["source"], json!("import"));
        assert_eq!(read(&data), Some(entry.clone()));

        let other = validate(EntryInput {
            port: Some(2222),
            revoked: true,
            comment: Some(" note "),
            ..input("2001:DB8::1", "ssh-ed25519", &blob)
        })
        .unwrap();
        assert_eq!(other.host, "2001:db8::1");
        assert_eq!(other.port, Some(2222));
        let data = to_data(&other);
        assert_eq!(data["marker"], json!("revoked"));
        assert_eq!(data["comment"], json!("note"));
        assert_eq!(read(&data), Some(other));
    }

    #[test]
    fn patterns_and_hashed_entries() {
        let blob = ed25519_blob();
        let pattern = validate(input("*.Example.com", "ssh-ed25519", &blob)).unwrap();
        assert_eq!(pattern.host, "*.example.com");
        let hashed_host = "|1|AAECAwQFBgcICQoLDA0ODxAREhM=|FBMSERAPDg0MCwoJCAcGBQQDAgE=";
        let hashed = validate(input(hashed_host, "ssh-ed25519", &blob)).unwrap();
        assert_eq!(hashed.host, hashed_host);
        assert_eq!(
            errors(validate(EntryInput {
                port: Some(2222),
                ..input(hashed_host, "ssh-ed25519", &blob)
            })),
            vec![("port".into(), NotAllowed)]
        );
    }

    #[test]
    fn validation_table() {
        let blob = ed25519_blob();
        for (host, reason) in [
            ("", Required),
            ("   ", Required),
            ("a b.example.com", Invalid),
            ("a,b.example.com", Invalid),
            ("[a.example.com]:22", Invalid),
            ("!a.example.com", Invalid),
            ("|1|notbase64!|x", Invalid),
            ("|1|AAAA|AAAA", Invalid),
            (
                "|2|AAECAwQFBgcICQoLDA0ODxAREhM=|FBMSERAPDg0MCwoJCAcGBQQDAgE=",
                Invalid,
            ),
        ] {
            assert_eq!(
                errors(validate(input(host, "ssh-ed25519", &blob))),
                vec![("host".to_owned(), reason)],
                "{host}"
            );
        }
        assert_eq!(
            errors(validate(input(&"h".repeat(1025), "ssh-ed25519", &blob))),
            vec![("host".to_owned(), TooLong)]
        );
        assert_eq!(
            errors(validate(EntryInput {
                port: Some(0),
                ..input("h.example.com", "ssh-ed25519", &blob)
            })),
            vec![("port".to_owned(), OutOfRange)]
        );
        assert_eq!(
            errors(validate(input("h.example.com", "SSH-ED25519", &blob))),
            vec![("key_type".to_owned(), Invalid)]
        );
        assert_eq!(
            errors(validate(input("h.example.com", "", &blob))),
            vec![("key_type".to_owned(), Required)]
        );
        assert_eq!(
            errors(validate(input("h.example.com", "ssh-rsa", &blob))),
            vec![("public_key".to_owned(), Invalid)]
        );
        assert_eq!(
            errors(validate(input(
                "h.example.com",
                "ssh-ed25519",
                "not base64!"
            ))),
            vec![("public_key".to_owned(), Invalid)]
        );
        assert_eq!(
            errors(validate(input("h.example.com", "ssh-ed25519", "AAAA"))),
            vec![("public_key".to_owned(), Invalid)]
        );
        assert_eq!(
            errors(validate(EntryInput {
                comment: Some("a\u{7}b"),
                ..input("h.example.com", "ssh-ed25519", &blob)
            })),
            vec![("comment".to_owned(), Invalid)]
        );
    }

    #[test]
    fn natural_key_compares_decoded_blobs_and_effective_ports() {
        let blob = ed25519_blob();
        let a = validate(input("h.example.com", "ssh-ed25519", &blob)).unwrap();
        let mut b = a.clone();
        b.public_key = blob.trim_end_matches('=').to_owned();
        b.port = Some(22);
        b.source = KnownHostSource::Tofu;
        assert!(same_natural_key(&a, &b));
        b.revoked = true;
        assert!(!same_natural_key(&a, &b));
    }

    #[test]
    fn malformed_stored_data_is_not_read() {
        let blob = ed25519_blob();
        for data in [
            json!({"key_type": "ssh-ed25519", "public_key": blob}),
            json!({"host": "h", "key_type": "ssh-ed25519"}),
            json!({"host": "h", "key_type": "ssh-ed25519", "public_key": "!"}),
            json!({"host": "h", "port": 0, "key_type": "ssh-ed25519", "public_key": blob}),
            json!({"host": "h", "port": "22", "key_type": "ssh-ed25519", "public_key": blob}),
            json!({"host": "h", "key_type": "Bad Type", "public_key": blob}),
        ] {
            assert!(read(data.as_object().unwrap()).is_none(), "{data}");
        }
    }
}
