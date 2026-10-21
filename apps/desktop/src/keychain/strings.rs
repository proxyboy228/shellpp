//! English strings of the Keychain screen (UI 0: the interface is in
//! English; the meaning comes from docs/ui-behavior-spec.md, section 7).

use shellpp_core::ValidationReason;
use shellpp_core::keys::convert::KeyProblem;

// Action bar (7.1).
pub const NEW_KEY: &str = "New key";
pub const NEW_KEY_MENU: &str = "More ways to add";
pub const GENERATE_KEY: &str = "Generate key";
pub const NEW_IDENTITY: &str = "New identity";
pub const CERTIFICATE: &str = "Certificate";
pub const FIDO2: &str = "FIDO2";
pub const SEARCH_PLACEHOLDER: &str = "Search keys and identities";
pub const VIEW_TILES: &str = "Tiles";
pub const VIEW_LIST: &str = "List";
pub const SORT: &str = "Sort";
pub const SORT_NAME: &str = "By name";
pub const SORT_CREATED: &str = "By creation date";
pub const SORT_MODIFIED: &str = "By modification date";

// Content (7.2).
pub const KEYS: &str = "Keys";
pub const IDENTITIES: &str = "Identities";
pub const NO_KEYS: &str = "No keys yet";
/// The subtitle of a key card: "Type" and the type.
pub const TYPE: &str = "Type";
/// The type of a key this client cannot read (7.2).
pub const TYPE_UNKNOWN: &str = "unknown";
pub const METHOD_PASSWORD: &str = "Password";
pub const METHOD_KEY: &str = "Key";
pub const METHOD_BOTH: &str = "Password and key";
pub const METHOD_NONE: &str = "No authentication method";
pub const COLUMN_NAME: &str = "Name";
pub const COLUMN_TYPE: &str = "Type";
pub const COLUMN_FINGERPRINT: &str = "SHA256 fingerprint";
pub const COLUMN_USERNAME: &str = "Username";
pub const COLUMN_METHOD: &str = "Authentication";

// Empty states.
pub const EMPTY_TITLE: &str = "No keys yet";
pub const EMPTY_BODY: &str = "Generate a new key pair, or import a key you already use: \
     OpenSSH, PEM and PuTTY (.ppk) files are read.";
pub const EMPTY_GENERATE: &str = "Generate";
pub const EMPTY_IMPORT: &str = "Import";
pub const EMPTY_IMPORT_SSH: &str = "Import from SSH folder";
/// The folder Import from SSH folder reads, in its tooltip.
pub const SSH_FOLDER_PATH: &str = "~/.ssh";
pub const NO_MATCHES: &str = "No keys or identities match the search";

// Key panel (7.3).
pub const PANEL_TITLE: &str = "Key details";
pub const FIELD_VAULT: &str = "Vault";
pub const FIELD_LABEL: &str = "Label";
pub const FIELD_PRIVATE_KEY: &str = "Private key";
pub const FIELD_PUBLIC_KEY: &str = "Public key";
pub const FIELD_FINGERPRINT: &str = "SHA256 fingerprint";
pub const FIELD_TYPE: &str = "Type";
pub const HIDDEN_NOTE: &str = "The private key is hidden.";
pub const SHOW: &str = "Show";
pub const HIDE: &str = "Hide";
pub const COPY_PRIVATE_KEY: &str = "Copy private key";
pub const PRIVATE_KEY_COPIED: &str = "Private key copied. The clipboard is cleared in 30 seconds.";
pub const PRIVATE_KEY_NOT_COPIED: &str = "The private key could not be copied";
pub const PRIVATE_KEY_NOT_SHOWN: &str = "The private key could not be read";
pub const COPY_PUBLIC_KEY: &str = "Copy public key";
pub const PUBLIC_KEY_COPIED: &str = "Public key copied";
pub const COPY_FINGERPRINT: &str = "Copy fingerprint";
pub const FINGERPRINT_COPIED: &str = "Fingerprint copied";
pub const PUBLIC_KEY_MISSING: &str = "The public key could not be read from this key.";
pub const NOT_PROTECTED: &str = "Not protected by a passphrase";
pub const PASSPHRASE_SAVED: &str = "Protected by a passphrase, saved in the vault";
pub const PASSPHRASE_ASKED: &str = "Protected by a passphrase, asked on every connection";
pub const UNKNOWN_REASON: &str = "This client cannot read this key: its type is not supported \
     or the record is damaged. It is kept as it is.";
pub const EXPORT_TITLE: &str = "Key export";
pub const EXPORT_HELP: &str = "Adds the public key to ~/.ssh/authorized_keys on the hosts you \
     choose, without duplicates.";
pub const EXPORT_TO_HOST: &str = "Export to host…";
pub const EXPORT_DIALOG_TITLE: &str = "Export to host";
pub const EXPORT_SELECT_ALL: &str = "Select all";

