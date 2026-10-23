//! CSV files for the import of UI 2.5 (SPEC "Import"; run 8 decision
//! D141): the character set, the delimiter, the records (RFC 4180) and the
//! columns.
//!
//! - The character set comes from a byte order mark (UTF-8, UTF-16 LE or
//!   BE), else UTF-8 when the bytes are valid UTF-8, else Windows-1251 when
//!   the bytes above ASCII are mostly Cyrillic letters there and come in
//!   runs (words), else Windows-1252, whose accented letters stand alone
//!   between ASCII ones ([`detect_encoding`]). The user can choose another
//!   one.
//! - The delimiter is the one of `,`, `;`, tab and `|` that splits the
//!   first records into the same number of fields, more than one
//!   ([`detect_delimiter`]). The user can choose another one.
//! - Fields may be quoted with `"`; a quote inside is doubled; a quoted
//!   field may hold the delimiter and line breaks. A text that ends inside
//!   a quoted field is reported ([`Records::unterminated`]): its last
//!   record would hold the rest of the file.
//! - Columns are guessed from a header row ([`guess_roles`]: English and
//!   Russian names); a first row that names no known column is a header of
//!   columns that are not imported when it looks like one (no port number,
//!   address or `@` in it), else data, and the columns take the order of
//!   the SPEC (`label, address, port, username, group, tags`).
//!
//! Nothing here logs the file's content.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

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
            "label" | "name" | "title" | "alias" | "имя" | "название" | "метка" => {
                Self::Label
            }
            "address" | "host" | "hostname" | "ip" | "server" | "addr" | "адрес" | "хост"
            | "сервер" | "имяхоста" => Self::Address,
            "port" | "порт" => Self::Port,
            "username" | "user" | "login" | "пользователь" | "логин" | "имяпользователя" => {
                Self::Username
            }
            "group" | "folder" | "path" | "groups" | "группа" | "папка" | "путь" => {
                Self::Group
            }
            "tags" | "tag" | "теги" | "тег" => Self::Tags,
            "notes"
            | "note"
            | "description"
            | "comment"
            | "comments"
            | "заметки"
            | "заметка"
            | "описание"
            | "комментарий"
            | "примечание" => Self::Notes,
            "password" | "pass" | "pwd" | "passwd" | "пароль" => Self::Password,
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
    // Windows-1251 puts Cyrillic letters at 0xC0-0xFF, and Russian words
    // are runs of them; Windows-1252 has its accented letters there too
    // (é, ü, ß), but each stands alone between ASCII letters.
    let high = |at: usize| bytes.get(at).is_some_and(|byte| *byte >= 0x80);
    let mut total = 0usize;
    let mut letters = 0usize;
    let mut in_runs = 0usize;
    for (at, byte) in bytes.iter().enumerate() {
        if *byte < 0x80 {
            continue;
        }
        total += 1;
        if *byte >= 0xC0 {
            letters += 1;
        }
        if (at > 0 && high(at - 1)) || high(at + 1) {
            in_runs += 1;
        }
    }
    if total > 0 && letters * 10 >= total * 7 && in_runs * 3 >= total * 2 {
        "windows-1251"
    } else {
        "windows-1252"
    }
}

/// The records of a CSV text, wiped when dropped (the file may hold
/// passwords).
#[derive(Default)]
pub struct Records {
    pub rows: Vec<Vec<String>>,
    /// The text ended inside a quoted field: the last record holds the
    /// rest of the file.
    pub unterminated: bool,
    /// The text has more than [`MAX_RECORDS`] records; the rest was not read.
    pub truncated: bool,
}

impl Drop for Records {
    fn drop(&mut self) {
        for field in self.rows.iter_mut().flatten() {
            field.zeroize();
        }
    }
}

impl std::fmt::Debug for Records {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Records")
            .field("rows", &self.rows.len())
            .field("unterminated", &self.unterminated)
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// Splits `text` into records of fields with `delimiter` (RFC 4180, with
/// LF or CRLF line ends). Empty lines are skipped. Stops after
/// [`MAX_RECORDS`] ([`Records::truncated`]); fields past [`MAX_FIELDS`]
/// are dropped.
pub fn records(text: &str, delimiter: char) -> Records {
    let mut out = Records::default();
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
                    out.rows.push(std::mem::take(&mut record));
                    if out.rows.len() >= MAX_RECORDS {
                        out.truncated = chars.any(|c| !c.is_whitespace());
                        return out;
                    }
                }
                was_quoted = false;
            }
            c => field.push(c),
        }
    }
    out.unterminated = quoted;
    if !record.is_empty() || !field.is_empty() || was_quoted {
        end_field(&mut record, &mut field);
        out.rows.push(record);
    }
    field.zeroize();
    out
}

