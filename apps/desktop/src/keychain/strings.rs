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
pub const EMPTY_IMPORT_SSH: &str = "Import from ~/.ssh";
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
pub const MENU_DUPLICATE: &str = "Duplicate";
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
pub const PASSPHRASE_STORE: &str = "Save in the vault";
pub const PASSPHRASE_ASK: &str = "Ask on every connection";
pub const GENERATE: &str = "Generate";
pub const GENERATING: &str = "Generating…";
pub const GENERATING_RSA: &str = "Generating an RSA key takes a few seconds.";
pub const GENERATE_FAILED: &str = "The key could not be generated";
pub const CANCEL: &str = "Cancel";

// Import (New key, 7.1, 7.3).
pub const IMPORT_TITLE: &str = "New key";
pub const IMPORT_HINT: &str = "Paste a private key, choose a file or drop one here.";
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
        _ => crate::panels::strings::reason(field, reason),
    }
}
