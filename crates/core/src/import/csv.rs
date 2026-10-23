//! CSV files for the import of UI 2.5 (SPEC "Import"; run 8 decision
//! D141): the character set, the delimiter, the records (RFC 4180) and the
//! columns.
//!
//! - The character set comes from a byte order mark (UTF-8, UTF-16 LE or
//!   BE), else UTF-8 when the bytes are valid UTF-8, else Windows-1251 when
//!   most bytes above ASCII are Cyrillic letters there, else Windows-1252
//!   ([`detect_encoding`]). The user can choose another one.
//! - The delimiter is the one of `,`, `;`, tab and `|` that splits the
//!   first records into the same number of fields, more than one
//!   ([`detect_delimiter`]). The user can choose another one.
//! - Fields may be quoted with `"`; a quote inside is doubled; a quoted
//!   field may hold the delimiter and line breaks.
//! - Columns are guessed from a header row ([`guess_roles`]); a file whose
//!   first row names no known column has no header and its columns are
//!   taken in the order of the SPEC (`label, address, port, username,
//!   group, tags`).
//!
//! Nothing here logs the file's content.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// The largest file read.
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// Most records read (a header included).
pub const MAX_RECORDS: usize = 100_000;
/// Most fields of one record.
pub const MAX_FIELDS: usize = 256;

/// The delimiters [`detect_delimiter`] chooses from.
pub const DELIMITERS: [char; 4] = [',', ';', '\t', '|'];

/// The character sets offered for a CSV file, as WHATWG labels.
pub const ENCODINGS: [&str; 8] = [
    "utf-8",
    "utf-16le",
    "utf-16be",
    "windows-1251",
    "windows-1252",
    "koi8-r",
    "ibm866",
    "iso-8859-2",
];

/// What a column holds (D141).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Not imported.
    Ignore,
    Label,
    Address,
    Port,
    Username,
    /// A group path, ` / ` between levels.
    Group,
    /// Tags separated by `,` or `;`.
    Tags,
    Notes,
    /// A password, stored as the host's secret (the dialog warns first).
    Password,
}

impl Role {
    /// The roles a column can take, in the order the dialog offers them.
    pub const ALL: [Self; 9] = [
        Self::Ignore,
        Self::Label,
        Self::Address,
        Self::Port,
        Self::Username,
        Self::Group,
        Self::Tags,
        Self::Notes,
        Self::Password,
    ];

    /// The SPEC's column order of a file without a header.
    pub const ORDER: [Self; 6] = [
        Self::Label,
        Self::Address,
        Self::Port,
        Self::Username,
        Self::Group,
        Self::Tags,
    ];

    /// The column name of this role in the template and in a header.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ignore => "",
            Self::Label => "label",
            Self::Address => "address",
            Self::Port => "port",
            Self::Username => "username",
            Self::Group => "group",
            Self::Tags => "tags",
            Self::Notes => "notes",
            Self::Password => "password",
        }
    }

    /// The role a header names, by its usual names (case-insensitive).
    pub fn from_header(header: &str) -> Option<Self> {
        let name: String = header
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();
        Some(match name.as_str() {
            "label" | "name" | "title" | "alias" => Self::Label,
            "address" | "host" | "hostname" | "ip" | "server" | "addr" => Self::Address,
            "port" => Self::Port,
            "username" | "user" | "login" => Self::Username,
            "group" | "folder" | "path" | "groups" => Self::Group,
            "tags" | "tag" => Self::Tags,
            "notes" | "note" | "description" | "comment" | "comments" => Self::Notes,
            "password" | "pass" | "pwd" | "passwd" => Self::Password,
            _ => return None,
        })
    }
}

/// The template the dialog saves (D141): the header and one example row.
pub const TEMPLATE: &str = "label,address,port,username,group,tags,notes\r\n\
                            Web server,web.example.com,22,deploy,Production / Web,\"prod,web\",\r\n";

