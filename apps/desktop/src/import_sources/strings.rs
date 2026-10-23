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
pub const SOURCE_LATER: &str = "Arrives in a later version.";

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
