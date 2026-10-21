//! English strings of the host and group panels (UI 0: the interface is in
//! English; the final texts live in the i18n catalog `en`, which re-exports
//! this module). The meaning comes from docs/ui-behavior-spec.md, 1.3 and 3.

use shellpp_core::ValidationReason;

// Header (UI 1.3).
pub const HOST_DETAILS: &str = "Host details";
pub const NEW_HOST: &str = "New host";
pub const GROUP_DETAILS: &str = "Group details";
pub const NEW_GROUP: &str = "New group";
pub const MORE_ACTIONS: &str = "More actions";
pub const COLLAPSE: &str = "Collapse panel";

// "…" menu of the host panel (UI 3.3).
pub const MENU_CONNECT: &str = "Connect";
pub const MENU_ADD_TELNET: &str = "Add Telnet";
pub const MENU_DUPLICATE: &str = "Duplicate";
pub const MENU_REMOVE: &str = "Remove";

// Group menu (UI 3.4).
pub const MENU_OPEN: &str = "Open";
pub const MENU_EDIT: &str = "Edit";
pub const MENU_MOVE_TO: &str = "Move to";

// Blocks (UI 3.1, 3.4).
pub const VAULT: &str = "Vault";
pub const PERSONAL_VAULT: &str = "Personal";
pub const ADDRESS: &str = "Address";
pub const ADDRESS_PLACEHOLDER: &str = "IP or hostname";
pub const GENERAL: &str = "General";
pub const LABEL: &str = "Label";
pub const LABEL_PLACEHOLDER: &str = "Same as the address";
pub const NAME: &str = "Name";
pub const NAME_PLACEHOLDER: &str = "Group name";
pub const PARENT_GROUP: &str = "Parent Group";
pub const TOP_LEVEL: &str = "Top level";
pub const SEARCH_GROUPS: &str = "Search groups";
pub const SEARCH_HOSTS: &str = "Search hosts";
pub const NO_MATCHES: &str = "Nothing found";
pub const TAGS: &str = "Tags";
pub const TAGS_PLACEHOLDER: &str = "Add a tag";
pub const COLOR: &str = "Color";
pub const BACKSPACE: &str = "Backspace";
pub const BACKSPACE_DEFAULT: &str = "Default";
pub const BACKSPACE_DEL: &str = "^? (DEL)";
pub const BACKSPACE_CTRL_H: &str = "^H";
pub const NOTES: &str = "Notes";
pub const NOTES_PLACEHOLDER: &str = "Anything worth remembering about this host";
pub const SHARE_HOST: &str = "Share this host";
pub const SSH: &str = "SSH";
pub const PORT: &str = "Port";
pub const DEFAULT_PORT: &str = "22";
pub const USERNAME: &str = "Username";
pub const PASSWORD: &str = "Password";
pub const PASSWORD_SAVED: &str = "Saved password";
pub const SHOW_PASSWORD: &str = "Show password";
pub const HIDE_PASSWORD: &str = "Hide password";
pub const ADD_CREDENTIAL: &str = "Add identity, key or certificate";
pub const CREDENTIAL_IDENTITY: &str = "Identity";
pub const CREDENTIAL_KEY: &str = "Key";
pub const CREDENTIAL_CERTIFICATE: &str = "Certificate";
pub const CREDENTIAL_FIDO2: &str = "FIDO2 key";
pub const LINKED: &str = "Linked";
pub const UNLINK: &str = "Unlink";
pub const REMOVE_CREDENTIAL: &str = "Remove from this host";
pub const MISSING_ITEM: &str = "Missing item";
pub const CHOOSE_IDENTITY: &str = "Saved identities";
pub const NEW_IDENTITY: &str = "New identity";
pub const IDENTITY_LABEL_PLACEHOLDER: &str = "Name, for example the login";
pub const CREATE: &str = "Create";
pub const SAVED_KEYS: &str = "Saved keys";
pub const ADD_A_KEY: &str = "Add a key";
pub const KEY_TEXT_PLACEHOLDER: &str = "Paste a private key (OpenSSH format)";
pub const CHOOSE_FILE: &str = "Choose a file\u{2026}";
pub const KEY_LABEL_PLACEHOLDER: &str = "From the key comment";
pub const KEY_PASSPHRASE_NOTE: &str = "This key is protected by a passphrase.";
pub const PASSPHRASE: &str = "Passphrase";
pub const PASSPHRASE_STORE: &str = "Store in vault";
pub const PASSPHRASE_ASK: &str = "Ask each time";
pub const IMPORT: &str = "Import";
pub const IMPORTING: &str = "Importing\u{2026}";
pub const KEY_ALREADY_SAVED: &str = "This key was already saved; the saved key is linked.";
pub const FILE_NOT_READ: &str = "The file could not be read.";
pub const CERTIFICATE_PLACEHOLDER: &str =
    "Paste an OpenSSH certificate (\u{2026}-cert-v01@openssh.com)";
