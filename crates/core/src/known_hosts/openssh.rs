//! The OpenSSH known_hosts file format (`sshd(8)`, section "SSH_KNOWN_HOSTS
//! FILE FORMAT"): the reader behind the Known Hosts import and the writer
//! behind its export (UI 10.1, D21; docs/data-model.md, sections 9.5 and
//! 18.4; docs/storage-format.md, section 13.6).
//!
//! Reading, line by line:
//!
//! - blank lines and `#` comments are ignored and not counted;
//! - `@cert-authority` lines are skipped and counted, as are negated (`!`)
//!   host patterns (data-model 9.5);
//! - `@revoked` lines become revoked entries;
//! - the host field is split at commas into one entry per pattern: a plain
//!   name or address, an OpenSSH pattern with `*` or `?`, `[host]:port`, or a
//!   hashed `|1|<salt>|<hash>` kept exactly as it is; a pattern repeated on
//!   the line is a duplicate;
//! - the key type must name the key blob's own algorithm; the rest of the
//!   line is the entry's comment (cut to 256 characters, dropped when it
//!   holds control characters);
//! - anything else (a missing field, an unknown marker, a malformed pattern
//!   or port, a key that does not decode, an SSH1 `bits exponent modulus`
//!   key, a line over [`MAX_LINE_BYTES`] or not UTF-8) makes the whole line
//!   invalid: it is counted and nothing of it is imported.
//!
//! Input limits: at most [`MAX_IMPORT_BYTES`] per file (the caller refuses a
//! larger one before reading it all), [`MAX_LINE_BYTES`] and
//! [`MAX_PATTERNS_PER_LINE`] host patterns per line. Every pattern of a line
//! copies its key, so a file gives at most [`MAX_IMPORT_ENTRIES`] entries
//! holding at most [`MAX_IMPORT_ENTRY_BYTES`] of text; a line past either
//! limit is invalid. The key and the comment of a line are checked once,
//! not once per pattern.
//!
//! Writing: one line per entry, sorted by host field, key type and key;
//! `[host]:port` for a port other than 22; hashed entries exactly as stored;
//! revoked entries prefixed with `@revoked`; LF line endings, no comments.
//!
//! Nothing here logs host names, addresses or keys.

use std::collections::HashSet;

use crate::model::known_host::{self, Entry, EntryInput, MAX_COMMENT_CHARS};
use crate::model::text;
use crate::service::dto::KnownHostSource;

use super::{HostKind, classify, decode_blob};

/// Largest known_hosts file the import reads (decision of stage 2b): a
/// real file with a hundred thousand hosts is about 10 MiB.
pub const MAX_IMPORT_BYTES: usize = 16 * 1024 * 1024;

/// Longest line the import reads; a longer line is invalid. The longest
/// valid entry is a 1024-character host with a 16384-character key.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Most host patterns on one line (negated ones included); a line with
/// more is invalid (decision of stage 2b). OpenSSH writes one or two.
pub const MAX_PATTERNS_PER_LINE: usize = 256;

/// Most entries one file gives (decision of stage 2b): twice the hundred
/// thousand hosts of a [`MAX_IMPORT_BYTES`] file. Lines past it are invalid.
pub const MAX_IMPORT_ENTRIES: usize = 200_000;

/// Most text the entries of one file hold: host, key type, key and comment
/// of each entry (decision of stage 2b). Every pattern of a line copies the
/// line's key, so this bounds the memory a file of patterns can take. Lines
/// past it are invalid.
pub const MAX_IMPORT_ENTRY_BYTES: usize = 4 * MAX_IMPORT_BYTES;

/// New entries the import writes per transaction; the core mutex is let go
/// between batches (decision of stage 2b).
pub const IMPORT_BATCH: usize = 2_000;

/// What a known_hosts file holds.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    /// Valid entries in file order (one per host pattern), `source = import`.
    pub entries: Vec<Entry>,
    /// `@cert-authority` lines and negated host patterns.
    pub skipped: usize,
    /// Lines that could not be read, and lines past the file's limits.
    pub invalid: usize,
    /// Host patterns repeated on their line (the entry is taken once).
    pub duplicates: usize,
}

impl std::fmt::Debug for ParsedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedFile")
            .field("entries", &self.entries.len())
            .field("skipped", &self.skipped)
            .field("invalid", &self.invalid)
            .field("duplicates", &self.duplicates)
            .finish()
    }
}