/// The delimiter of `text`: among [`DELIMITERS`], the one that gives the
/// first records the same number of fields (more than one), the most
/// fields winning a tie; `,` when none does.
pub fn detect_delimiter(text: &str) -> char {
    // The first lines are enough (a quoted line break only shortens them).
    let head = Zeroizing::new(text.lines().take(20).collect::<Vec<_>>().join("\n"));
    let mut best: Option<(char, usize)> = None;
    for delimiter in DELIMITERS {
        let found = records(&head, delimiter);
        let found = &found.rows;
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

/// Whether a cell looks like data rather than a column name: a port
/// number, an IP address, or text with a `.` or `@` (a host name, a user).
fn data_like(cell: &str) -> bool {
    let cell = cell.trim();
    cell.parse::<u16>().is_ok_and(|port| port > 0)
        || cell.parse::<std::net::IpAddr>().is_ok()
        || cell.contains(['.', '@'])
}

/// The roles of the columns of `first` (the first record), and whether it
/// is a header: a header names at least one known column, or names none
/// but looks like one (no cell looks like data), and then no column is
/// imported until the user maps it. Otherwise the columns take
/// [`Role::ORDER`].
pub fn guess_roles(first: &[String]) -> (Vec<Role>, bool) {
    let named: Vec<Option<Role>> = first.iter().map(|cell| Role::from_header(cell)).collect();
    let header = named.iter().any(Option::is_some);
    if !header && !first.iter().any(|cell| data_like(cell)) {
        return (vec![Role::Ignore; first.len()], true);
    }
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
        let found = records(
            "a,\"b,c\",\"d \"\"e\"\"\"\r\n\r\nf,,\"\"\n\"line\nbreak\",x",
            ',',
        );
        assert_eq!(
            found.rows,
            owned(&[
                &["a", "b,c", "d \"e\""],
                &["f", "", ""],
                &["line\nbreak", "x"]
            ])
        );
        assert!(!found.unterminated && !found.truncated);
        assert_eq!(records("one", ',').rows, owned(&[&["one"]]));
        assert!(records("", ',').rows.is_empty());
        assert!(records("\n\r\n", ',').rows.is_empty());
        // A quote inside an unquoted field is kept as it is.
        assert_eq!(records("ab\"c,d", ',').rows, owned(&[&["ab\"c", "d"]]));
    }

    #[test]
    fn an_open_quote_and_too_many_records_are_reported() {
        let found = records("a,\"see wiki,pw1\nb,c,pw2\n", ',');
        assert!(found.unterminated);
        assert_eq!(found.rows.len(), 1);
        let many = "x\n".repeat(MAX_RECORDS + 1);
        let found = records(&many, ',');
        assert!(found.truncated);
        assert_eq!(found.rows.len(), MAX_RECORDS);
        // Exactly the limit, with a trailing line end, is complete.
        let found = records(&"x\n".repeat(MAX_RECORDS), ',');
        assert!(!found.truncated);
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
        // German names: accented letters alone between ASCII ones.
        assert_eq!(
            detect_encoding(b"M\xFCller,host\nG\xF6\xDFe,x\nJ\xE4ger,y"),
            "windows-1252"
        );
        assert_eq!(
            detect_encoding(b"\xCC\xEE\xF1\xEA\xE2\xE0,\xEF\xF0\xEE\xE4"),
            "windows-1251"
        );
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
        let first = records(TEMPLATE, ',').rows[0].clone();
        let (roles, has_header) = guess_roles(&first);
        assert!(has_header);
        assert!(!roles.contains(&Role::Ignore));
        // Russian names.
        let header: Vec<String> = ["Имя", "Адрес", "Порт", "Пользователь", "Пароль"]
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
                Role::Password
            ]
        );
        // Names this app does not know: a header whose columns are not
        // imported (never a password taken for a group by position).
        let unknown: Vec<String> = ["Nom", "Hôte", "Mot de passe"].map(str::to_owned).to_vec();
        assert_eq!(guess_roles(&unknown), (vec![Role::Ignore; 3], true));
        let words: Vec<String> = ["db", "db1"].map(str::to_owned).to_vec();
        assert_eq!(guess_roles(&words), (vec![Role::Ignore; 2], true));
        // A first row of data: a host name or a port.
        let data: Vec<String> = ["db", "db.lan"].map(str::to_owned).to_vec();
        assert_eq!(
            guess_roles(&data),
            (vec![Role::Label, Role::Address], false)
        );
    }
}
