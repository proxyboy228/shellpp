//! Strings of session tabs (UI 4.1, 4.3, 4.6, 4.8, 10.2), catalog `en`.

// Connection screen (UI 4.8).
pub const STATUS_CONNECTING: &str = "Connecting";
pub const STATUS_FAILED: &str = "Could not connect";
pub const STEP_RESOLVING: &str = "Resolving the address";
pub const STEP_CONNECTING: &str = "Connecting to the server";
pub const STEP_HOST_KEY: &str = "Checking the host key";
pub const STEP_AUTHENTICATING: &str = "Authenticating";
pub const STEP_OPENING_SESSION: &str = "Opening the session";
pub const STEP_LOCAL: &str = "Starting the shell";
pub const CANCEL: &str = "Cancel";
pub const RETRY: &str = "Retry";
pub const CLOSE: &str = "Close";

// Reasons (UI 4.6, 4.8).
pub const REASON_NOT_FOUND: &str =
    "The address was not found. Check the host name and the network connection.";
pub const REASON_REFUSED: &str =
    "The connection was refused. Check the port and that the SSH server is running.";
pub const REASON_TIMEOUT: &str = "The connection timed out.";
pub const REASON_NETWORK: &str = "A network error occurred. Check the network connection.";
pub const REASON_HOST_KEY_REJECTED: &str = "The host key was not accepted.";
pub const REASON_HOST_KEY_REVOKED: &str =
    "The host key of this server is marked as revoked. The connection is blocked.";
pub const REASON_AUTH_FAILED: &str =
    "Authentication failed. The server did not accept the username, password or key.";
pub const REASON_CANCELLED: &str = "The connection was cancelled.";
pub const REASON_PROTOCOL: &str = "The server and Shell++ could not agree on the SSH protocol.";
pub const REASON_CHANNEL_FAILED: &str = "The server refused to open the session.";
pub const REASON_DISCONNECTED: &str = "The server closed the connection.";
pub const REASON_CONNECTION_LOST: &str = "The connection was lost. Check the network connection.";
pub const REASON_KEEPALIVE: &str = "The server stopped answering (keepalive timeout).";
pub const REASON_CLOSED_BY_SERVER: &str = "The server closed the session without an exit code.";
pub const REASON_KEY_UNUSABLE: &str = "The private key cannot be used.";
pub const REASON_PROXY_FAILED: &str = "The proxy did not open the connection.";
pub const REASON_PROXY_AUTH_FAILED: &str = "The proxy did not accept the username or password.";
pub const REASON_FORWARDING_REFUSED: &str =
    "A jump host does not allow connections to the next host.";
pub const REASON_LOCKED: &str = "Shell++ is locked. Unlock it and try again.";
pub const REASON_HOST_MISSING: &str = "The host no longer exists.";
pub const REASON_INVALID_TARGET: &str = "The address, port or username is not valid.";
pub const REASON_JUMP_HOSTS: &str = "This host connects through jump hosts, which Shell++ does not support yet. It is not connected directly.";
pub const REASON_LOCAL_SHELL: &str = "The local shell could not be started.";
pub const REASON_OTHER: &str = "Something went wrong. Try again.";

// Session end and drop (UI 4.6).
pub const CONNECTION_LOST: &str = "Connection lost";
pub const RECONNECT: &str = "Reconnect";
pub const RECONNECT_NOW: &str = "Reconnect now";
pub const WAITING_FOR_UNLOCK: &str = "Reconnecting after Shell++ is unlocked";
pub const AUTO_RECONNECT_STOPPED: &str = "Automatic reconnection is stopped.";

/// "Session ended with code 2".
pub fn ended_with_code(code: u32) -> String {
    format!("Session ended with code {code}")
}

/// "Session ended with signal TERM".
pub fn ended_with_signal(signal: &str) -> String {
    format!("Session ended with signal {signal}")
}

/// "Reconnecting in 4 s".
pub fn reconnect_in(seconds: u64) -> String {
    format!("Reconnecting in {seconds} s")
}

/// "Reconnecting (attempt 3)".
pub fn reconnecting(attempt: u32) -> String {
    if attempt <= 1 {
        "Reconnecting".to_owned()
    } else {
        format!("Reconnecting (attempt {attempt})")
    }
}