/// What one line holds.
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// Blank or a `#` comment.
    Ignored,
    /// Entries, the number of negated patterns next to them and the number
    /// of patterns repeated on the line.
    Entries(Vec<Entry>, usize, usize),
    /// An `@cert-authority` line.
    CertAuthority,
    Invalid,
}

/// The text an entry holds, counted against [`MAX_IMPORT_ENTRY_BYTES`].
fn entry_bytes(entry: &Entry) -> usize {
    entry.host.len()
        + entry.key_type.len()
        + entry.public_key.len()
        + entry.comment.as_ref().map_or(0, String::len)
}

/// Reads a known_hosts file. Never fails: what cannot be read is counted.
pub fn parse(input: &[u8]) -> ParsedFile {
    let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
    let mut file = ParsedFile::default();
    let mut bytes = 0;
    for raw in input.split(|byte| *byte == b'\n') {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let line = if raw.len() > MAX_LINE_BYTES {
            Line::Invalid
        } else {
            match std::str::from_utf8(raw) {
                Ok(line) => parse_line(line),
                Err(_) => Line::Invalid,
            }
        };
        match line {
            Line::Ignored => {}
            Line::Entries(entries, negated, repeated) => {
                let size: usize = entries.iter().map(entry_bytes).sum();
                if file.entries.len() + entries.len() > MAX_IMPORT_ENTRIES
                    || bytes + size > MAX_IMPORT_ENTRY_BYTES
                {
                    file.invalid += 1;
                    continue;
                }
                bytes += size;
                file.entries.extend(entries);
                file.skipped += negated;
                file.duplicates += repeated;
            }
            Line::CertAuthority => file.skipped += 1,
            Line::Invalid => file.invalid += 1,
        }
    }
    file
}

/// Splits off the first whitespace-separated field.
fn next_field(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches([' ', '\t']);
    if text.is_empty() {
        return None;
    }
    let end = text.find([' ', '\t']).unwrap_or(text.len());
    Some((&text[..end], &text[end..]))
}

fn parse_line(line: &str) -> Line {
    let trimmed = line.trim_matches([' ', '\t']);
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Line::Ignored;
    }
    let Some((first, rest)) = next_field(trimmed) else {
        return Line::Ignored;
    };
    let (revoked, hosts, rest) = match first {
        "@revoked" => match next_field(rest) {
            Some((hosts, rest)) => (true, hosts, rest),
            None => return Line::Invalid,
        },
        // Skipped whatever follows (D21); certificates are a later stage.
        "@cert-authority" => return Line::CertAuthority,
        marker if marker.starts_with('@') => return Line::Invalid,
        hosts => (false, hosts, rest),
    };
    let Some((key_type, rest)) = next_field(rest) else {
        return Line::Invalid;
    };
    let Some((public_key, rest)) = next_field(rest) else {
        return Line::Invalid;
    };
    // The key must decode and be of the named type (an SSH1 key's
    // "bits exponent modulus" fails here too).
    let Some(blob) = decode_blob(public_key) else {
        return Line::Invalid;
    };
    if super::blob_key_type(&blob).as_deref() != Some(key_type) {
        return Line::Invalid;
    }
    if hosts.split(',').count() > MAX_PATTERNS_PER_LINE {
        return Line::Invalid;
    }
    let comment = comment_of(rest);
    let mut entries: Vec<Entry> = Vec::new();
    let mut seen: HashSet<(String, Option<u16>)> = HashSet::new();
    let mut negated = 0;
    let mut repeated = 0;
    for pattern in hosts.split(',') {
        if pattern.starts_with('!') {
            negated += 1;
            continue;
        }
        let Some((host, port)) = split_pattern(pattern) else {
            return Line::Invalid;
        };
        // The key and the comment are checked with the first pattern; the
        // others take them over and check only their host and port.
        let entry = match entries.first() {
            Some(template) => known_host::with_host(template, host, port),
            None => known_host::validate(EntryInput {
                host,
                port,
                key_type,
                public_key,
                revoked,
                source: KnownHostSource::Import,
                comment: comment.as_deref(),
            })
            .ok(),
        };
        let Some(entry) = entry else {
            return Line::Invalid;
        };
        if seen.insert((entry.host.clone(), entry.port)) {
            entries.push(entry);
        } else {
            repeated += 1;
        }
    }
    Line::Entries(entries, negated, repeated)
}