/// "3 selected".
pub fn export_selected(count: usize) -> String {
    format!("{count} selected")
}
pub const EXPORT_HOSTS_HINT: &str = "Hosts sign in with their saved credentials; a host whose key \
     is not in Known Hosts yet is left as it is.";
pub const EXPORT_SEARCH: &str = "Search hosts";
pub const EXPORT_NO_HOSTS: &str = "No saved host matches.";
pub const EXPORT_RUN: &str = "Export";
pub const EXPORT_CANCEL: &str = "Cancel";
pub const EXPORT_CLOSE: &str = "Close";
pub const EXPORT_RUNNING: &str = "Exporting…";
pub const EXPORT_ADDED: &str = "Added";
pub const EXPORT_ALREADY: &str = "Already there";
pub const EXPORT_UNKNOWN_KEY: &str = "Connect once to save its host key first";
pub const EXPORT_NO_CREDENTIALS: &str = "No saved credentials signed in";
pub const MENU_DUPLICATE: &str = "Duplicate";
pub const MENU_REMOVE: &str = "Remove";
pub const REMOVE_TITLE: &str = "Remove key?";
pub const REMOVE: &str = "Remove";
pub const REMOVE_UNUSED: &str = "No host or identity uses it.";
pub const REMOVE_USED: &str = "These will lose the key and connect with their other sign-in methods until another key is set:";
pub const REMOVE_HOSTS: &str = "Hosts";
pub const REMOVE_GROUPS: &str = "Groups";
pub const REMOVE_IDENTITIES: &str = "Identities";
pub const KEY_REMOVED: &str = "Key removed";
pub const REMOVE_FAILED: &str = "The key could not be removed";
pub const UNDO_FAILED: &str = "The key could not be restored";
pub const MENU_EXPORT_FILE: &str = "Export to file…";
pub const LABEL_NOT_SAVED: &str = "The label could not be saved";

// Generate key (7.1).
pub const GENERATE_TITLE: &str = "Generate key";
pub const FIELD_KEY_TYPE: &str = "Key type";
pub const FIELD_CURVE: &str = "Curve";
pub const FIELD_KEY_SIZE: &str = "Key size";
pub const ED25519: &str = "Ed25519";
pub const ECDSA: &str = "ECDSA";
pub const RSA: &str = "RSA";
pub const P256: &str = "P-256";
pub const P384: &str = "P-384";
pub const P521: &str = "P-521";
pub const BITS_3072: &str = "3072 bits";
pub const BITS_4096: &str = "4096 bits";
pub const LABEL_PLACEHOLDER: &str = "For example, work laptop";
pub const FIELD_PASSPHRASE: &str = "Passphrase";
pub const PASSPHRASE_OPTIONAL: &str = "Optional";
pub const FIELD_PASSPHRASE_REPEAT: &str = "Repeat passphrase";
pub const PASSPHRASE_REPEAT_PLACEHOLDER: &str = "The same passphrase again";
pub const PASSPHRASE_STORE: &str = "Save in the vault";
pub const PASSPHRASE_ASK: &str = "Ask on every connection";
pub const GENERATE: &str = "Generate";
pub const GENERATING: &str = "Generating…";
pub const GENERATING_RSA: &str = "Generating an RSA key takes a few seconds.";
pub const GENERATE_FAILED: &str = "The key could not be generated";
pub const CANCEL: &str = "Cancel";

/// The first line of the Remove confirmation.
pub fn remove_body(label: &str) -> String {
    format!("Remove the key \u{201c}{label}\u{201d}? Undo is offered for a few seconds.")
}

// Import (New key, 7.1, 7.3).
pub const IMPORT_TITLE: &str = "New key";
pub const IMPORT_HINT: &str = "Paste a private key, choose a file or drop one here.";
pub const IMPORT_SECURITY_KEY_TITLE: &str = "FIDO2 key";
pub const IMPORT_SECURITY_KEY_HINT: &str = "Paste or choose the private key file that \
     ssh-keygen -t ed25519-sk (or ecdsa-sk) made. It holds only a handle: the key stays on \
     the security key.";
/// What Certificate does (stage 4, item 9).
pub const CERTIFICATE_TOOLTIP: &str = "Add an OpenSSH user certificate to the key it certifies";
pub const CERTIFICATE_TITLE: &str = "Certificate";
pub const CERTIFICATE_DIALOG_TITLE: &str = "Add a certificate";
pub const CERTIFICATE_HINT: &str = "Paste the certificate (the key-cert.pub file) or choose the \
     file. It goes onto the key in the Keychain whose public key it certifies.";
