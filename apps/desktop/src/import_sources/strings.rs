//! Interface strings of the import dialog (UI 2.5; en; D39).

pub const TITLE: &str = "Import";
pub const SUMMARY_TITLE: &str = "Import Finished";

pub const SOURCE_SSH_CONFIG: &str = "OpenSSH config";
/// The file of the OpenSSH config source, under its name.
pub const SOURCE_SSH_CONFIG_PATH: &str = "~/.ssh/config";
pub const SOURCE_JSON: &str = "Shell++ export (JSON)";
pub const SOURCE_KNOWN_HOSTS: &str = "Known hosts";
/// The file of the known hosts source, under its name.
pub const SOURCE_KNOWN_HOSTS_PATH: &str = "~/.ssh/known_hosts";
pub const SOURCE_PUTTY: &str = "PuTTY sessions";
pub const SOURCE_CSV: &str = "CSV";

pub const FILE: &str = "File";
pub const BROWSE: &str = "Choose file…";
pub const FILE_HINT: &str = "Hosts with patterns (Host *) are applied to the hosts they match and are not imported themselves.";
pub const FILE_MISSING: &str = "Choose a file.";
pub const FILE_NOT_FOUND: &str = "The file does not exist.";
pub const READ_FAILED: &str = "The file could not be read.";

pub const PREVIEW_HOSTS: &str = "Hosts";
pub const PREVIEW_KEYS: &str = "Key files";
pub const PREVIEW_PROBLEMS: &str = "Not read";
pub const PREVIEW_EMPTY: &str = "The file names no hosts.";
pub const PREVIEW_EMPTY_PUTTY: &str = "PuTTY has no saved sessions.";
pub const PUTTY_READ_FAILED: &str = "The PuTTY sessions could not be read.";
pub const STATUS_NEW: &str = "new";
pub const STATUS_EXISTS: &str = "already exists";
pub const STATUS_ERROR: &str = "not imported";
pub const KEY_NEW: &str = "new key";
pub const KEY_EXISTS: &str = "already in the Keychain";
pub const KEY_PASSPHRASE: &str = "protected by a passphrase";
pub const KEY_MISSING: &str = "file not found";
pub const PASSPHRASE: &str = "Passphrase";
pub const SAVE_PASSPHRASES: &str = "Save the passphrases in the vault";
pub const TEMPLATES: &str = "patterns applied to the hosts above";

pub const SUMMARY_ADDED: &str = "Added";
pub const SUMMARY_SKIPPED: &str = "Skipped (already imported)";
pub const SUMMARY_ERRORS: &str = "Errors";
pub const SUMMARY_KEYS_ADDED: &str = "Keys added";
pub const SUMMARY_KEYS_EXISTING: &str = "Keys already in the Keychain";
pub const SUMMARY_KEYS_SKIPPED: &str = "Key files left out";
pub const SUMMARY_JUMP_LEFT_OUT: &str = "Hosts without their jump hosts (see their notes)";
pub const SUMMARY_COMMANDS_SKIPPED: &str = "Commands neither run nor imported";
pub const IMPORT_FAILED: &str = "The import failed.";

pub const NEXT: &str = "Next";
pub const IMPORT: &str = "Import";
pub const IMPORTING: &str = "Importing…";
pub const READING: &str = "Reading…";
pub const BACK: &str = "Back";
pub const CANCEL: &str = "Cancel";
pub const CLOSE: &str = "Close";

pub const CSV_FILE_HINT: &str = "Columns such as label, address, port, username, group, tags and notes; the next step maps them.";
pub const CSV_TEMPLATE: &str = "Save template…";
pub const CSV_TEMPLATE_SAVED: &str = "Template saved.";
pub const CSV_TEMPLATE_FAILED: &str = "The template could not be saved.";
pub const CSV_TOO_LARGE: &str = "The file is larger than 16 MB or has more than 100,000 rows.";
pub const CSV_OPEN_QUOTE: &str =
    "A quote in the file is never closed, so its rows cannot be told apart.";
pub const CSV_ENCODING: &str = "Character set";
pub const CSV_DELIMITER: &str = "Delimiter";
pub const CSV_TAB: &str = "Tab";
pub const CSV_HEADER: &str = "First row is a header";
pub const CSV_COLUMNS: &str = "Columns";
pub const CSV_ROLE_IGNORE: &str = "not imported";
pub const CSV_NO_ADDRESS: &str = "Choose the column that holds the address.";
pub const CSV_PASSWORDS: &str = "This file holds passwords in clear text. They are stored encrypted in the vault; delete the file after the import.";
pub const CSV_EMPTY: &str = "The file has no rows.";
pub const CSV_SUMMARY_SKIPPED: &str = "Skipped (already exist)";
pub const CSV_SUMMARY_GROUPS: &str = "Groups added";
pub const CSV_DELETE_FILE: &str =
    "The CSV file still holds the passwords in clear text: delete it.";

/// "N rows in the file".
pub fn csv_rows(count: usize) -> String {
    match count {
        1 => "1 row in the file".to_owned(),
        n => format!("{n} rows in the file"),
    }
}

/// The counts above the CSV preview.
pub fn csv_counts(new: usize, existing: usize, errors: usize, groups: usize) -> String {
    let mut text = format!("{new} new, {existing} already exist, {errors} not imported");
    if groups > 0 {
        text.push_str(&format!("; {groups} new groups"));
    }
    text
}

/// A row's group.
pub fn csv_group(path: &str) -> String {
    format!("Group: {path}")
}

/// Why a row is not imported.
pub fn csv_reason(line: usize, reason: &str) -> String {
    format!("Line {line}: {reason}")
}

/// A row without a label or address.
pub fn csv_line(line: usize) -> String {
    format!("Line {line}")
}

/// Rows the preview leaves out.
pub fn csv_more(count: usize) -> String {
    format!("and {count} more rows")
}

/// "N of M hosts selected".
pub fn selected(chosen: usize, total: usize) -> String {
    format!("{chosen} of {total} new hosts selected")
}

/// "N templates" line of the preview.
pub fn templates(count: usize) -> String {
    match count {
        1 => format!("1 Host line with patterns: {TEMPLATES}"),
        n => format!("{n} Host lines with patterns: {TEMPLATES}"),
    }
}