// Prompts (UI 4.8).
pub const USERNAME_TITLE: &str = "Username";
pub const USERNAME_LABEL: &str = "Username";
pub const PASSWORD_TITLE: &str = "Password";
pub const PASSWORD_LABEL: &str = "Password";
pub const PASSWORD_ERROR: &str = "Wrong password. Try again.";
pub const PASSWORD_SAVE: &str = "Save the password in the encrypted storage";
pub const PASSPHRASE_TITLE: &str = "Key passphrase";
pub const PASSPHRASE_LABEL: &str = "Passphrase";
pub const PASSPHRASE_ERROR: &str = "Wrong passphrase. Try again.";
pub const PASSPHRASE_REMEMBER: &str = "Remember until Shell++ locks or exits";
pub const PASSPHRASE_SAVE: &str = "Save the passphrase in the encrypted storage";
pub const PASSPHRASE_REMEMBER_HINT: &str = "Kept in memory only, never written to disk.";
pub const KI_TITLE: &str = "The server asks";
pub const KI_ERROR: &str = "The server did not accept the answer. Try again.";
pub const KI_ANSWER: &str = "Answer";
pub const CONTINUE: &str = "Continue";

/// "Password for alice@example.test".
pub fn password_for(target: &str) -> String {
    format!("Password for {target}")
}

/// "Passphrase of the key id_ed25519".
pub fn passphrase_for(key: &str) -> String {
    format!("Passphrase of the key {key}")
}

/// "Message from db.example.test": the caption of the server's text in a
/// keyboard-interactive dialog.
pub fn message_from(host: &str) -> String {
    format!("Message from {host}")
}

/// "Question 2 of 3".
pub fn question_of(index: u32, count: u32) -> String {
    format!("Question {} of {count}", index + 1)
}

// Host keys (UI 10.2).
pub const HOST_KEY_ADDRESS: &str = "Address";
pub const HOST_KEY_TYPE: &str = "Key type";
pub const HOST_KEY_FINGERPRINT: &str = "SHA256 fingerprint";
pub const UNKNOWN_TITLE: &str = "First connection to this server";
pub const ACCEPT_AND_SAVE: &str = "Accept and save";
pub const CONNECT_ONCE: &str = "Connect once";
pub const CHANGED_TITLE: &str = "The host key has changed";
pub const CHANGED_OLD: &str = "Saved key";
pub const CHANGED_NEW: &str = "Presented key";
pub const CHANGED_HINT: &str = "Replace the saved key only when you know why it changed, for example after the administrator confirmed the new fingerprint.";
pub const REPLACE_KEY: &str = "Replace saved key";
pub const REVOKED_TITLE: &str = "Revoked host key";
pub const REVOKED_HINT: &str = "There is no way to continue with this key.";

/// The TOFU explanation.
pub fn unknown_text(address: &str) -> String {
    format!(
        "There is no saved key for {address} yet. Before you continue, compare the fingerprint with the one published by the server's administrator."
    )
}

/// The changed-key warning.
pub fn changed_text(address: &str) -> String {
    format!(
        "{address} presented a different key than the saved one. Someone may be intercepting the connection (a man-in-the-middle attack), or the server was reinstalled or got a new key. The connection is blocked."
    )
}

/// The revoked-key warning.
pub fn revoked_text(address: &str) -> String {
    format!(
        "{address} presented a key that is marked as revoked in Known Hosts. The connection is blocked."
    )
}

// Multi-line paste (UI 4.3). One line with a final line break asks too:
// the line break runs it.
pub const PASTE: &str = "Paste";

/// The title of the confirmation for text of `count` lines.
pub fn paste_title(count: usize) -> &'static str {
    if count == 1 {
        "Paste and run?"
    } else {
        "Paste several lines?"
    }
}

/// "The text has 3 lines. In a shell, each line break runs the line before
/// it."
pub fn paste_lines(count: usize) -> String {
    if count == 1 {
        "The text has 1 line. In a shell, the line break runs it.".to_owned()
    } else {
        format!("The text has {count} lines. In a shell, each line break runs the line before it.")
    }
}

/// "… and 12 more lines".
pub fn paste_more(count: usize) -> String {
    if count == 1 {
        "… and 1 more line".to_owned()
    } else {
        format!("… and {count} more lines")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One line with a final line break asks too: its wording is singular
    /// and the title fits one line.
    #[test]
    fn paste_wording_follows_the_line_count() {
        assert_eq!(
            paste_lines(1),
            "The text has 1 line. In a shell, the line break runs it."
        );
        assert_eq!(paste_title(1), "Paste and run?");
        assert_eq!(
            paste_lines(3),
            "The text has 3 lines. In a shell, each line break runs the line before it."
        );
        assert_eq!(paste_title(3), "Paste several lines?");
        assert_eq!(paste_more(1), "… and 1 more line");
        assert_eq!(paste_more(12), "… and 12 more lines");
    }
}