/// The text of `bytes` in `label` (a WHATWG label); a byte order mark of
/// that character set is removed. `None` for an unknown label.
pub fn decode<'a>(bytes: &'a [u8], label: &str) -> Option<Cow<'a, str>> {
    let encoding = encoding_rs::Encoding::for_label(label.as_bytes())?;
    let (text, _) = encoding.decode_with_bom_removal(bytes);
    Some(text)
}

/// The character set of `bytes`, as a WHATWG label.
pub fn detect_encoding(bytes: &[u8]) -> &'static str {
    if let Some((encoding, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return match encoding.name() {
            "UTF-16LE" => "utf-16le",
            "UTF-16BE" => "utf-16be",
            _ => "utf-8",
        };
    }
    if std::str::from_utf8(bytes).is_ok() {
        return "utf-8";
    }
    // Windows-1251 puts Cyrillic letters at 0xC0-0xFF; Windows-1252 text
    // has fewer bytes there than elsewhere above ASCII (and Russian text
    // in 1252 terms would be mostly accented capitals).
    let high: Vec<u8> = bytes.iter().copied().filter(|b| *b >= 0x80).collect();
    let letters = high.iter().filter(|b| **b >= 0xC0).count();
    if !high.is_empty() && letters * 10 >= high.len() * 7 {
        "windows-1251"
    } else {
        "windows-1252"
    }
}

/// Splits `text` into records of fields with `delimiter` (RFC 4180, with
/// LF or CRLF line ends). Empty lines are skipped. Stops after
/// [`MAX_RECORDS`]; fields past [`MAX_FIELDS`] are dropped.
pub fn records(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // The field started with a quote (so an empty quoted field counts).
    let mut was_quoted = false;
    let mut chars = text.chars().peekable();
    let end_field = |record: &mut Vec<String>, field: &mut String| {
        if record.len() < MAX_FIELDS {
            record.push(std::mem::take(field));
        } else {
            field.clear();
        }
    };
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            c if c == delimiter => {
                end_field(&mut record, &mut field);
                was_quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                let empty = record.is_empty() && field.is_empty() && !was_quoted;
                if !empty {
                    end_field(&mut record, &mut field);
                    out.push(std::mem::take(&mut record));
                    if out.len() >= MAX_RECORDS {
                        return out;
                    }
                }
                was_quoted = false;
            }
            c => field.push(c),
        }
    }
    if !record.is_empty() || !field.is_empty() || was_quoted {
        end_field(&mut record, &mut field);
        out.push(record);
    }
    out
}

/// The delimiter of `text`: among [`DELIMITERS`], the one that gives the
/// first records the same number of fields (more than one), the most
/// fields winning a tie; `,` when none does.
pub fn detect_delimiter(text: &str) -> char {
    // The first lines are enough (a quoted line break only shortens them).
    let head: String = text.lines().take(20).collect::<Vec<_>>().join("\n");
    let mut best: Option<(char, usize)> = None;
    for delimiter in DELIMITERS {
        let found = records(&head, delimiter);
        let Some(first) = found.first() else { continue };
        let fields = first.len();
        if fields < 2 {
            continue;
        }
        // The last record may be cut by the 20-line limit.
        let consistent = found
            .iter()
            .take(found.len().saturating_sub(1).max(1))
            .all(|record| record.len() == fields);
        if consistent && best.is_none_or(|(_, most)| fields > most) {
            best = Some((delimiter, fields));
        }
    }
    best.map_or(',', |(delimiter, _)| delimiter)
}

