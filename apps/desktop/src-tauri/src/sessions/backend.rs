//! What the session manager needs from the core (stage 2 plan, section 4.5).
//!
//! The manager calls the core only through [`SessionCore`], always on a
//! blocking thread, so tests can run the state machine against an in-memory
//! fake and the real [`Core`] is never called from an async task directly.

use secrecy::SecretString;
use shellpp_core::Core;
use shellpp_core::error::CoreError;
use shellpp_core::os_detect::{self, DetectedOs};
use shellpp_core::service::dto::{OsSource, RecordRef, SessionTarget, SettingsValues};
use shellpp_core::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryRecordInput, HostKeyCheck,
    HostKeyStatus, SessionConfig,
};
use uuid::Uuid;

/// The core operations of sessions. Implemented by [`Core`]; tests use fakes.
pub trait SessionCore: Send + Sync + 'static {
    fn is_unlocked(&self) -> bool;
    /// Every setting (readable in every lock state).
    fn settings(&self) -> Result<SettingsValues, CoreError>;
    fn session_config(&self, target: &SessionTarget) -> Result<SessionConfig, CoreError>;
    fn known_host_check(&self, check: &HostKeyCheck) -> Result<HostKeyStatus, CoreError>;
    fn known_host_accept(&self, check: &HostKeyCheck) -> Result<(), CoreError>;
    fn known_host_replace(&self, record_id: Uuid, check: &HostKeyCheck) -> Result<(), CoreError>;
    fn save_prompted_password(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError>;
    fn save_key_passphrase(
        &self,
        key: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError>;
    /// OS named by the server identification string.
    fn os_from_banner(&self, ident: &str) -> Option<DetectedOs>;
    /// OS from the stdout of the detection script.
    fn os_from_script(&self, stdout: &str) -> Option<DetectedOs>;
    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError>;
    fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError>;
    /// Whether a session linked to `host` records history now.
    fn record_history_now(&self, host: Option<RecordRef>) -> Result<bool, CoreError>;
    fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError>;
    fn connection_log_end(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    );
}

impl SessionCore for Core {
    fn is_unlocked(&self) -> bool {
        Core::is_unlocked(self)
    }

    fn settings(&self) -> Result<SettingsValues, CoreError> {
        self.settings_get()
    }

    fn session_config(&self, target: &SessionTarget) -> Result<SessionConfig, CoreError> {
        Core::session_config(self, target)
    }

    fn known_host_check(&self, check: &HostKeyCheck) -> Result<HostKeyStatus, CoreError> {
        Core::known_host_check(self, check)
    }

    fn known_host_accept(&self, check: &HostKeyCheck) -> Result<(), CoreError> {
        Core::known_host_accept(self, check).map(|_| ())
    }

    fn known_host_replace(&self, record_id: Uuid, check: &HostKeyCheck) -> Result<(), CoreError> {
        Core::known_host_replace(self, record_id, check).map(|_| ())
    }

    fn save_prompted_password(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError> {
        Core::save_prompted_password(self, host, password)
    }

    fn save_key_passphrase(
        &self,
        key: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError> {
        Core::save_key_passphrase(self, key, passphrase)
    }

    fn os_from_banner(&self, ident: &str) -> Option<DetectedOs> {
        os_detect::from_banner(ident)
    }

    fn os_from_script(&self, stdout: &str) -> Option<DetectedOs> {
        os_detect::from_script_output(stdout)
    }

    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        Core::host_os_detected(self, host, os, source)
    }

    fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError> {
        Core::history_record(self, input)
    }

    fn record_history_now(&self, host: Option<RecordRef>) -> Result<bool, CoreError> {
        Core::record_history_now(self, host)
    }

    fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError> {
        Core::connection_log_start(self, entry)
    }

    fn connection_log_end(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    ) {
        Core::connection_log_end(self, log_id, reason, exit_status);
    }
}

/// Typed reads of the settings the session manager uses (stage 2 plan, 5.5).
#[derive(Clone, Debug, PartialEq)]
pub struct SessionSettings {
    pub autoreconnect: bool,
    pub record_history: bool,
    pub os_detection_enabled: bool,
    pub term: String,
    pub local_shell: Option<String>,
    pub default_theme: serde_json::Value,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            autoreconnect: true,
            record_history: true,
            os_detection_enabled: true,
            term: DEFAULT_TERM.to_owned(),
            local_shell: None,
            default_theme: default_theme(),
        }
    }
}

/// Setting `terminal.term` default (UI 12.4).
pub const DEFAULT_TERM: &str = "xterm-256color";

/// The built-in default theme reference.
pub fn default_theme() -> serde_json::Value {
    serde_json::json!({"builtin": "graphite"})
}

impl SessionSettings {
    /// Reads the values; missing or mistyped values keep the defaults.
    pub fn from_values(values: &SettingsValues) -> Self {
        let defaults = Self::default();
        let flag = |key: &str, default: bool| {
            values
                .get(key)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default)
        };
        Self {
            autoreconnect: flag("terminal.autoreconnect", defaults.autoreconnect),
            record_history: flag("terminal.record_history", defaults.record_history),
            os_detection_enabled: flag(
                "terminal.os_detection_enabled",
                defaults.os_detection_enabled,
            ),
            term: values
                .get("terminal.term")
                .and_then(serde_json::Value::as_str)
                .filter(|term| !term.is_empty())
                .map_or(defaults.term, str::to_owned),
            local_shell: values
                .get("terminal.local_shell")
                .and_then(serde_json::Value::as_str)
                .filter(|shell| !shell.is_empty())
                .map(str::to_owned),
            default_theme: values
                .get("terminal.default_theme")
                .filter(|theme| theme.is_object())
                .cloned()
                .unwrap_or(defaults.default_theme),
        }
    }

    /// Reads the settings from the core; the defaults when that fails.
    pub fn read(core: &dyn SessionCore) -> Self {
        match core.settings() {
            Ok(values) => Self::from_values(&values),
            Err(error) => {
                tracing::warn!(code = error.code(), "settings not readable; defaults used");
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn settings_defaults_and_values() {
        assert_eq!(
            SessionSettings::from_values(&SettingsValues::new()),
            SessionSettings::default()
        );
        let mut values = SettingsValues::new();
        values.insert("terminal.autoreconnect".into(), json!(false));
        values.insert("terminal.record_history".into(), json!(false));
        values.insert("terminal.os_detection_enabled".into(), json!(false));
        values.insert("terminal.term".into(), json!("vt100"));
        values.insert("terminal.local_shell".into(), json!("cmd"));
        values.insert("terminal.default_theme".into(), json!({"builtin": "paper"}));
        let settings = SessionSettings::from_values(&values);
        assert!(!settings.autoreconnect);
        assert!(!settings.record_history);
        assert!(!settings.os_detection_enabled);
        assert_eq!(settings.term, "vt100");
        assert_eq!(settings.local_shell.as_deref(), Some("cmd"));
        assert_eq!(settings.default_theme, json!({"builtin": "paper"}));

        values.insert("terminal.local_shell".into(), serde_json::Value::Null);
        values.insert("terminal.autoreconnect".into(), json!("yes"));
        let settings = SessionSettings::from_values(&values);
        assert_eq!(settings.local_shell, None);
        assert!(settings.autoreconnect);
    }
}