/// The comment after the key: tabs become spaces, cut to 256 characters;
/// `None` when empty or when it holds other control characters.
fn comment_of(rest: &str) -> Option<String> {
    let comment: String = rest
        .trim_matches([' ', '\t'])
        .replace('\t', " ")
        .chars()
        .take(MAX_COMMENT_CHARS)
        .collect();
    text::string(&comment, MAX_COMMENT_CHARS).ok().flatten()
}

/// One host pattern of the host field: `(host, port)`. `[host]:port` gives
/// the port; a hashed entry or a plain pattern has none (22). `None` when the
/// pattern is malformed.
fn split_pattern(pattern: &str) -> Option<(&str, Option<u16>)> {
    if pattern.is_empty() {
        return None;
    }
    if let Some(inner) = pattern.strip_prefix('[') {
        let (host, port) = inner.split_once("]:")?;
        if host.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let port: u16 = port.parse().ok().filter(|port| *port != 0)?;
        // A hashed host is never inside brackets: its port is in the hash.
        if matches!(classify(host), HostKind::Hashed { .. } | HostKind::Invalid) {
            return None;
        }
        return Some((host, Some(port)));
    }
    Some((pattern, None))
}

/// The host field of an entry: hashed entries as stored, `[host]:port` for
/// a port other than 22, the host otherwise.
pub fn host_field(entry: &Entry) -> String {
    match entry.port {
        Some(port) if port != 22 => format!("[{}]:{port}", entry.host),
        _ => entry.host.clone(),
    }
}

/// The line of one entry (without the line break).
pub fn line_of(entry: &Entry) -> String {
    let key = decode_blob(&entry.public_key)
        .map(|blob| super::encode_blob(&blob))
        .unwrap_or_else(|| entry.public_key.clone());
    let line = format!("{} {} {key}", host_field(entry), entry.key_type);
    if entry.revoked {
        format!("@revoked {line}")
    } else {
        line
    }
}

