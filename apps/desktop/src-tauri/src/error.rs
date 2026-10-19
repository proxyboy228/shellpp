//! The error shape of every command: `{"code": "...", "details": {...}}`
//! (docs/architecture.md, section 3.4).
//!
//! Codes come from [`CoreError::code`]; the host adds no codes of its own.
//! Errors never carry secrets, host data or paths (docs/security.md, section 9.1).

use serde::Serialize;
use serde_json::{Value, json};
use shellpp_core::error::{CoreError, FieldError, ValidationReason};

/// A rejected command as the webview receives it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CmdError {
    pub code: &'static str,
    pub details: Value,
}

impl CmdError {
    /// An error code without details.
    pub fn code(code: &'static str) -> Self {
        Self {
            code,
            details: json!({}),
        }
    }

    /// `validation_failed` for a single field.
    pub fn validation(field: &str, reason: ValidationReason) -> Self {
        Self::from(CoreError::Validation(vec![FieldError::new(field, reason)]))
    }

    /// `locked`: the command needs the unlocked state.
    pub fn locked() -> Self {
        Self::code("locked")
    }

    /// `wrong_state`: the command is not valid in the current lock state.
    pub fn wrong_state() -> Self {
        Self::code("wrong_state")
    }

    /// `unavailable`: a platform service (clipboard, launcher, keychain) failed.
    pub fn unavailable() -> Self {
        Self::code("unavailable")
    }

    /// `internal`: a bug or a panic inside a command.
    pub fn internal() -> Self {
        Self::code("internal")
    }
}

impl From<CoreError> for CmdError {
    fn from(error: CoreError) -> Self {
        Self::from(&error)
    }
}

impl From<&CoreError> for CmdError {
    fn from(error: &CoreError) -> Self {
        Self {
            code: error.code(),
            details: error.details(),
        }
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}

impl std::error::Error for CmdError {}

#[cfg(test)]
mod tests {
    use super::*;
    use shellpp_core::crypto::CryptoError;
    use shellpp_core::store::StoreError;

    fn to_json(error: CoreError) -> Value {
        serde_json::to_value(CmdError::from(error)).expect("serializable")
    }

    #[test]
    fn every_core_error_code_serializes_with_details() {
        let cases: Vec<(CoreError, Value)> = vec![
            (CoreError::Locked, json!({"code": "locked", "details": {}})),
            (
                CoreError::WrongState,
                json!({"code": "wrong_state", "details": {}}),
            ),
            (
                CoreError::InvalidPassword,
                json!({"code": "invalid_password", "details": {}}),
            ),
            (
                CoreError::InvalidRecoveryKey,
                json!({"code": "invalid_recovery_key", "details": {}}),
            ),
            (
                CoreError::NotFound,
                json!({"code": "not_found", "details": {}}),
            ),
            (
                CoreError::Validation(vec![
                    FieldError::new("address", ValidationReason::Required),
                    FieldError::new("ssh.port", ValidationReason::OutOfRange),
                ]),
                json!({"code": "validation_failed", "details": {"errors": [
                    {"field": "address", "reason": "required"},
                    {"field": "ssh.port", "reason": "out_of_range"}
                ]}}),
            ),
            (
                CoreError::ReadOnly,
                json!({"code": "read_only", "details": {}}),
            ),
            (
                CoreError::TooLarge,
                json!({"code": "too_large", "details": {}}),
            ),
            (
                CoreError::Unavailable,
                json!({"code": "unavailable", "details": {}}),
            ),
            (
                CoreError::NewerSchema { found: 7 },
                json!({"code": "newer_schema", "details": {"found": 7}}),
            ),
            (
                CoreError::Damaged,
                json!({"code": "damaged", "details": {}}),
            ),
            (
                CoreError::Crypto(CryptoError::Aead),
                json!({"code": "damaged", "details": {}}),
            ),
            (
                CoreError::Crypto(CryptoError::TooLarge),
                json!({"code": "too_large", "details": {}}),
            ),
            (
                CoreError::Crypto(CryptoError::Kdf),
                json!({"code": "internal", "details": {}}),
            ),
            (
                CoreError::Store(StoreError::NewerSchema { found: 3 }),
                json!({"code": "newer_schema", "details": {}}),
            ),
            (
                CoreError::Store(StoreError::Io(std::io::Error::other("x"))),
                json!({"code": "io_error", "details": {}}),
            ),
            (
                CoreError::Store(StoreError::Corrupt("x")),
                json!({"code": "damaged", "details": {}}),
            ),
            (
                CoreError::Store(StoreError::Migration { to: 2 }),
                json!({"code": "storage_error", "details": {}}),
            ),
            (
                CoreError::Internal("x"),
                json!({"code": "internal", "details": {}}),
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(to_json(error), expected);
        }
    }

    #[test]
    fn validation_helper_uses_the_core_shape() {
        let error = CmdError::validation("url", ValidationReason::NotAllowed);
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({"code": "validation_failed", "details": {"errors": [{"field": "url", "reason": "not_allowed"}]}})
        );
    }

    #[test]
    fn host_codes_have_empty_details() {
        for (error, code) in [
            (CmdError::locked(), "locked"),
            (CmdError::wrong_state(), "wrong_state"),
            (CmdError::unavailable(), "unavailable"),
            (CmdError::internal(), "internal"),
        ] {
            assert_eq!(
                serde_json::to_value(error).unwrap(),
                json!({"code": code, "details": {}})
            );
        }
    }
}