pub const CERTIFICATE_NEEDS_KEY: &str = "A certificate belongs to a key. Link a key first.";
pub const ATTACH: &str = "Attach";
pub const NO_SNIPPETS: &str = "No snippets yet";
pub const MOSH: &str = "Mosh";
pub const TELNET: &str = "Telnet";
pub const TELNET_DEFAULT_PORT: &str = "23";
pub const REMOVE_TELNET: &str = "Remove Telnet";
pub const CHARSET_DEFAULT: &str = "UTF-8 (default)";

/// "Certificate for deploy-key".
pub fn certificate_for(key: &str) -> String {
    format!("Certificate for {key}")
}

/// "Certificate of deploy-key" (the linked certificate row).
pub fn certificate_of(key: &str) -> String {
    format!("Certificate of {key}")
}
pub const SHOW_MORE: &str = "Show more";
pub const SHOW_LESS: &str = "Show less";
pub const AGENT_FORWARDING: &str = "Agent Forwarding";
pub const LEGACY_ALGORITHMS: &str = "Allow legacy algorithms";
pub const LEGACY_ALGORITHMS_WARNING: &str = "Shell++ also offers SHA-1 key exchange, SHA-1 \
     MACs, ssh-rsa signatures and CBC ciphers to this host, after the modern ones. They are \
     weak: turn this on only for an old server or device that offers nothing else.";
pub const AGENT_FORWARDING_WARNING: &str = "While a session is open, anyone with root on \
     this host can sign in elsewhere with your keys.";
pub const AGENT_FORWARD_SYSTEM: &str = "Also offer the system agent's keys";
pub const AGENT_CONFIRM: &str = "Ask before every signature";
pub const AGENT_CONFIRM_NOTE: &str = "Each signature waits for your Allow in the session's tab.";
pub const STARTUP_SNIPPET: &str = "Startup snippet";
pub const NONE: &str = "None";
pub const HOST_CHAINING: &str = "Host Chaining";
pub const ADD_JUMP_HOST: &str = "Add jump host";
pub const DIRECT: &str = "Direct connection";
pub const PROXY: &str = "Proxy";
pub const PROXY_NONE: &str = "None";
pub const PROXY_HTTP: &str = "HTTP";
pub const PROXY_SOCKS5: &str = "SOCKS5";
pub const PROXY_ADDRESS: &str = "Proxy address";
pub const PROXY_PORT: &str = "Proxy port";
pub const PROXY_USERNAME: &str = "Proxy username";
pub const PROXY_PASSWORD: &str = "Proxy password";
pub const ENVIRONMENT: &str = "Environment Variables";
pub const ENV_NAME: &str = "NAME";
pub const ENV_VALUE: &str = "value";
pub const ADD_VARIABLE: &str = "Add variable";
pub const REMOVE_VARIABLE: &str = "Remove variable";
pub const CHARSET: &str = "Charset";
pub const CHARSET_PLACEHOLDER: &str = "utf-8";
pub const KEEPALIVE: &str = "Keepalive";
pub const KEEPALIVE_PLACEHOLDER: &str = "As in settings (seconds)";
pub const RECORD_HISTORY: &str = "Record history";
pub const RECORD_OUTPUT: &str = "Record output";
pub const AS_IN_SETTINGS: &str = "As in settings";
pub const ON: &str = "On";
pub const OFF: &str = "Off";
pub const TERMINAL_THEME: &str = "Terminal theme";
pub const DEFAULT_THEME_NAME: &str = "Graphite";
pub const ADD_TELNET: &str = "Add Telnet";
pub const CONNECT: &str = "Connect";
pub const REMOVE_TAG: &str = "Remove tag";

/// "Create group "Staging"" (Parent Group, create on the fly).
pub fn create_group(name: &str) -> String {
    format!("Create group \u{201c}{name}\u{201d}")
}

/// "Create tag "prod"".
pub fn create_tag(name: &str) -> String {
    format!("Create tag \u{201c}{name}\u{201d}")
}

/// The inherited-value placeholder (UI 3.1, 3.4; data-model 8.3):
/// "2222 · from group Production".
pub fn inherited(value: &str, group: &str) -> String {
    format!("{value} \u{b7} from group {group}")
}

