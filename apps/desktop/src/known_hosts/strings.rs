//! English strings of the Known Hosts screen (UI 0: the interface is in
//! English; the meaning comes from docs/ui-behavior-spec.md, section 10.1).

pub const TITLE: &str = "Known Hosts";

// Action bar.
pub const IMPORT: &str = "Import";
pub const EXPORT: &str = "Export";
pub const SEARCH_PLACEHOLDER: &str = "Search known hosts";
pub const VIEW_TILES: &str = "Tiles";
pub const VIEW_LIST: &str = "List";
pub const SORT: &str = "Sort";
pub const SORT_ADDRESS: &str = "By address";
pub const SORT_ADDED: &str = "By date added";
pub const SORT_KEY_TYPE: &str = "By key type";

// Cards and rows.
/// The title of a hashed entry's card instead of its address (10.1).
pub const HASHED_HOST: &str = "Hashed host";
/// The badge of a revoked key (10.1).
pub const REVOKED: &str = "revoked";
pub const COLUMN_ADDRESS: &str = "Address";
pub const COLUMN_KEY_TYPE: &str = "Key type";
pub const COLUMN_FINGERPRINT: &str = "Fingerprint";
pub const COLUMN_ADDED: &str = "Date added";

// Empty states.
pub const EMPTY_TITLE: &str = "No known hosts";
pub const EMPTY_BODY: &str = "A host's key is saved here when you trust it on the first connection. \
     You can also import a known_hosts file.";
pub const EMPTY_IMPORT: &str = "Import known_hosts";
pub const NO_MATCHES: &str = "No known hosts match the search";

// Details panel.
pub const PANEL_TITLE: &str = "Known host";
pub const FIELD_ADDRESS: &str = "Address";
pub const FIELD_PORT: &str = "Port";
pub const FIELD_KEY_TYPE: &str = "Key type";
pub const FIELD_FINGERPRINT: &str = "SHA256 fingerprint";
pub const FIELD_ADDED: &str = "Date added";
pub const FIELD_COMMENT: &str = "Comment";
pub const FIELD_HOSTS: &str = "Saved hosts with this address";
pub const NO_SAVED_HOSTS: &str = "None";
/// The port of a hashed entry: it is part of the hash.
pub const HASHED_PORT: &str = "In the hash";
pub const HASHED_NOTE: &str =
    "The address is stored as a hash. It is checked on every connection but cannot be shown.";
pub const REVOKED_NOTE: &str = "This key is revoked: every connection that presents it is blocked.";
pub const COPY_FINGERPRINT: &str = "Copy fingerprint";
pub const FINGERPRINT_COPIED: &str = "Fingerprint copied";
pub const REMOVE: &str = "Remove";

// Removal (10.1: confirmation and Undo as for hosts).
pub const REMOVE_TITLE: &str = "Remove known host?";
pub const CANCEL: &str = "Cancel";
pub const REMOVED: &str = "Known host removed";
pub const REMOVE_FAILED: &str = "The known host could not be removed";
pub const UNDO_FAILED: &str = "The removal can no longer be undone";

/// The body of the removal confirmation for an entry shown as `title`.
pub fn remove_body(title: &str, revoked: bool) -> String {
    if revoked {
        format!("{title}: the revoked key will no longer be blocked.")
    } else {
        format!("The next connection to {title} will ask again whether to trust its key.")
    }
}

// Import dialog (2.5 with the known_hosts source).
pub const IMPORT_TITLE: &str = "Import known_hosts";
pub const IMPORT_FILE: &str = "File";
pub const IMPORT_BROWSE: &str = "Choose file";
pub const IMPORT_RUN: &str = "Import";
pub const IMPORT_RUNNING: &str = "Importing…";
pub const IMPORT_HINT: &str = "Hashed entries are kept as they are and still checked on connect. \
     @revoked entries become revoked keys; @cert-authority lines and negated (!) patterns \
     are skipped.";
pub const SUMMARY_TITLE: &str = "Import finished";
pub const SUMMARY_IMPORTED: &str = "Imported";
pub const SUMMARY_DUPLICATES: &str = "Already in the list";
pub const SUMMARY_SKIPPED: &str = "Skipped (@cert-authority, negated patterns)";
pub const SUMMARY_INVALID: &str = "Invalid lines";
pub const CLOSE: &str = "Close";
pub const FILE_MISSING: &str = "Choose a file.";
pub const FILE_NOT_FOUND: &str = "The file was not found.";
pub const FILE_UNREADABLE: &str = "The file could not be read.";
pub const FILE_TOO_LARGE: &str = "The file is larger than 16 MiB.";
pub const IMPORT_FAILED: &str = "The import failed.";

// Export (storage-format 13.6).
pub const EXPORT_FILE_NAME: &str = "shellpp-known_hosts";
pub const EXPORTED: &str = "Known hosts exported";
pub const EXPORT_FAILED: &str = "Known hosts could not be exported";
