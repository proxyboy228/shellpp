//! Shell++ desktop application (crate `shellpp-desktop`, binary `shellpp`) on
//! GPUI Kit: the window, the screens, the theme and the platform services that
//! `shellpp-core` needs (docs/architecture.md, sections 2.4, 3 and 5).
//!
//! Module map:
//! - [`app`]: startup into a window (1280x800, frameless), the root view
//!   that switches between the lock screens and the shell, shutdown.
//! - [`state`]: [`state::AppState`], the one entity that owns the core facade,
//!   the lock state, the loaded hosts and groups and the selection. Views
//!   read and write records only through it (there is no IPC layer).
//! - [`shell`]: title bar, sidebar, "In development" placeholder, toasts.
//! - [`hosts`]: the Hosts screen. [`panels`]: the host and group panels.
//! - [`lock`]: the first-run and unlock screens. [`settings`]: the
//!   Settings window (Security section).
//! - [`theme`]: `themes/graphite.json` in gpui-kit's theme registry (dark and
//!   light), the token mapping and the colors that are not part of gpui-kit's
//!   `ThemeColor` ([`theme::palette`]).
//! - [`dirs`], [`logging`], [`hardening`], [`platform`]: process setup and
//!   the platform services handed to the core.
//! - [`profile`]: temporary profiles (tests, `--demo`); [`testing`]: the test
//!   helper that opens a core in a fresh temporary profile.
//! - [`i18n`]: English interface strings. `dev` (feature `dev-tools`): demo data.

// gpui-kit re-exports GPUI. Aliasing the kit as `gpui` lets GPUI's derive
// macros (IntoElement, Action, ...) resolve their `gpui::` paths through it.
extern crate gpui_kit as gpui;

pub mod app;
#[cfg(feature = "dev-tools")]
pub mod dev;
pub mod dirs;
pub mod hardening;
pub mod hosts;
pub mod i18n;
pub mod lock;
pub mod logging;
pub mod panels;
pub mod platform;
pub mod profile;
pub mod settings;
pub mod shell;
pub mod state;
pub mod testing;
pub mod theme;

/// Environment variable that marks a test run: with `SHELLPP_TEST=1` the data
/// directory must come from `SHELLPP_DATA_DIR` (see [`dirs::resolve_current`]).
pub const ENV_TEST: &str = "SHELLPP_TEST";