pub const CERTIFICATE_ADD: &str = "Add";
pub const CERTIFICATE_KEY_ID: &str = "Key ID";
pub const CERTIFICATE_PRINCIPALS: &str = "Principals";
pub const CERTIFICATE_ANY_PRINCIPAL: &str = "Any user";
pub const CERTIFICATE_VALID: &str = "Valid";
pub const CERTIFICATE_FOREVER: &str = "forever";
pub const CERTIFICATE_CA: &str = "CA fingerprint";
pub const CERTIFICATE_EXPIRED: &str = "This certificate has expired: servers refuse it.";
pub const CERTIFICATE_NOT_YET: &str = "This certificate is not valid yet.";
pub const CERTIFICATE_REMOVE: &str = "Remove certificate";
pub const CERTIFICATE_REMOVE_FAILED: &str = "The certificate could not be removed";
pub const CERTIFICATE_NO_KEY: &str =
    "No key in the Keychain matches this certificate: import its private key first.";
pub const CERTIFICATE_INVALID: &str = "This is not an OpenSSH user certificate.";
pub const FIDO2_TOOLTIP: &str = "Import a key made on a FIDO2 security key. Creating one here \
     and signing in with it arrive in a later stage.";
pub const IMPORT_TEXT_PLACEHOLDER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
pub const CHOOSE_FILE: &str = "Choose file…";
pub const IMPORT_LABEL_PLACEHOLDER: &str = "From the key comment";
pub const FORMAT: &str = "Format";
pub const CHECKING: &str = "Checking…";
pub const NEEDS_PASSPHRASE: &str = "Protected by a passphrase: enter it to import the key.";
pub const IMPORT: &str = "Import";
pub const IMPORTING: &str = "Importing…";
pub const IMPORT_FAILED: &str = "The key could not be imported";
pub const ALREADY_SAVED: &str = "This key is already in the Keychain";
pub const SSH_FOLDER: &str = "Keys in ~/.ssh";
pub const SSH_FOLDER_EMPTY: &str = "No private keys were found in ~/.ssh.";
pub const SSH_FOLDER_MISSING: &str = "The ~/.ssh folder was not found.";
pub const FILE_NOT_READ: &str = "The file could not be read.";
pub const FILE_TOO_LARGE: &str = "The file is too large for a key.";

/// The format of a key the import form recognized.
pub fn format_text(format: &str, encrypted: bool) -> String {
    if encrypted {
        format!("{format}, protected by a passphrase")
    } else {
        format.to_owned()
    }
}

/// Why a text cannot be imported (the type "unknown", UI 7.2): the
/// meaning of the core's [`KeyProblem`].
pub fn problem(problem: &KeyProblem) -> String {
    match problem {
        KeyProblem::NotAKey => "This is not a private key in a supported format \
             (OpenSSH, PEM or PuTTY .ppk)."
            .to_owned(),
        KeyProblem::PublicKey => {
            "This is a public key. Import the private key: the file of the same name \
             without .pub."
                .to_owned()
        }
        KeyProblem::Damaged => "The key is damaged or incomplete.".to_owned(),
        KeyProblem::Algorithm(algorithm) if algorithm.is_empty() => {
            "This key type is not supported.".to_owned()
        }
        KeyProblem::Algorithm(algorithm) => {
            format!("The key type {algorithm} is not supported.")
        }
        KeyProblem::RsaTooSmall => "RSA keys under 2048 bits are not accepted.".to_owned(),
        KeyProblem::Cipher(cipher) => {
            format!("The key's encryption {cipher} is not supported.")
        }
        KeyProblem::KdfTooExpensive => {
            "The key's passphrase protection asks for more work than is allowed.".to_owned()
        }
        KeyProblem::PassphraseNeeded => "The key is protected by a passphrase.".to_owned(),
        KeyProblem::WrongPassphrase => "The passphrase does not open this key.".to_owned(),
    }
}

/// The reason under an invalid field of the Keychain's forms.
pub fn reason(field: &str, reason: ValidationReason) -> &'static str {
    match (field, reason) {
        ("label", ValidationReason::Required) => "Enter a label.",
        ("label", ValidationReason::TooLong) => "Use at most 256 characters.",
        ("private_key", ValidationReason::Required) => "Paste a key, choose a file or drop one.",
        ("private_key", ValidationReason::TooLong) => "This text is too long for a key.",
        ("private_key", ValidationReason::NotAllowed) => {
            "This key type, size or encryption is not supported."
        }
        ("private_key", _) => "This is not a private key in a supported format.",
        ("passphrase", ValidationReason::Required) => "Enter the passphrase of this key.",
        ("passphrase", ValidationReason::TooLong) => "Use at most 1024 bytes.",
        ("passphrase", ValidationReason::NotAllowed) => "This key has no passphrase.",
        ("passphrase", _) => "The passphrase does not open this key.",
        ("passphrase_repeat", _) => "The passphrases do not match.",
        _ => crate::panels::strings::reason(field, reason),
    }
}
