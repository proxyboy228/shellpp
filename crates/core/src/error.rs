//! The error type of the core facade and its stable codes
//! (docs/architecture.md, section 3.4: `{"code": "...", "details": {}}`).
//!
//! Owner: lead. This file is part of the IPC contract: every `code` below has
//! an i18n key `errors.<code>` in the frontend. Add a variant only together with
//! its code, its i18n key and an entry in the stage plan.
//!
//! Error values never contain secrets, host addresses, usernames, command text
//! or file paths (docs/security.md, section 9.1).

use serde::Serialize;
use serde_json::{Value, json};

use crate::crypto::CryptoError;
use crate::store::StoreError;

/// Why a field value was refused (`validation_failed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationReason {
    /// A required value is missing or empty.
    Required,
    /// Longer than the field limit.
    TooLong,
    /// Shorter than the minimum (for example a master password under 8 characters).
    TooShort,
    /// Not a valid value for this field (wrong characters, format or enum value).
    Invalid,
    /// A number outside its range.
    OutOfRange,
    /// A value that must be unique is repeated (tag, environment variable name).
    Duplicate,
    /// Too many items in a list.
    TooMany,
    /// A group would be moved into itself or its subtree.
    Cycle,
    /// Group nesting deeper than 32 levels.
    TooDeep,
    /// The value is not allowed here (for example a secret path inside `data`).
    NotAllowed,
    /// A referenced record does not exist or has the wrong type.
    MissingRef,
}

/// One refused field. `field` is a dotted path such as `address`, `tags.3`,
/// `ssh.port`, `ssh.env.2.name`, `name`, `parent_id`, `password`, `new_password`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub reason: ValidationReason,
}

impl FieldError {
    pub fn new(field: impl Into<String>, reason: ValidationReason) -> Self {
        Self {
            field: field.into(),
            reason,
        }
    }
}

/// Why an SSH connection failed (`ssh_error`, `details.reason`; stage 2 plan,
/// section 5.3). The desktop host maps `shellpp_ssh::FailureKind` to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshReason {
    /// The host name did not resolve.
    NotFound,
    /// The TCP connection was refused.
    Refused,
    /// Connecting or the handshake timed out.
    Timeout,
    /// Another network error.
    Network,
    /// The user rejected the host key (Cancel in the TOFU or changed-key dialog).
    HostKeyRejected,
    /// Every authentication method failed.
    AuthFailed,
    /// Protocol error or no common algorithm.
    Protocol,
    /// The server refused the session, PTY, shell or exec request.
    ChannelFailed,
    /// The connection was closed (transport error, keepalive timeout).
    Disconnected,
    /// The linked private key cannot be decoded.
    KeyUnusable,
}

impl SshReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::Network => "network",
            Self::HostKeyRejected => "host_key_rejected",
            Self::AuthFailed => "auth_failed",
            Self::Protocol => "protocol",
            Self::ChannelFailed => "channel_failed",
            Self::Disconnected => "disconnected",
            Self::KeyUnusable => "key_unusable",
        }
    }
}

/// Errors returned by [`crate::service::Core`].
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The app is locked (or still at first run) and the operation needs keys.
    #[error("locked")]
    Locked,
    /// The operation is not valid in the current lock state (for example
    /// `first_run_setup` after the first run, or `unlock` while unlocked).
    #[error("wrong state")]
    WrongState,
    /// The master password did not open the vault key.
    #[error("invalid password")]
    InvalidPassword,
    /// Unlock attempts pause after 5 wrong master passwords in a row
    /// (decision D29); `retry_after_secs` is the time left, rounded up.
    #[error("too many attempts")]
    TooManyAttempts { retry_after_secs: u64 },
    /// The recovery key is well-formed but does not open the recovery wrap.
    #[error("invalid recovery key")]
    InvalidRecoveryKey,
    /// A record, vault or secret field does not exist.
    #[error("not found")]
    NotFound,
    /// One or more fields were refused; nothing was written.
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    /// The record or vault cannot be written (archived vault, viewer role, newer payload version).
    #[error("read only")]
    ReadOnly,
    /// The payload exceeds the envelope size limit.
    #[error("too large")]
    TooLarge,
    /// A platform service is missing (no OS keychain, no clipboard).
    #[error("unavailable")]
    Unavailable,
    /// The database was written by a newer app version and was not opened.
    #[error("newer schema")]
    NewerSchema { found: u32 },
    /// A stored record or key wrap failed its integrity checks.
    #[error("damaged data")]
    Damaged,
    /// The user cancelled the operation (a connection attempt or a prompt).
    #[error("cancelled")]
    Cancelled,
    /// An SSH connection failed; the reason is in `details.reason`.
    #[error("ssh error")]
    Ssh(SshReason),
    /// The server presented a host key marked `@revoked` in known hosts
    /// (docs/data-model.md, section 9.5; UI 10.2). Never continues.
    #[error("host key revoked")]
    HostKeyRevoked,
    /// A sync request failed (stage 7): no answer, a refused certificate
    /// (`details` names the problem), or the server's error code.
    #[error("sync error: {0}")]
    Sync(crate::sync::HttpError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A bug or an impossible state. The string is a static description without data.
    #[error("internal error: {0}")]
    Internal(&'static str),
}

impl CoreError {
    /// Shorthand for a single refused field.
    pub fn field(field: impl Into<String>, reason: ValidationReason) -> Self {
        Self::Validation(vec![FieldError::new(field, reason)])
    }

    /// The stable snake_case code sent to the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::WrongState => "wrong_state",
            Self::InvalidPassword => "invalid_password",
            Self::TooManyAttempts { .. } => "too_many_attempts",
            Self::InvalidRecoveryKey => "invalid_recovery_key",
            Self::NotFound => "not_found",
            Self::Validation(_) => "validation_failed",
            Self::ReadOnly => "read_only",
            Self::TooLarge => "too_large",
            Self::Unavailable => "unavailable",
            Self::NewerSchema { .. } => "newer_schema",
            Self::Damaged => "damaged",
            Self::Cancelled => "cancelled",
            Self::Ssh(_) => "ssh_error",
            Self::HostKeyRevoked => "host_key_revoked",
            Self::Sync(_) => "sync_error",
            Self::Crypto(error) => error.code(),
            Self::Store(error) => error.code(),
            Self::Internal(_) => "internal",
        }
    }

    /// Code-specific details: `{"errors": [{"field", "reason"}]}` for
    /// `validation_failed`, `{"found": N}` for `newer_schema`,
    /// `{"reason": "..."}` for `ssh_error`, `{"retry_after_secs": N}` for
    /// `too_many_attempts`, `{}` otherwise.
    pub fn details(&self) -> Value {
        match self {
            Self::Validation(errors) => json!({ "errors": errors }),
            Self::NewerSchema { found } => json!({ "found": found }),
            Self::Ssh(reason) => json!({ "reason": reason }),
            Self::TooManyAttempts { retry_after_secs } => {
                json!({ "retry_after_secs": retry_after_secs })
            }
            Self::Sync(error) => json!({ "reason": error.code() }),
            _ => json!({}),
        }
    }
}
