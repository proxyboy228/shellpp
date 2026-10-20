//! Interface strings of Export Data and Import Data (en; D38).

pub const EXPORT_TITLE: &str = "Export Data";
pub const EXPORT_ENCRYPT: &str = "Encrypt with a password";
pub const EXPORT_ENCRYPT_HINT: &str =
    "Recommended. Without the password nobody can read the file, and nobody can recover it.";
pub const EXPORT_PASSWORD: &str = "Password";
pub const EXPORT_REPEAT: &str = "Repeat password";
pub const EXPORT_PLAIN_WARNING: &str = "The file will hold your passwords and private keys in clear text. Anyone who gets the file can use them.";
pub const EXPORT_UNDERSTAND: &str = "I understand";
pub const EXPORT_RUN: &str = "Export…";
pub const EXPORT_RUNNING: &str = "Exporting…";
pub const EXPORT_FAILED: &str = "The data could not be exported.";
pub const EXPORT_WRITE_FAILED: &str = "The file could not be written.";

pub const PASSWORD_TOO_SHORT: &str = "At least 8 characters.";
pub const PASSWORD_MISMATCH: &str = "The passwords do not match.";

pub const IMPORT_TITLE: &str = "Import Data";
pub const IMPORT_FILE: &str = "File";
pub const IMPORT_BROWSE: &str = "Choose file…";
pub const IMPORT_HINT: &str = "A Shell++ export (.json or .encrypted.json). Records that are here already are kept; a record changed on both sides is imported as a copy.";
pub const IMPORT_SETTINGS: &str = "Apply the settings in the file";
pub const IMPORT_NEXT: &str = "Import";
pub const IMPORT_RUNNING: &str = "Importing…";
pub const IMPORT_PASSWORD: &str = "Export password";
pub const IMPORT_PASSWORD_HINT: &str =
    "The file is encrypted. Enter the password chosen at export.";
pub const FILE_MISSING: &str = "Choose a file.";
pub const FILE_NOT_FOUND: &str = "The file does not exist.";
pub const FILE_UNREADABLE: &str = "The file cannot be read.";
pub const FILE_TOO_LARGE: &str = "The file is larger than 64 MB.";
pub const FILE_NOT_EXPORT: &str = "This is not a Shell++ export.";
pub const PASSWORD_WRONG: &str = "Wrong password.";
pub const IMPORT_FAILED: &str = "The data could not be imported.";

pub const SUMMARY_TITLE: &str = "Import Finished";
pub const SUMMARY_ADDED: &str = "Added";
pub const SUMMARY_EXISTING: &str = "Already here";
pub const SUMMARY_CONFLICTS: &str = "Imported as copies";
pub const SUMMARY_SKIPPED: &str = "Skipped (history, logs, device data)";
pub const SUMMARY_ERRORS: &str = "Errors";
pub const SUMMARY_SETTINGS: &str = "Settings applied";
pub const SUMMARY_SETTINGS_LEFT_OUT: &str =
    "Settings kept as they are here (programs, scripts, security)";

pub const CANCEL: &str = "Cancel";
pub const CLOSE: &str = "Close";

/// The toast after an export.
pub fn exported(records: usize) -> String {
    match records {
        1 => "Exported 1 record.".to_owned(),
        n => format!("Exported {n} records."),
    }
}
