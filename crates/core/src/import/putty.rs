//! PuTTY saved sessions for the import of UI 2.5 (SPEC "Import"; run 8
//! decision D140): the values of each session and the readers of the
//! formats PuTTY stores in them.
//!
//! - PuTTY keeps a session as a registry key under
//!   `HKCU\Software\SimonTatham\PuTTY\Sessions`: the key name is the session
//!   name with `%XX` escapes ([`session_name`]), each value a string or a
//!   DWORD ([`Value`]). The registry is read by `shellpp-desktop` on
//!   Windows behind [`SessionSource`]; tests use a fake.
//! - `PortForwardings` and `Environment` are maps: `key=value` entries
//!   separated by `,`, with `\` escaping `,`, `=` and `\` ([`map_entries`]).
//!   A dynamic forwarding is `D<port>` with an empty value (or none, in
//!   older versions).
//! - `LineCodePage` names a character set the way PuTTY's menu does
//!   (`ISO-8859-2:1999 (Latin-2, East Europe)`, `Win1251 (Cyrillic)`);
//!   [`charset_label`] turns it into a WHATWG label.
//!
//! Nothing here logs names or values.

use std::collections::BTreeMap;
use std::fmt;

/// The registry key of the saved sessions, under `HKEY_CURRENT_USER`.
pub const SESSIONS_KEY: &str = r"Software\SimonTatham\PuTTY\Sessions";

/// The session PuTTY applies to new sessions: a template, never a host.
pub const DEFAULT_SETTINGS: &str = "Default Settings";

/// Most sessions read.
pub const MAX_SESSIONS: usize = 10_000;
/// The longest string value kept (longer ones are dropped as unreadable).
pub const MAX_VALUE_CHARS: usize = 32 * 1024;

/// One value of a session.
#[derive(Clone, PartialEq, Eq)]
pub enum Value {
    /// `REG_SZ` (and `REG_EXPAND_SZ`, unexpanded).
    Text(String),
    /// `REG_DWORD`.
    Number(u32),
}

impl fmt::Debug for Value {
    /// Never prints a string: `ProxyPassword` is one.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => write!(f, "Text(<{} chars>)", text.chars().count()),
            Self::Number(number) => write!(f, "Number({number})"),
        }
    }
}

/// One saved session: the registry key name as it is (with its `%XX`
/// escapes) and its values by name.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub key: String,
    pub values: BTreeMap<String, Value>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("values", &self.values.len())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// The session name PuTTY shows ([`session_name`] of the key), bytes
    /// that are not UTF-8 read in the ANSI `code_page` of Windows.
    pub fn name(&self, code_page: Option<u32>) -> String {
        session_name(&self.key, code_page)
    }

    /// A string value; a DWORD is not one.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.values.get(name) {
            Some(Value::Text(text)) => Some(text.as_str()),
            _ => None,
        }
    }

    /// A non-empty string value, trimmed.
    pub fn nonempty(&self, name: &str) -> Option<&str> {
        self.text(name)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// A DWORD value; PuTTY writes numbers as DWORDs, a decimal string is
    /// accepted too.
    pub fn number(&self, name: &str) -> Option<u32> {
        match self.values.get(name) {
            Some(Value::Number(number)) => Some(*number),
            Some(Value::Text(text)) => text.trim().parse().ok(),
            None => None,
        }
    }
}

/// What a [`SessionSource`] read: the sessions, the keys it could not read
/// (by key name), and the ANSI code page of Windows (`GetACP`), which
/// older PuTTY versions escape non-ASCII session names in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sessions {
    pub sessions: Vec<Session>,
    pub unreadable: Vec<String>,
    pub code_page: Option<u32>,
}

/// Where the saved sessions come from: the Windows registry in
/// `shellpp-desktop`, a fixed list in tests and the demo.
pub trait SessionSource: Send + Sync {
    /// Every saved session (at most [`MAX_SESSIONS`]); `Ok` with none when
    /// PuTTY has never saved one.
    fn sessions(&self) -> std::io::Result<Sessions>;
}

/// A fixed list of sessions (tests, the demo, and other systems than
/// Windows, where there are none).
#[derive(Clone, Debug, Default)]
pub struct FixedSessions(pub Sessions);

impl SessionSource for FixedSessions {
    fn sessions(&self) -> std::io::Result<Sessions> {
        Ok(self.0.clone())
    }
}

/// The character set of a Windows ANSI code page; Windows-1252 when there
/// is none or this app cannot read it.
fn ansi_encoding(code_page: Option<u32>) -> &'static encoding_rs::Encoding {
    let label = match code_page {
        Some(932) => "shift_jis".to_owned(),
        Some(936) => "gbk".to_owned(),
        Some(949) => "euc-kr".to_owned(),
        Some(950) => "big5".to_owned(),
        Some(code_page) => format!("windows-{code_page}"),
        None => "windows-1252".to_owned(),
    };
    encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
}