/// A known_hosts file of `entries` (docs/storage-format.md, section 13.6):
/// one line per entry, sorted by host field, key type and key, each ending
/// in LF. Empty for no entries.
pub fn write(entries: &[Entry]) -> String {
    let mut lines: Vec<(String, String, String)> = entries
        .iter()
        .map(|entry| (host_field(entry), entry.key_type.clone(), line_of(entry)))
        .collect();
    lines.sort();
    let mut file = String::new();
    for (_, _, line) in lines {
        file.push_str(&line);
        file.push('\n');
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::known_hosts::{encode_blob, entry_matches, hash_host};

    /// A valid Ed25519 public key blob (32 bytes of `tag`), base64.
    fn ed25519(tag: u8) -> String {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[tag; 32]);
        encode_blob(&blob)
    }

    fn hosts(file: &ParsedFile) -> Vec<(String, Option<u16>, bool)> {
        file.entries
            .iter()
            .map(|entry| (entry.host.clone(), entry.port, entry.revoked))
            .collect()
    }

    #[test]
    fn plain_bracketed_pattern_and_comma_separated_hosts() {
        let key = ed25519(1);
        let input = format!(
            "# a comment line\n\
             \n\
             Web1.Example.com,192.0.2.10 ssh-ed25519 {key} imported from a laptop\n\
             [db.example.com]:2222 ssh-ed25519 {key}\r\n\
             \t*.corp.example.com ssh-ed25519 {key}\n\
             [*.example.org]:2200,2001:db8::1 ssh-ed25519 {key}\n"
        );
        let file = parse(input.as_bytes());
        assert_eq!(file.invalid, 0);
        assert_eq!(file.skipped, 0);
        assert_eq!(
            hosts(&file),
            vec![
                ("web1.example.com".into(), None, false),
                ("192.0.2.10".into(), None, false),
                ("db.example.com".into(), Some(2222), false),
                ("*.corp.example.com".into(), None, false),
                ("*.example.org".into(), Some(2200), false),
                ("2001:db8::1".into(), None, false),
            ]
        );
        assert_eq!(
            file.entries[0].comment.as_deref(),
            Some("imported from a laptop")
        );
        assert_eq!(file.entries[2].comment, None);
        assert!(
            file.entries
                .iter()
                .all(|entry| entry.source == KnownHostSource::Import && entry.public_key == key)
        );
    }

    #[test]
    fn hashed_entries_are_kept_as_they_are_and_still_match() {
        let key = ed25519(2);
        let salt = [5u8; 20];
        let hashed = hash_host(&salt, "h.example.com", 22);
        let on_port = hash_host(&salt, "h.example.com", 2222);
        let input = format!("{hashed} ssh-ed25519 {key}\n{on_port} ssh-ed25519 {key}\n");
        let file = parse(input.as_bytes());
        assert_eq!(file.invalid, 0);
        assert_eq!(file.entries[0].host, hashed);
        assert_eq!(file.entries[0].port, None);
        assert!(entry_matches(
            &file.entries[0].host,
            None,
            "h.example.com",
            22
        ));
        assert!(entry_matches(
            &file.entries[1].host,
            None,
            "h.example.com",
            2222
        ));
        // A hashed host in brackets is malformed.
        let bracketed = format!("[{hashed}]:2222 ssh-ed25519 {key}\n");
        assert_eq!(parse(bracketed.as_bytes()).invalid, 1);
    }

    #[test]
    fn markers_negations_and_invalid_lines_are_counted() {
        let key = ed25519(3);
        let input = format!(
            "@revoked old.example.net ssh-ed25519 {key}\n\
             @cert-authority *.example.com ssh-ed25519 {key}\n\
             @cert-authority\n\
             !bad.example.com,good.example.com ssh-ed25519 {key}\n\
             @unknown h.example.com ssh-ed25519 {key}\n\
             h.example.com ssh-ed25519\n\
             h.example.com ssh-rsa {key}\n\
             h.example.com ssh-ed25519 not-base64!\n\
             h.example.com 2048 65537 12345678901234567890\n\
             [h.example.com]:0 ssh-ed25519 {key}\n\
             [h.example.com]:x ssh-ed25519 {key}\n\
             [h.example.com ssh-ed25519 {key}\n\
             a,,b ssh-ed25519 {key}\n\
             bad[name ssh-ed25519 {key}\n\
             @revoked\n"
        );
        let file = parse(input.as_bytes());
        assert_eq!(
            hosts(&file),
            vec![
                ("old.example.net".into(), None, true),
                ("good.example.com".into(), None, false),
            ]
        );
        // Two @cert-authority lines and one negated pattern.
        assert_eq!(file.skipped, 3);
        assert_eq!(file.invalid, 11);
    }

    #[test]
    fn limits_encoding_and_comments() {
        let key = ed25519(4);
        let long = format!("{} ssh-ed25519 {key}\n", "a".repeat(MAX_LINE_BYTES));
        assert_eq!(parse(long.as_bytes()).invalid, 1);
        let mut not_utf8 = b"h\xFF.example.com ssh-ed25519 ".to_vec();
        not_utf8.extend_from_slice(key.as_bytes());
        assert_eq!(parse(&not_utf8).invalid, 1);
        // A byte order mark at the start is ignored.
        let bom = format!("\u{feff}h.example.com ssh-ed25519 {key}");
        let file = parse(bom.as_bytes());
        assert_eq!((file.entries.len(), file.invalid), (1, 0));
        // A host over 1024 characters is invalid.
        let host = format!("{} ssh-ed25519 {key}", "h".repeat(1025));
        assert_eq!(parse(host.as_bytes()).invalid, 1);
        // Comments: tabs become spaces, long ones are cut, control
        // characters drop them.
        let input = format!(
            "a.example.com ssh-ed25519 {key} one\ttwo\n\
             b.example.com ssh-ed25519 {key} {}\n\
             c.example.com ssh-ed25519 {key} bell\u{7}\n",
            "x".repeat(300)
        );
        let file = parse(input.as_bytes());
        assert_eq!(file.entries[0].comment.as_deref(), Some("one two"));
        assert_eq!(
            file.entries[1]
                .comment
                .as_deref()
                .map(|c| c.chars().count()),
            Some(256)
        );
        assert_eq!(file.entries[2].comment, None);
        assert_eq!(parse(b"").entries.len(), 0);
    }

    /// A hostile file stays within the limits: a line of thousands of
    /// patterns is invalid, a pattern repeated on its line is one entry,
    /// and the entries of a file stop at their count and their text.
    #[test]
    fn patterns_entries_and_their_text_are_bounded() {
        let key = ed25519(9);
        // 10 000 patterns: invalid, nothing of it is read.
        let many: Vec<String> = (0..10_000).map(|n| format!("h{n}")).collect();
        let line = format!("{} ssh-ed25519 {key}\n", many.join(","));
        let file = parse(line.as_bytes());
        assert_eq!((file.entries.len(), file.invalid), (0, 1));
        // The limit itself is fine, one more is not.
        let at_limit: Vec<String> = (0..MAX_PATTERNS_PER_LINE)
            .map(|n| format!("h{n}"))
            .collect();
        let line = format!("{} ssh-ed25519 {key}\n", at_limit.join(","));
        let file = parse(line.as_bytes());
        assert_eq!(
            (file.entries.len(), file.invalid),
            (MAX_PATTERNS_PER_LINE, 0)
        );
        assert!(file.entries.iter().all(|entry| entry.public_key == key));
        let over = format!("x,{}", at_limit.join(","));
        assert_eq!(
            parse(format!("{over} ssh-ed25519 {key}\n").as_bytes()).invalid,
            1
        );
        // The same pattern again on its line: one entry, the rest are
        // duplicates (`[host]:22` is the same entry).
        let same = vec!["a.example.com"; 200].join(",");
        let file = parse(format!("{same},[a.example.com]:22 ssh-ed25519 {key}\n").as_bytes());
        assert_eq!(hosts(&file), vec![("a.example.com".into(), None, false)]);
        assert_eq!((file.duplicates, file.invalid), (200, 0));

        // The entry count: lines past it are invalid.
        let per_line: Vec<String> = (0..MAX_PATTERNS_PER_LINE)
            .map(|n| format!("p{n}"))
            .collect();
        let per_line = per_line.join(",");
        let taken = MAX_IMPORT_ENTRIES / MAX_PATTERNS_PER_LINE;
        let lines = taken + 3;
        let input = format!("{per_line} ssh-ed25519 {key}\n").repeat(lines);
        let file = parse(input.as_bytes());
        assert_eq!(file.entries.len(), taken * MAX_PATTERNS_PER_LINE);
        assert!(file.entries.len() <= MAX_IMPORT_ENTRIES);
        assert_eq!(file.invalid, lines - taken);

        // The text: a long comment copied by every pattern stops at the
        // budget long before the entry count.
        let comment = "c".repeat(MAX_COMMENT_CHARS);
        let line = format!("{per_line} ssh-ed25519 {key} {comment}\n");
        let per_line_bytes: usize = parse(line.as_bytes()).entries.iter().map(entry_bytes).sum();
        let fit = MAX_IMPORT_ENTRY_BYTES / per_line_bytes;
        let input = line.repeat(fit + 2);
        let file = parse(input.as_bytes());
        let total: usize = file.entries.iter().map(entry_bytes).sum();
        assert!(total <= MAX_IMPORT_ENTRY_BYTES);
        assert_eq!(file.entries.len(), fit * MAX_PATTERNS_PER_LINE);
        assert_eq!(file.invalid, 2);
    }

    #[test]
    fn export_writes_the_storage_format_and_reads_back_the_same() {
        let key = ed25519(6);
        let other = ed25519(7);
        let hashed = hash_host(&[8u8; 20], "h.example.com", 22);
        let input = format!(
            "zulu.example.com ssh-ed25519 {key} comment\n\
             [alpha.example.com]:2222 ssh-ed25519 {other}\n\
             @revoked old.example.net ssh-ed25519 {other}\n\
             {hashed} ssh-ed25519 {key}\n\
             *.example.org ssh-ed25519 {key}\n"
        );
        let file = parse(input.as_bytes());
        let written = write(&file.entries);
        assert_eq!(
            written,
            format!(
                "*.example.org ssh-ed25519 {key}\n\
                 [alpha.example.com]:2222 ssh-ed25519 {other}\n\
                 @revoked old.example.net ssh-ed25519 {other}\n\
                 zulu.example.com ssh-ed25519 {key}\n\
                 {hashed} ssh-ed25519 {key}\n"
            )
        );
        let again = parse(written.as_bytes());
        assert_eq!(again.invalid, 0);
        assert_eq!(again.entries.len(), file.entries.len());
        for entry in &file.entries {
            assert!(
                again
                    .entries
                    .iter()
                    .any(|read| known_host::same_natural_key(read, entry))
            );
        }
        assert_eq!(write(&[]), "");
    }
}
