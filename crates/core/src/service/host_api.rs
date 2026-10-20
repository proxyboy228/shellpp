//! Types the desktop host exchanges with the core for sessions (stage 2 plan,
//! section 4.4). They are never sent to the webview: some carry secrets.
//!
//! Owner: S4 (with S5 as the only caller). Written by the lead; change the
//! shapes only through the plan.

use std::fmt;

use secrecy::SecretString;
use uuid::Uuid;

use super::dto::{Backspace, HistoryCandidate, HostOs, RecordRef};

/// The effective connection settings of a session (docs/data-model.md,
/// sections 8.3 and 8.5; docs/architecture.md, section 4.2, step 2).
pub struct SessionConfig {
    /// The saved host, `None` for a quick connect.
    pub host: Option<RecordRef>,
    /// Tab title: the host label, else its address.
    pub title: String,
    pub address: String,
    /// Effective port (own, inherited, else 22).
    pub port: u16,
    /// Effective username (own, inherited); `None` = ask (UI 4.8).
    pub username: Option<String>,
    /// Effective password (`host.ssh.password`, else the identity's,
    /// docs/data-model.md, section 8.3).
    pub password: Option<SecretString>,
    /// Effective key (`ssh.key_id`, own or inherited). A missing key record
    /// counts as unset (docs/data-model.md, section 8.3).
    pub key: Option<SessionKey>,
    /// Effective keepalive in seconds (host `ssh.keepalive_s`, else
    /// `terminal.keepalive_s`); 0 = off.
    pub keepalive_s: u32,
    /// Setting `terminal.term`.
    pub term: String,
    pub agent_forwarding: bool,
    /// `ssh.env`, values as stored.
    pub env: Vec<(String, SecretString)>,
    /// Host `charset` (WHATWG label, lowercase), default `utf-8`.
    pub charset: String,
    pub backspace: Backspace,
    /// Effective theme reference (`theme_ref` JSON).
    pub terminal_theme: serde_json::Value,
    /// Effective "Record history".
    pub record_history: bool,
    /// Some when OS detection should run after connecting: detection is on,
    /// the host is saved and has no `os` yet. The script text (setting
    /// `terminal.os_detection_script` or the built-in script).
    pub os_detection_script: Option<String>,
    /// The host's current `os`.
    pub os: Option<HostOs>,
    /// Key types saved in known hosts for (address, port), for the preferred
    /// host key algorithms (stage 2 plan, section 4.2).
    pub known_key_types: Vec<String>,
    /// A prompted password may be saved: a saved host in the personal vault.
    pub can_save_password: bool,
    /// Hops of the effective `jump_chain` (own, else inherited; `[]` stops
    /// the lookup). Jump hosts arrive with stage 5: until then a host with
    /// hops is refused instead of connected directly.
    pub jump_hosts: usize,
}

impl fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionConfig")
            .field("host", &self.host)
            .field("password", &self.password.is_some())
            .field("key", &self.key.is_some())
            .field("keepalive_s", &self.keepalive_s)
            .finish_non_exhaustive()
    }
}

/// The effective key of a session.
pub struct SessionKey {
    pub key: RecordRef,
    pub label: String,
    /// OpenSSH private key text, possibly encrypted.
    pub private_key: SecretString,
    pub passphrase: Option<SecretString>,
    /// A prompted passphrase may be saved in the key record.
    pub can_save_passphrase: bool,
}

impl fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionKey")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// A presented host key to check against known hosts.
#[derive(Clone)]
pub struct HostKeyCheck {
    /// The address as connected (host name or IP, no brackets).
    pub address: String,
    pub port: u16,
    pub key_type: String,
    /// Base64 key blob.
    pub public_key: String,
}

impl fmt::Debug for HostKeyCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostKeyCheck")
            .field("key_type", &self.key_type)
            .finish_non_exhaustive()
    }
}

/// Result of a known hosts lookup (docs/data-model.md, section 9.5; UI 10.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// An entry for this host has this key: continue.
    Match { record_id: Uuid },
    /// No entry matches the host: TOFU dialog.
    Unknown,
    /// Entries match the host, none with this key: blocked, changed-key dialog.
    /// `record_id` is the entry "Replace saved key" updates.
    Changed {
        record_id: Uuid,
        old_key_type: String,
        old_fingerprint: String,
    },
    /// A `revoked` entry has this key (any host): blocked, no way to continue.
    Revoked { record_id: Uuid },
}

/// A history candidate with what only the host knows.
pub struct HistoryRecordInput {
    /// The session's saved host; `None` for local terminals and quick connects.
    pub host: Option<RecordRef>,
    /// For the "same as the previous entry of this session" rule.
    pub session_id: Uuid,
    pub candidate: HistoryCandidate,
    /// The line contained input sent with `session_inject` (docs/security.md, section 10.1).
    pub injected: bool,
    /// Effective "Record history" of the session.
    pub record_history: bool,
}

impl fmt::Debug for HistoryRecordInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HistoryRecordInput")
            .field("host", &self.host)
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

/// What the core did with a candidate (tests and debug logs; never sent to the UI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDecision {
    Recorded,
    Dropped(HistoryDropReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDropReason {
    /// History recording is off for the host or globally, or the linked
    /// host of the personal vault no longer exists.
    Off,
    AltScreen,
    EchoOff,
    PasswordPrompt,
    Empty,
    LeadingSpace,
    TooLong,
    Secret,
    Injected,
    Duplicate,
    Locked,
}

/// Start of a connection log entry (docs/data-model.md, section 13.3; written
/// when the session is established).
pub struct ConnectionLogStart {
    pub host: Option<RecordRef>,
    /// Snapshot of the host label.
    pub host_label: Option<String>,
    pub address: String,
    pub port: u16,
    pub username: Option<String>,
    pub os: Option<HostOs>,
}

impl fmt::Debug for ConnectionLogStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionLogStart")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// `end_reason` of a connection log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionEndReason {
    /// Normal end or closed by the user.
    Closed,
    /// Non-zero exit status (UI 4.6).
    ExitStatus,
    /// Transport error.
    Dropped,
}