/// The session name of a registry key name: PuTTY writes a space, `\`,
/// `*`, `?`, `%`, control and non-ASCII bytes and a leading `.` as `%XX`.
/// The bytes are UTF-8 in recent versions; older ones wrote the ANSI code
/// page of Windows (`code_page`, Windows-1252 when unknown).
pub fn session_name(key: &str, code_page: Option<u32>) -> String {
    let bytes = key.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(hex) = key.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    match String::from_utf8(out) {
        Ok(name) => name,
        Err(error) => ansi_encoding(code_page)
            .decode_without_bom_handling(error.as_bytes())
            .0
            .into_owned(),
    }
}

/// The entries of a PuTTY map value (`PortForwardings`, `Environment`):
/// `(key, value)`, the value `None` when the entry has no `=`.
pub fn map_entries(text: &str) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    let mut key = String::new();
    let mut value: Option<String> = None;
    let mut chars = text.chars();
    let flush = |key: &mut String, value: &mut Option<String>, entries: &mut Vec<_>| {
        if !key.is_empty() || value.is_some() {
            entries.push((std::mem::take(key), value.take()));
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    value.as_mut().unwrap_or(&mut key).push(escaped);
                }
            }
            ',' => flush(&mut key, &mut value, &mut entries),
            '=' if value.is_none() => value = Some(String::new()),
            c => value.as_mut().unwrap_or(&mut key).push(c),
        }
    }
    flush(&mut key, &mut value, &mut entries);
    entries
}

/// The kind of a PuTTY port forwarding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

/// One entry of `PortForwardings`: `[4|6](L|R|D)[address:]port[=destination]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forward {
    pub kind: ForwardKind,
    /// `4` or `6` when the entry is limited to one address family.
    pub family: Option<char>,
    /// The listening side as written: `port` or `address:port`.
    pub source: String,
    /// `host:port` of a local or remote forwarding.
    pub destination: Option<String>,
    /// The entry as PuTTY stored it (for the notes).
    pub raw: String,
}

/// The forwardings of a `PortForwardings` value; entries that are not one
/// are returned as they were written.
pub fn forwards(text: &str) -> (Vec<Forward>, Vec<String>) {
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for (key, value) in map_entries(text) {
        let raw = match &value {
            Some(value) => format!("{key}={value}"),
            None => key.clone(),
        };
        let mut rest = key.as_str();
        let family = match rest.chars().next() {
            Some(c @ ('4' | '6')) => {
                rest = &rest[1..];
                Some(c)
            }
            _ => None,
        };
        let kind = match rest.chars().next() {
            Some('L') => ForwardKind::Local,
            Some('R') => ForwardKind::Remote,
            Some('D') => ForwardKind::Dynamic,
            _ => {
                bad.push(raw);
                continue;
            }
        };
        let source = rest[1..].to_owned();
        let destination = value.filter(|value| !value.is_empty());
        let complete = match kind {
            ForwardKind::Dynamic => destination.is_none(),
            ForwardKind::Local | ForwardKind::Remote => destination.is_some(),
        };
        if source.is_empty() || !complete {
            bad.push(raw);
            continue;
        }
        out.push(Forward {
            kind,
            family,
            source,
            destination,
            raw,
        });
    }
    (out, bad)
}

/// Whether PuTTY connects to `host` (the host name as written) without
/// its proxy, as its `proxy_for_destination` decides without resolving
/// the name: a local name (`localhost`, `127.*`, `::1`) unless
/// `proxy_localhost`, or a match in `exclude_list`. The list's entries are
/// separated by commas or spaces; `*suffix` and `prefix*` match the ends
/// of the name, any other entry its start, all case-insensitively.
pub fn proxy_bypassed(host: &str, exclude_list: &str, proxy_localhost: bool) -> bool {
    let local = host == "localhost" || host == "::1" || host.starts_with("127.");
    if local && !proxy_localhost {
        return true;
    }
    let host = host.to_ascii_lowercase();
    exclude_list
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|entry| {
            // PuTTY reads an entry up to the first character that is not
            // a letter, digit, `-`, `.` or `*`.
            let end = entry
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '*')))
                .unwrap_or(entry.len());
            entry[..end].to_ascii_lowercase()
        })
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if let Some(suffix) = entry.strip_prefix('*') {
                host.ends_with(suffix)
            } else if let Some(prefix) = entry.strip_suffix('*') {
                host.starts_with(prefix)
            } else {
                host.starts_with(&entry)
            }
        })
}

