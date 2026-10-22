//! The app's identity on Windows (D70; D83, item 4): the process sets its
//! AppUserModelID at every start, so the taskbar groups its windows and
//! system notifications name it; the ID is registered for the user under
//! `HKCU\Software\Classes\AppUserModelId` at the first start (not at the
//! first notification), so a notification shows "Shell++" from the start.
//! A test run (`SHELLPP_TEST=1`) and a temporary profile (`--demo`) set and
//! register nothing.
//!
//! The calls go through [`IdentitySystem`], so tests check what a start
//! does without touching the process or the registry.

use gpui::App;

use super::APP_USER_MODEL_ID;
use crate::i18n;

/// Where the identity goes.
pub trait IdentitySystem {
    /// Sets the AppUserModelID of this process.
    fn set_process_identity(&self, id: &str, name: &str);
    /// Whether the ID is registered for the user.
    fn is_registered(&self, id: &str) -> bool;
    /// Registers the ID for the user with its display name; `false` when
    /// the registry refused.
    fn register(&self, id: &str, name: &str) -> bool;
}

/// What a start did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// A test run or a temporary profile: nothing.
    Nothing,
    /// The process identity only (already registered).
    Set,
    /// The process identity and the registration (the first start).
    SetAndRegistered,
}

/// Sets the identity of a start on the user's profile (`real_profile`),
/// registering it when it is not yet.
pub fn apply(real_profile: bool, system: &dyn IdentitySystem) -> Applied {
    if !real_profile {
        return Applied::Nothing;
    }
    system.set_process_identity(APP_USER_MODEL_ID, i18n::APP_NAME);
    if system.is_registered(APP_USER_MODEL_ID) {
        return Applied::Set;
    }
    if system.register(APP_USER_MODEL_ID, i18n::APP_NAME) {
        Applied::SetAndRegistered
    } else {
        tracing::warn!("the app identity was not registered");
        Applied::Set
    }
}

/// The real system: GPUI sets the process identity (and keeps it for its
/// notifications); the registry of the current user holds the
/// registration.
#[derive(Clone, Copy)]
pub struct System<'a>(pub &'a App);

impl std::fmt::Debug for System<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("System")
    }
}

#[cfg(target_os = "windows")]
fn registry_key(id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{id}")
}

impl IdentitySystem for System<'_> {
    fn set_process_identity(&self, id: &str, name: &str) {
        self.0.set_app_identity(id, name);
    }

    #[cfg(target_os = "windows")]
    fn is_registered(&self, id: &str) -> bool {
        windows_registry::CURRENT_USER
            .open(registry_key(id))
            .and_then(|key| key.get_string("DisplayName"))
            .is_ok_and(|name| !name.is_empty())
    }

    #[cfg(target_os = "windows")]
    fn register(&self, id: &str, name: &str) -> bool {
        windows_registry::CURRENT_USER
            .create(registry_key(id))
            .and_then(|key| key.set_string("DisplayName", name))
            .is_ok()
    }

    /// Other systems have nothing to register.
    #[cfg(not(target_os = "windows"))]
    fn is_registered(&self, _id: &str) -> bool {
        true
    }

    #[cfg(not(target_os = "windows"))]
    fn register(&self, _id: &str, _name: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Records the calls; a registry of its own.
    #[derive(Default)]
    struct Fake {
        calls: RefCell<Vec<String>>,
        registered: RefCell<Option<String>>,
        refuse: bool,
    }

    impl IdentitySystem for Fake {
        fn set_process_identity(&self, id: &str, name: &str) {
            self.calls.borrow_mut().push(format!("set {id} {name}"));
        }

        fn is_registered(&self, id: &str) -> bool {
            self.registered.borrow().as_deref() == Some(id)
        }

        fn register(&self, id: &str, name: &str) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("register {id} {name}"));
            if self.refuse {
                return false;
            }
            *self.registered.borrow_mut() = Some(id.to_owned());
            true
        }
    }

    #[test]
    fn the_first_start_sets_and_registers_later_ones_only_set() {
        let system = Fake::default();
        assert_eq!(apply(true, &system), Applied::SetAndRegistered);
        assert_eq!(apply(true, &system), Applied::Set);
        assert_eq!(
            *system.calls.borrow(),
            [
                "set Shellpp.Desktop Shell++",
                "register Shellpp.Desktop Shell++",
                "set Shellpp.Desktop Shell++",
            ]
        );
    }

    #[test]
    fn a_test_run_or_a_demo_sets_and_registers_nothing() {
        let system = Fake::default();
        assert_eq!(apply(false, &system), Applied::Nothing);
        assert!(system.calls.borrow().is_empty());
        assert!(system.registered.borrow().is_none());
    }

    #[test]
    fn a_refused_registration_keeps_the_identity_and_tries_again_next_time() {
        let system = Fake {
            refuse: true,
            ..Fake::default()
        };
        assert_eq!(apply(true, &system), Applied::Set);
        assert_eq!(apply(true, &system), Applied::Set);
        assert_eq!(
            system
                .calls
                .borrow()
                .iter()
                .filter(|call| call.starts_with("register"))
                .count(),
            2
        );
    }
}
