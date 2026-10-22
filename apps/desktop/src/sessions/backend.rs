//! What the session layer needs from the core. Sessions call the core only
//! through [`SessionCore`] and only on blocking threads of the session
//! runtime, never from an async task directly.

use secrecy::SecretString;
use shellpp_core::Core;
use shellpp_core::error::CoreError;
use shellpp_core::os_detect::{self, DetectedOs};
use shellpp_core::service::dto::{OsSource, RecordRef, SessionTarget, SettingsValues};
use shellpp_core::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryDropReason,
    HistoryRecordInput, HostKeyCheck, HostKeyStatus, SessionConfig,
};
use uuid::Uuid;

/// The core operations of sessions. Implemented by [`Core`].
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
    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError>;
    fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError>;
    fn connection_log_end(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    );
    /// The recording of the session of log entry `log_id`, when its output
    /// is recorded (UI 12.6, 3.1; D75); `None` when it is not or its file
    /// could not be made. Fakes record nothing.
    fn recording_for(
        &self,
        _host: Option<RecordRef>,
        _log_id: Uuid,
        _size: (u16, u16),
    ) -> Option<shellpp_core::recording::RecordingWriter> {
        None
    }
    /// The forwarding rules of the personal vault (stage 5). Fakes have
    /// none.
    fn forwarding_rules(
        &self,
    ) -> Result<Vec<shellpp_core::service::snippets::ForwardingRuleView>, CoreError> {
        Ok(Vec::new())
    }
    /// The Keychain keys of the built-in agent (stage 5, D60). Fakes have
    /// none.
    fn agent_keys(&self) -> Result<Vec<shellpp_core::service::host_api::SessionKey>, CoreError> {
        Ok(Vec::new())
    }
    /// A command line the terminal's history capture passed
    /// (docs/security.md, section 10.1): the core checks every rule again
    /// and writes a `history_entry` when they all pass. Fakes drop it.
    fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError> {
        drop(input);
        Ok(HistoryDecision::Dropped(HistoryDropReason::Off))
    }
    /// The session of [`SessionCore::history_record`]'s `session_id` ended:
    /// its previous command is forgotten. Fakes keep nothing.
    fn history_session_end(&self, session_id: Uuid) {
        let _ = session_id;
    }
    /// Whether a session to `host` records history now (docs/security.md,
    /// section 10.1, rule 4: the host's override, else the setting);
    /// `false` when the core cannot tell (locked). Fakes say yes.
    fn record_history_now(&self, host: RecordRef) -> bool {
        let _ = host;
        true
    }
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

    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        Core::host_os_detected(self, host, os, source)
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

    fn recording_for(
        &self,
        host: Option<RecordRef>,
        log_id: Uuid,
        size: (u16, u16),
    ) -> Option<shellpp_core::recording::RecordingWriter> {
        if !Core::record_output_for(self, host) {
            return None;
        }
        match Core::recording_start(self, log_id, size) {
            Ok(writer) => Some(writer),
            Err(error) => {
                tracing::warn!(code = error.code(), "session output not recorded");
                None
            }
        }
    }

    fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError> {
        Core::history_record(self, input)
    }

    fn history_session_end(&self, session_id: Uuid) {
        Core::history_session_end(self, session_id);
    }

    fn record_history_now(&self, host: RecordRef) -> bool {
        Core::record_history_now(self, Some(host)).unwrap_or(false)
    }

    fn agent_keys(&self) -> Result<Vec<shellpp_core::service::host_api::SessionKey>, CoreError> {
        Core::agent_keys(self)
    }

    fn forwarding_rules(
        &self,
    ) -> Result<Vec<shellpp_core::service::snippets::ForwardingRuleView>, CoreError> {
        Core::forwarding_rules_personal(self)
    }
}

/// OS named by the server identification string.
pub(crate) fn os_from_banner(ident: &str) -> Option<DetectedOs> {
    os_detect::from_banner(ident)
}

/// OS from the stdout of the detection script.
pub(crate) fn os_from_script(stdout: &str) -> Option<DetectedOs> {
    os_detect::from_script_output(stdout)
}

/// Setting `terminal.term` default (UI 12.4).
pub const DEFAULT_TERM: &str = "xterm-256color";
/// Built-in default terminal theme (design-system 4.1).
pub const DEFAULT_THEME: &str = "graphite";

/// Typed reads of the settings sessions use (UI 12.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSettings {
    /// `terminal.autoreconnect`, on by default.
    pub autoreconnect: bool,
    pub os_detection_enabled: bool,
    /// `terminal.term`: TERM of SSH sessions and local terminals.
    pub term: String,
    /// `terminal.local_shell`: a shell id or a custom path.
    pub local_shell: Option<String>,
    /// Built-in slug of `terminal.default_theme`.
    pub default_theme: String,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            autoreconnect: true,
            os_detection_enabled: true,
            term: DEFAULT_TERM.to_owned(),
            local_shell: None,
            default_theme: DEFAULT_THEME.to_owned(),
        }
    }
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
                .and_then(theme_slug)
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

/// The built-in slug of a `theme_ref` (`{"builtin": "paper"}`).
pub(crate) fn theme_slug(value: &serde_json::Value) -> Option<String> {
    value
        .get("builtin")
        .and_then(serde_json::Value::as_str)
        .filter(|slug| !slug.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn settings_defaults_and_values() {
        let mut values = SettingsValues::new();
        assert_eq!(
            SessionSettings::from_values(&values),
            SessionSettings::default()
        );
        values.insert("terminal.autoreconnect".into(), json!(false));
        values.insert("terminal.os_detection_enabled".into(), json!(false));
        values.insert("terminal.term".into(), json!("vt100"));
        values.insert("terminal.local_shell".into(), json!("cmd"));
        values.insert("terminal.default_theme".into(), json!({"builtin": "paper"}));
        let settings = SessionSettings::from_values(&values);
        assert!(!settings.autoreconnect);
        assert!(!settings.os_detection_enabled);
        assert_eq!(settings.term, "vt100");
        assert_eq!(settings.local_shell.as_deref(), Some("cmd"));
        assert_eq!(settings.default_theme, "paper");

        values.insert("terminal.local_shell".into(), serde_json::Value::Null);
        values.insert("terminal.autoreconnect".into(), json!("yes"));
        let settings = SessionSettings::from_values(&values);
        assert_eq!(settings.local_shell, None);
        assert!(settings.autoreconnect);
    }
}
