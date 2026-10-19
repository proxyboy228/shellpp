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
    /// Shorter than the minimum (for example a master password under 10 characters).
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
            Self::InvalidRecoveryKey => "invalid_recovery_key",
            Self::NotFound => "not_found",
            Self::Validation(_) => "validation_failed",
            Self::ReadOnly => "read_only",
            Self::TooLarge => "too_large",
            Self::Unavailable => "unavailable",
            Self::NewerSchema { .. } => "newer_schema",
            Self::Damaged => "damaged",
            Self::Crypto(error) => error.code(),
            Self::Store(error) => error.code(),
            Self::Internal(_) => "internal",
        }
    }

    /// Code-specific details: `{"errors": [{"field", "reason"}]}` for
    /// `validation_failed`, `{"found": N}` for `newer_schema`, `{}` otherwise.
    pub fn details(&self) -> Value {
        match self {
            Self::Validation(errors) => json!({ "errors": errors }),
            Self::NewerSchema { found } => json!({ "found": found }),
            _ => json!({}),
        }
    }
}