/// The same without a value to show (a linked identity or key).
pub fn from_group(group: &str) -> String {
    format!("from group {group}")
}

// Placeholders of targets that are not built yet (lead decision 5).
pub const IN_DEVELOPMENT: &str = "In development";

// Removal (UI 2.4, 3.4).
pub const REMOVE_HOST_TITLE: &str = "Remove host?";
pub const REMOVE_HOST_BODY: &str = "The host is removed from this vault.";
pub const REMOVE_GROUP_TITLE: &str = "Remove group?";
pub const REMOVE_GROUP_EMPTY_BODY: &str = "The group is empty and is removed.";
pub const REMOVE_GROUP_NON_EMPTY_BODY: &str =
    "Its subgroups and hosts move one level up; then the group is removed.";
pub const CANCEL: &str = "Cancel";
pub const REMOVED_ONE: &str = "Removed";
pub const DUPLICATED: &str = "Duplicated";
pub const NOT_SAVED: &str = "Something went wrong. Try again.";
pub const GROUP_CREATED: &str = "Group created";

/// Why a value was refused, shown under the field (UI 1.3).
pub fn reason(field: &str, reason: ValidationReason) -> &'static str {
    match (field, reason) {
        ("address" | "ssh.proxy.address", ValidationReason::Required) => {
            "Enter an IP address or a host name."
        }
        ("address" | "ssh.proxy.address", _) => {
            "Use an IP address or a host name without spaces, @, / or brackets."
        }
        ("name", ValidationReason::Required) => "A group needs a name.",
        ("label", ValidationReason::Required) => "Enter a name.",
        ("private_key", ValidationReason::Required) => "Paste a key or choose a file.",
        ("private_key", ValidationReason::NotAllowed) => "This key type or size is not supported.",
        ("private_key", _) => "Use a private key in the OpenSSH format.",
        ("passphrase", ValidationReason::Required) => "Enter the passphrase of this key.",
        ("passphrase", _) => "The passphrase does not open this key.",
        ("certificate", ValidationReason::Required) => "Paste a certificate.",
        ("certificate", ValidationReason::NotAllowed) => {
            "This is not a user certificate for the linked key."
        }
        ("certificate", _) => "Paste one OpenSSH certificate line.",
        ("ssh.port" | "ssh.proxy.port" | "telnet.port", ValidationReason::Required) => {
            "Enter a port."
        }
        ("ssh.port" | "ssh.proxy.port" | "telnet.port", _) => "Use a port from 1 to 65535.",
        ("ssh.keepalive_s", _) => "Use 0 to 3600 seconds; 0 turns keepalive off.",
        ("charset", _) => "Use an encoding label such as utf-8.",
        (_, ValidationReason::Required) => "This field is required.",
        (_, ValidationReason::TooLong) => "This value is too long.",
        (_, ValidationReason::TooShort) => "This value is too short.",
        (_, ValidationReason::OutOfRange) => "This number is out of range.",
        (_, ValidationReason::Duplicate) => "This name is already used.",
        (_, ValidationReason::TooMany) => "There are too many items.",
        (_, ValidationReason::Cycle) => "A group cannot go inside itself.",
        (_, ValidationReason::TooDeep) => "Groups cannot be nested this deep.",
        (_, ValidationReason::MissingRef) => "The linked item no longer exists.",
        (_, ValidationReason::NotAllowed | ValidationReason::Invalid) => "This value is not valid.",
    }
}

/// The identity hint of the SSH block (UI 3.1, stage 3).
pub const IDENTITY_HINT_CREATE: &str = "Create an identity";
pub const IDENTITY_HINT_FAILED: &str = "The identity could not be created.";

/// "This username and password are also used in N other hosts."
pub fn identity_hint_password(others: usize) -> String {
    match others {
        1 => "This username and password are also used in 1 other host. One identity can hold them for all of them.".to_owned(),
        n => format!("This username and password are also used in {n} other hosts. One identity can hold them for all of them."),
    }
}

/// "This username and key are also used in N other hosts."
pub fn identity_hint_key(others: usize) -> String {
    match others {
        1 => "This username and key are also used in 1 other host. One identity can hold them for all of them.".to_owned(),
        n => format!("This username and key are also used in {n} other hosts. One identity can hold them for all of them."),
    }
}

/// The toast after "Create an identity".
pub fn identity_created(hosts: usize) -> String {
    format!("Identity created for {hosts} hosts.")
}