/// The WHATWG label of a PuTTY `LineCodePage`, or `None` for one this app
/// cannot decode ("Use font encoding", "CP437", "VSCII" …). An empty value
/// is PuTTY's default, UTF-8.
pub fn charset_label(code_page: &str) -> Option<String> {
    let name = code_page.trim();
    if name.is_empty() {
        return Some("utf-8".to_owned());
    }
    // "ISO-8859-2:1999 (Latin-2, East Europe)" → "ISO-8859-2".
    let name = name.split(" (").next().unwrap_or(name);
    let name = name.split(':').next().unwrap_or(name).trim();
    let lower = name.to_ascii_lowercase();
    let label = if let Some(number) = lower.strip_prefix("win") {
        format!("windows-{number}")
    } else {
        lower
    };
    encoding_rs::Encoding::for_label(label.as_bytes())
        .map(|encoding| encoding.name().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_names_are_unescaped() {
        assert_eq!(session_name("web%20prod", None), "web prod");
        assert_eq!(session_name("%2Edot%25", None), ".dot%");
        assert_eq!(session_name("plain", None), "plain");
        // A broken escape stays as it is.
        assert_eq!(session_name("50%zz", None), "50%zz");
        assert_eq!(session_name("end%2", None), "end%2");
        // UTF-8 bytes, and ANSI bytes that are not UTF-8: Windows-1252
        // unless the code page says otherwise.
        assert_eq!(session_name("%D0%B1%D0%B4", Some(1251)), "бд");
        assert_eq!(session_name("caf%E9", None), "café");
        assert_eq!(session_name("%D1%E5%F0%E2%E5%F0", Some(1251)), "Сервер");
    }

    #[test]
    fn local_and_excluded_hosts_bypass_the_proxy() {
        assert!(proxy_bypassed("127.0.0.1", "", false));
        assert!(proxy_bypassed("localhost", "", false));
        assert!(proxy_bypassed("::1", "", false));
        assert!(!proxy_bypassed("127.0.0.1", "", true));
        assert!(!proxy_bypassed("web.example.com", "", false));
        let list = "*.corp.example, 10.*  intranet;junk";
        assert!(proxy_bypassed("db.CORP.example", list, false));
        assert!(proxy_bypassed("10.1.2.3", list, false));
        assert!(proxy_bypassed("intranet.example", list, false));
        assert!(!proxy_bypassed("corp.example", list, false));
        assert!(!proxy_bypassed("192.0.2.1", list, false));
    }

    #[test]
    fn map_values_with_escapes() {
        assert_eq!(
            map_entries(r"LANG=C,PS1=a\,b\=c\\d,EMPTY="),
            [
                ("LANG".to_owned(), Some("C".to_owned())),
                ("PS1".to_owned(), Some(r"a,b=c\d".to_owned())),
                ("EMPTY".to_owned(), Some(String::new())),
            ]
        );
        assert_eq!(
            map_entries("D1080,L80=x:80"),
            [
                ("D1080".to_owned(), None),
                ("L80".to_owned(), Some("x:80".to_owned())),
            ]
        );
        assert!(map_entries("").is_empty());
    }

    #[test]
    fn port_forwardings() {
        let (found, bad) = forwards(
            "L8080=localhost:80,4R2222=10.0.0.5:22,D1080=,6D1081,\
             L127.0.0.1:9000=db:5432,X1=y,L9=,R5",
        );
        let kinds: Vec<_> = found
            .iter()
            .map(|forward| (forward.kind, forward.family, forward.source.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (ForwardKind::Local, None, "8080"),
                (ForwardKind::Remote, Some('4'), "2222"),
                (ForwardKind::Dynamic, None, "1080"),
                (ForwardKind::Dynamic, Some('6'), "1081"),
                (ForwardKind::Local, None, "127.0.0.1:9000"),
            ]
        );
        assert_eq!(found[0].destination.as_deref(), Some("localhost:80"));
        assert_eq!(found[2].destination, None);
        assert_eq!(bad, ["X1=y", "L9=", "R5"]);
    }

    #[test]
    fn code_pages_become_whatwg_labels() {
        assert_eq!(charset_label("UTF-8").as_deref(), Some("utf-8"));
        assert_eq!(charset_label("").as_deref(), Some("utf-8"));
        assert_eq!(
            charset_label("ISO-8859-2:1999 (Latin-2, East Europe)").as_deref(),
            Some("iso-8859-2")
        );
        assert_eq!(
            charset_label("Win1251 (Cyrillic)").as_deref(),
            Some("windows-1251")
        );
        assert_eq!(charset_label("KOI8-U").as_deref(), Some("koi8-u"));
        assert_eq!(charset_label("CP866").as_deref(), Some("ibm866"));
        assert_eq!(charset_label("Use font encoding"), None);
        assert_eq!(charset_label("CP437"), None);
    }

    #[test]
    fn values_never_print_their_text() {
        let session = Session {
            key: "s".into(),
            values: BTreeMap::from([("ProxyPassword".into(), Value::Text("hunter2".into()))]),
        };
        let shown = format!("{session:?} {:?}", session.values);
        assert!(!shown.contains("hunter2"), "{shown}");
        assert_eq!(session.number("Missing"), None);
    }
}
