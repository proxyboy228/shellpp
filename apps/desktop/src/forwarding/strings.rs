//! The texts of the Port Forwarding screen (UI 8).

// Action bar (8.1).
pub const NEW_RULE: &str = "New forwarding";
pub const NEW_RULE_MENU: &str = "New forwarding of a type";
pub const NEW_LOCAL: &str = "Local forwarding";
pub const NEW_REMOTE: &str = "Remote forwarding";
pub const NEW_DYNAMIC: &str = "Dynamic (SOCKS) forwarding";
pub const SEARCH_PLACEHOLDER: &str = "Search rules";

// Cards (8.2).
pub const STOPPED: &str = "Stopped";
pub const STARTING: &str = "Starting";
pub const RUNNING: &str = "Running";
pub const EMPTY_TITLE: &str = "No forwarding rules";
pub const EMPTY_BODY: &str =
    "A rule opens a port here or on a server and carries its connections through SSH.";
pub const NO_MATCHES: &str = "No rules match the search";
pub const NO_HOSTS: &str = "Add a host first: a rule goes through one of your SSH hosts.";

/// "Reconnecting (attempt 2)".
pub fn reconnecting(attempt: u32) -> String {
    format!("Reconnecting (attempt {attempt})")
}

/// "Port 8080 is busy. Choose another port." (UI 8.4.)
pub fn port_busy(port: u16) -> String {
    format!("Port {port} is busy. Choose another port.")
}

/// "Port 80 needs administrator rights. Choose a port above 1023."
pub fn port_denied(port: u16) -> String {
    format!("Port {port} needs administrator rights. Choose a port above 1023.")
}

pub const ADDRESS_UNAVAILABLE: &str = "This computer has no such address.";
pub const REFUSED: &str =
    "The server refused to listen on that port: it is in use or forwarding is off there.";
pub const DISCONNECTED: &str = "The connection is closed.";
pub const TIMEOUT: &str = "The server did not answer in time.";

// Menu (8.2).
pub const START: &str = "Start";
pub const STOP: &str = "Stop";
pub const EDIT: &str = "Edit";
pub const DUPLICATE: &str = "Duplicate";
pub const REMOVE: &str = "Remove";
pub const CANCEL: &str = "Cancel";
pub const REMOVE_TITLE: &str = "Remove forwarding rule?";
pub const REMOVED: &str = "Forwarding rule removed";
pub const REMOVE_FAILED: &str = "The rule could not be removed";
pub const UNDO_FAILED: &str = "The rule could not be brought back";
pub const SAVE_FAILED: &str = "The rule could not be saved";
pub const COPY_SUFFIX: &str = "Copy";

/// "Remove "Database"? Its port closes now."
pub fn remove_body(title: &str, running: bool) -> String {
    if running {
        format!("Remove \"{title}\"? Its port closes now.")
    } else {
        format!("Remove \"{title}\"?")
    }
}

// Form (8.4).
pub const PANEL_NEW: &str = "New forwarding";
pub const PANEL_EDIT: &str = "Forwarding rule";
pub const TYPE: &str = "Type";
pub const TYPE_LOCAL: &str = "Local";
pub const TYPE_REMOTE: &str = "Remote";
pub const TYPE_DYNAMIC: &str = "Dynamic";
pub const EXPLAIN_LOCAL: &str =
    "A port on this computer leads to a service that the SSH host can reach.";
pub const EXPLAIN_REMOTE: &str =
    "A port on the SSH host leads back to a service that this computer can reach.";
pub const EXPLAIN_DYNAMIC: &str =
    "A SOCKS proxy on this computer: every connection through it goes via the SSH host.";
pub const HOST: &str = "Via host";
pub const HOST_NONE: &str = "Choose a host";
pub const LABEL: &str = "Label";
pub const LABEL_PLACEHOLDER: &str = "Database";
pub const LISTEN_HERE: &str = "Listen on this computer";
pub const LISTEN_THERE: &str = "Listen on the SSH host";
pub const SOCKS_HERE: &str = "SOCKS proxy on this computer";
pub const DEST_THERE: &str = "Destination, as the SSH host sees it";
pub const DEST_HERE: &str = "Destination, as this computer sees it";
pub const ADDRESS: &str = "Address";
pub const PORT: &str = "Port";
pub const AUTOSTART: &str = "Start automatically";
pub const ON_CONNECT: &str = "When I connect to the host";
pub const ON_LAUNCH: &str = "When the app starts";
pub const ON_LAUNCH_NOTE: &str = "While the app is locked, the rule waits for unlock.";
pub const WIDE_LOCAL: &str =
    "The port opens to every network this computer is on, not only to this computer.";
pub const WIDE_REMOTE: &str = "The port opens to every network of the SSH host. The server allows it only with \
     GatewayPorts yes or clientspecified in its sshd_config.";
pub const CREATE: &str = "Create";
pub const CREATE_AND_START: &str = "Create and start";
pub const SAVE: &str = "Save";

// Field errors.
pub const ERROR_HOST: &str = "Choose the host the rule goes through.";
pub const ERROR_PORT: &str = "Enter a port from 1 to 65535.";
pub const ERROR_ADDRESS: &str = "Enter an IP address or localhost.";
pub const ERROR_DEST: &str = "Enter the destination address.";
pub const ERROR_TOO_LONG: &str = "This is too long.";
pub const ERROR_INVALID: &str = "This value is not valid.";
