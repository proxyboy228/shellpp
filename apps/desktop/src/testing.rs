//! Test helper: a fresh temporary profile per test with a core opened in it
//! (owner's step 0). Never the permanent profile, never the OS keychain or
//! the OS clipboard (every platform service is in memory), and KDF parameters
//! are the real ones (docs/architecture.md, section 9.4).
//!
//! ```no_run
//! use shellpp_desktop::testing::TestProfile;
//! let opened = TestProfile::new().open_unlocked();
//! // `opened` goes to `AppState::new`; the temporary directory is deleted
//! // when the app state shuts down (or is dropped).
//! ```
//!
//! Compiled into every build (it only uses public core APIs and adds no
//! dependency), so integration tests under `tests/` can use it too.

use secrecy::SecretString;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::dto::LockState;
use shellpp_core::{Core, CoreConfig};

use crate::platform;
use crate::platform::events::event_channel;
use crate::profile::{TempProfile, random_suffix};
use crate::state::OpenedCore;

/// Directory name prefix of test profiles under the OS temporary directory.
pub const TEST_PROFILE_PREFIX: &str = "shellpp-test";

/// Sets up GPUI Kit and the dark theme for a UI test, with motion reduced.
/// GPUI's animations run on the real clock, not on the test executor's, and
/// a test click draws a frame after the pointer move, the press and the
/// release: a dialog still sliding into place on a loaded machine moves its
/// button between them, so the click is lost (D52).
/// Every UI test starts with this instead of `gpui_kit::init` and
/// `theme::install`.
pub fn init_ui(cx: &mut gpui::App) {
    gpui_kit::init(cx);
    crate::theme::install(crate::theme::Appearance::Dark, cx);
    cx.set_reduce_motion(true);
}

/// A master password generated at run time (never a fixed test value).
pub fn generated_password() -> SecretString {
    let suffix = random_suffix().expect("the OS random generator works");
    SecretString::from(format!("Pw-{suffix}-x"))
}

/// A temporary profile with a generated master password.
#[derive(Debug)]
pub struct TestProfile {
    /// `None` once handed over to an [`OpenedCore`].
    profile: Option<TempProfile>,
    path: std::path::PathBuf,
    password: SecretString,
    /// The in-memory platform of the last opened core (clipboard, clock).
    memory: Option<MemoryPlatform>,
}

impl Default for TestProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl TestProfile {
    /// Creates `<temp>/shellpp-test-<random>`.
    pub fn new() -> Self {
        let profile = TempProfile::create(TEST_PROFILE_PREFIX).expect("temporary profile");
        Self {
            path: profile.path().to_path_buf(),
            profile: Some(profile),
            password: generated_password(),
            memory: None,
        }
    }

    /// The profile directory.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The generated master password.
    pub fn password(&self) -> &SecretString {
        &self.password
    }

    /// The in-memory platform of the last [`Self::open`] (clipboard, clock).
    pub fn memory(&self) -> Option<&MemoryPlatform> {
        self.memory.as_ref()
    }

    /// Opens the core in this profile without unlocking (`first_run` on a new
    /// profile). The profile directory stays owned by `self`.
    pub fn open(&mut self) -> OpenedCore {
        let dirs = crate::dirs::AppDirs::under(self.path.clone());
        dirs.create().expect("profile directories");
        let (sink, events) = event_channel();
        let (platform, memory) = platform::in_memory(sink);
        self.memory = Some(memory);
        let core = Core::open(
            CoreConfig {
                data_dir: dirs.data.clone(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            platform,
        )
        .expect("core opens in the test profile");
        OpenedCore {
            core: std::sync::Arc::new(core),
            events,
            extras: Default::default(),
            dirs: Some(dirs),
            temp_profile: None,
        }
    }

    /// Opens the core and unlocks it: the first run with the generated
    /// password on a new profile, an unlock otherwise.
    pub fn open_unlocked(&mut self) -> OpenedCore {
        let opened = self.open();
        unlock_with(&opened, &self.password);
        opened
    }

    /// Like [`Self::open_unlocked`], and hands the profile directory to the
    /// returned value: it is deleted when the app state shuts down or drops.
    pub fn into_unlocked(mut self) -> OpenedCore {
        let mut opened = self.open_unlocked();
        opened.temp_profile = self.profile.take();
        opened
    }

    /// Like [`Self::open`] (first run or locked), handing the profile over;
    /// returns the generated password for the lock screens.
    pub fn into_opened(mut self) -> (OpenedCore, SecretString) {
        let mut opened = self.open();
        opened.temp_profile = self.profile.take();
        (opened, self.password.clone())
    }
}

/// Runs the first run or an unlock with `password` (blocking).
pub fn unlock_with(opened: &OpenedCore, password: &SecretString) {
    match opened.core.lock_status().state {
        LockState::FirstRun => {
            opened
                .core
                .first_run_setup(password.clone())
                .expect("first run in the test profile");
        }
        LockState::Locked => {
            opened
                .core
                .unlock(password.clone())
                .expect("unlock in the test profile");
        }
        LockState::Unlocked => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_fresh_temporary_directories() {
        let mut profile = TestProfile::new();
        let path = profile.path().to_path_buf();
        assert!(path.starts_with(std::env::temp_dir()));
        let permanent = crate::dirs::permanent(
            crate::dirs::OsFamily::current(),
            &crate::dirs::BaseDirs::from_os(),
        );
        if let Ok(permanent) = permanent {
            assert!(!path.starts_with(&permanent.data));
        }
        let opened = profile.open_unlocked();
        assert_eq!(opened.core.lock_status().state, LockState::Unlocked);
        assert!(path.join(shellpp_core::ids::DB_FILE_NAME).is_file());
        drop(opened);
        drop(profile);
        assert!(!path.exists());
    }

    #[test]
    fn handed_over_profiles_live_with_the_opened_core() {
        let profile = TestProfile::new();
        let path = profile.path().to_path_buf();
        let opened = profile.into_unlocked();
        assert!(
            path.is_dir(),
            "the TestProfile is gone, the directory is not"
        );
        drop(opened);
        assert!(!path.exists());
    }

    #[test]
    fn passwords_are_generated() {
        use secrecy::ExposeSecret as _;
        let first = generated_password();
        let second = generated_password();
        assert_ne!(first.expose_secret(), second.expose_secret());
        assert!(first.expose_secret().len() >= 16);
    }
}
