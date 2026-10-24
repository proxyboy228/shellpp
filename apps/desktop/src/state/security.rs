//! Lock and security operations of the app state (UI 13.1-13.4, decision
//! D29): the first run with "Remember on this device", the unlock attempt
//! limit, "Start with a new vault", and the Security settings (auto-lock,
//! remember toggle).
//!
//! Calls that run Argon2id or touch the OS keychain or the data files run on
//! the background executor and return a task; the lock state is refreshed
//! when they finish (the core's `lock:changed` event does the same a moment
//! later).

use std::time::Duration;

use gpui::{App, AppContext as _, Context, Global, Task};
use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::CoreError;
use shellpp_core::lock::FirstRunOptions;
use shellpp_core::service::dto::{LockStatus, SettingsSetInput};
use shellpp_core::settings::{AUTO_LOCK_KEY, SYSTEM_AGENT_WHILE_LOCKED_KEY};

use super::AppState;

/// Auto-lock intervals offered in Settings > Security, in minutes (D29);
/// `None` is "Never", the default.
pub const AUTO_LOCK_CHOICES: [Option<u64>; 5] = [None, Some(5), Some(15), Some(30), Some(60)];

/// The file name the old database got at the last "Start with a new vault"
/// in this run (shown on the first-run screen until a vault is created).
#[derive(Clone, Debug, Default)]
pub struct ArchivedDataFile(pub Option<String>);

impl Global for ArchivedDataFile {}

impl ArchivedDataFile {
    /// The archived file name, if any.
    pub fn get(cx: &App) -> Option<String> {
        cx.try_global::<Self>().and_then(|this| this.0.clone())
    }
}

impl AppState {
    /// The lock status of the core (`None` without a core).
    pub fn lock_status(&self) -> Option<LockStatus> {
        self.core.as_ref().map(|core| core.lock_status())
    }

    /// `true` when an OS keychain is usable, so "Remember on this device"
    /// is offered.
    pub fn remember_available(&self) -> bool {
        self.lock_status()
            .is_some_and(|status| status.remember_device_available)
    }

    /// `true` while a device key is stored for this vault.
    pub fn remember_enabled(&self) -> bool {
        self.lock_status()
            .is_some_and(|status| status.remember_device_enabled)
    }

    /// First run as the first-run screen runs it (UI 13.1): the master
    /// password and "Remember on this device". Argon2id and the keychain
    /// write run on the background executor. The recovery key is created
    /// and kept in the vault, but local mode never shows it (D29).
    pub fn create_vault(
        &mut self,
        password: SecretString,
        remember_device: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<LockStatus, CoreError>> {
        let core = match self.require_core() {
            Ok(core) => core.clone(),
            Err(error) => return Task::ready(Err(error)),
        };
        let options = FirstRunOptions { remember_device };
        let work = cx.background_spawn(async move { core.first_run_setup_with(password, options) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |state, cx| {
                if result.is_ok() {
                    cx.set_global(ArchivedDataFile(None));
                }
                state.refresh_lock(cx);
            });
            result
        })
    }

    /// Time left in the pause after 5 wrong passwords in a row (D29);
    /// `None` when an unlock attempt is allowed now.
    pub fn unlock_retry_after(&self) -> Option<Duration> {
        self.core
            .as_ref()
            .and_then(|core| core.unlock_retry_after())
    }

    /// "Forgot password?" -> "Start with a new vault" (UI 13.2, D29): the
    /// core renames the data file (never deletes it) and returns to the
    /// first run. Returns the archive's file name.
    pub fn start_new_vault(&mut self, cx: &mut Context<Self>) -> Task<Result<String, CoreError>> {
        let core = match self.require_core() {
            Ok(core) => core.clone(),
            Err(error) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move { core.start_new_vault() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |state, cx| {
                if let Ok(name) = &result {
                    tracing::info!("started with a new vault");
                    cx.set_global(ArchivedDataFile(Some(name.clone())));
                }
                state.refresh_lock(cx);
            });
            result
        })
    }

    /// The auto-lock interval in minutes; `None` is "Never" (the default).
    pub fn auto_lock_minutes(&self) -> Option<u64> {
        self.core
            .as_ref()
            .and_then(|core| core.settings_get().ok())
            .and_then(|values| values.get(AUTO_LOCK_KEY).and_then(Value::as_u64))
    }

    /// Sets the auto-lock interval (Settings > Security). Needs the unlocked
    /// state; the core's timer uses the new value at once.
    pub fn set_auto_lock_minutes(
        &mut self,
        minutes: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        let value = minutes.map_or(Value::Null, |minutes| json!(minutes));
        let mut values = std::collections::BTreeMap::new();
        values.insert(AUTO_LOCK_KEY.to_owned(), value);
        self.require_core()?.settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        })?;
        tracing::info!(minutes = ?minutes, "auto-lock interval changed");
        cx.notify();
        Ok(())
    }

    /// Whether forwarded agents keep offering the system agent's keys while
    /// the app is locked (D194); off by default.
    pub fn system_agent_while_locked(&self) -> bool {
        self.core
            .as_ref()
            .and_then(|core| core.settings_get().ok())
            .and_then(|values| {
                values
                    .get(SYSTEM_AGENT_WHILE_LOCKED_KEY)
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false)
    }

    /// Sets it (Settings > Security); the sessions read it at each lock.
    pub fn set_system_agent_while_locked(
        &mut self,
        on: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        let mut values = std::collections::BTreeMap::new();
        values.insert(SYSTEM_AGENT_WHILE_LOCKED_KEY.to_owned(), json!(on));
        self.require_core()?.settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        })?;
        tracing::info!(on, "system agent while locked changed");
        cx.notify();
        Ok(())
    }

    /// Turns "Remember on this device" on or off (Settings > Security). The
    /// keychain call runs on the background executor.
    pub fn set_remember_device(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Task<Result<LockStatus, CoreError>> {
        let core = match self.require_core() {
            Ok(core) => core.clone(),
            Err(error) => return Task::ready(Err(error)),
        };
        let work = cx.background_spawn(async move { core.remember_device_set(enabled) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |_, cx| cx.notify());
            result
        })
    }
}
