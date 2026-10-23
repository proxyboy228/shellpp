//! The PuTTY sessions of the import dialog (UI 2.5; run 8 decision D140):
//! read from the Windows registry here, turned into hosts by the core
//! (`shellpp_core::import::putty`, docs/architecture.md, section 3).
//!
//! - [`source`]: the override of [`set_source`] (tests and the demo), else
//!   no sessions in a test run (never the user's registry), else the
//!   registry on Windows and none elsewhere.
//! - [`Registry`] reads `HKCU\Software\SimonTatham\PuTTY\Sessions`: every
//!   subkey is a session, its `REG_SZ` and `REG_DWORD` values are kept,
//!   other types are skipped. A missing key means PuTTY never saved one.
//!
//! Nothing here logs session names or values.

use std::sync::Arc;

use gpui::{App, Global};
use shellpp_core::import::putty::{FixedSessions, SessionSource};

/// The source set with [`set_source`].
#[derive(Clone)]
struct Override(Arc<dyn SessionSource>);

impl Global for Override {}

/// Replaces where the dialog reads PuTTY sessions (tests and the demo).
pub fn set_source(source: Arc<dyn SessionSource>, cx: &mut App) {
    cx.set_global(Override(source));
}

/// Where the dialog reads PuTTY sessions.
pub fn source(cx: &App) -> Arc<dyn SessionSource> {
    if let Some(found) = cx.try_global::<Override>() {
        return found.0.clone();
    }
    if crate::dirs::is_test_run() {
        return Arc::new(FixedSessions::default());
    }
    system()
}

#[cfg(windows)]
fn system() -> Arc<dyn SessionSource> {
    Arc::new(Registry)
}

#[cfg(not(windows))]
fn system() -> Arc<dyn SessionSource> {
    Arc::new(FixedSessions::default())
}

/// The saved sessions in the current user's registry.
#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub struct Registry;

#[cfg(windows)]
impl SessionSource for Registry {
    fn sessions(&self) -> std::io::Result<shellpp_core::import::putty::Sessions> {
        use shellpp_core::import::putty::{self, Session, Sessions, Value};
        use windows_registry::Type;

        let root = match windows_registry::CURRENT_USER.open(putty::SESSIONS_KEY) {
            Ok(root) => root,
            Err(error) => {
                let error = std::io::Error::from(error);
                return if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(Sessions::default())
                } else {
                    Err(error)
                };
            }
        };
        let mut out = Sessions::default();
        for name in root.keys()?.take(putty::MAX_SESSIONS) {
            let Ok(key) = root.open(&name) else {
                out.unreadable.push(name);
                continue;
            };
            let Ok(values) = key.values() else {
                out.unreadable.push(name);
                continue;
            };
            let mut session = Session {
                key: name,
                values: Default::default(),
            };
            for (value_name, value) in values {
                let value = match value.ty() {
                    Type::U32 => u32::try_from(value).ok().map(Value::Number),
                    Type::String | Type::ExpandString => String::try_from(value)
                        .ok()
                        .filter(|text| text.chars().count() <= putty::MAX_VALUE_CHARS)
                        .map(Value::Text),
                    _ => None,
                };
                if let Some(value) = value {
                    session.values.insert(value_name, value);
                }
            }
            out.sessions.push(session);
        }
        Ok(out)
    }
}