/// The roles of the columns of `first` (the first record), and whether it
/// is a header: a header names at least one known column. Without a header
/// the columns take [`Role::ORDER`].
pub fn guess_roles(first: &[String]) -> (Vec<Role>, bool) {
    let named: Vec<Option<Role>> = first.iter().map(|cell| Role::from_header(cell)).collect();
    let header = named.iter().any(Option::is_some);
    if header {
        // Each role once: a second column with the same name is ignored.
        let mut seen = Vec::new();
        let roles = named
            .into_iter()
            .map(|role| match role {
                Some(role) if !seen.contains(&role) => {
                    seen.push(role);
                    role
                }
                _ => Role::Ignore,
            })
            .collect();
        return (roles, true);
    }
    let roles = (0..first.len())
        .map(|index| Role::ORDER.get(index).copied().unwrap_or(Role::Ignore))
        .collect();
    (roles, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|field| (*field).to_owned()).collect())
            .collect()
    }

    #[test]
    fn quoted_fields_line_ends_and_empty_lines() {
        assert_eq!(
            records(
                "a,\"b,c\",\"d \"\"e\"\"\"\r\n\r\nf,,\"\"\n\"line\nbreak\",x",
                ','
            ),
            owned(&[
                &["a", "b,c", "d \"e\""],
                &["f", "", ""],
                &["line\nbreak", "x"]
            ])
        );
        assert_eq!(records("one", ','), owned(&[&["one"]]));
        assert!(records("", ',').is_empty());
        assert!(records("\n\r\n", ',').is_empty());
        // A quote inside an unquoted field is kept as it is.
        assert_eq!(records("ab\"c,d", ','), owned(&[&["ab\"c", "d"]]));
    }

    #[test]
    fn delimiters_are_detected() {
        assert_eq!(detect_delimiter("a;b;c\n1;2;3\n"), ';');
        assert_eq!(detect_delimiter("a\tb\n1\t2\n"), '\t');
        assert_eq!(detect_delimiter("a,b\n1,2\n"), ',');
        // Commas inside quotes do not count.
        assert_eq!(detect_delimiter("\"x,y\";b\n\"1,2\";3\n"), ';');
        assert_eq!(detect_delimiter("single\n"), ',');
        assert_eq!(detect_delimiter("a|b|c\n1|2|3"), '|');
    }

    #[test]
    fn encodings_are_detected_and_decoded() {
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFlabel"), "utf-8");
        assert_eq!(detect_encoding(b"\xFF\xFEl\x00"), "utf-16le");
        assert_eq!(detect_encoding(b"\xFE\xFF\x00l"), "utf-16be");
        assert_eq!(detect_encoding("сервер".as_bytes()), "utf-8");
        // "Сервер" in Windows-1251, and "Café" in Windows-1252.
        let cyrillic = b"\xD1\xE5\xF0\xE2\xE5\xF0,host";
        assert_eq!(detect_encoding(cyrillic), "windows-1251");
        assert_eq!(decode(cyrillic, "windows-1251").unwrap(), "Сервер,host");
        assert_eq!(detect_encoding(b"Caf\xE9 \x80 \x93x\x94"), "windows-1252");
        assert_eq!(decode(b"\xEF\xBB\xBFa", "utf-8").unwrap(), "a");
        assert!(decode(b"a", "no-such-charset").is_none());
    }

    #[test]
    fn headers_are_recognized() {
        let header: Vec<String> = ["Name", "Host Name", "PORT", "Login", "x", "name"]
            .map(str::to_owned)
            .to_vec();
        let (roles, has_header) = guess_roles(&header);
        assert!(has_header);
        assert_eq!(
            roles,
            [
                Role::Label,
                Role::Address,
                Role::Port,
                Role::Username,
                Role::Ignore,
                Role::Ignore
            ]
        );
        let data: Vec<String> = ["web", "10.0.0.1", "22"].map(str::to_owned).to_vec();
        let (roles, has_header) = guess_roles(&data);
        assert!(!has_header);
        assert_eq!(roles, [Role::Label, Role::Address, Role::Port]);
        // The template's own header maps every column.
        let first = records(TEMPLATE, ',').remove(0);
        let (roles, has_header) = guess_roles(&first);
        assert!(has_header);
        assert!(!roles.contains(&Role::Ignore));
    }
}
