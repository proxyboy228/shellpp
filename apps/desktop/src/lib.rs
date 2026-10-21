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
//! - [`shell`]: title bar, sidebar, session tabs, "In development"
//!   placeholder, toasts. [`session_ui`]: the content of a session tab
//!   (connection screen, prompts, host key dialogs, end and drop overlays).
//! - [`hosts`]: the Hosts screen. [`panels`]: the host and group panels.
//!   [`known_hosts`]: the Known Hosts screen (UI 10.1). [`keychain`]: the
//!   Keychain screen (UI 7.1-7.3).
//! - [`lock`]: the first-run and unlock screens. [`settings`]: the
//!   Settings window (Security section).
//! - [`theme`]: `themes/graphite.json` in gpui-kit's theme registry (dark and
//!   light), the token mapping and the colors that are not part of gpui-kit's
//!   `ThemeColor` ([`theme::palette`]).
//! - [`dirs`], [`logging`], [`hardening`], [`platform`]: process setup and
//!   the platform services handed to the core.
//! - [`profile`]: temporary profiles (tests, `--demo`); [`testing`]: the test
//!   helper that opens a core in a fresh temporary profile.
//! - [`terminal`]: the terminal emulator view (alacritty_terminal and our
//!   own GPUI element), fed by the session layer. [`terminal_sidebar`]: the
//!   terminal sidebar of a window (UI 4.2: Tools, Snippets, History,
//!   Appearance).
//! - [`panes`]: the panes of a session tab (UI 4.7): splits, dividers, the
//!   active pane, Restore snapshots of the pane tree.
//! - [`new_tab`]: the new tab page (UI 6), the Ctrl+K search overlay and the
//!   host picker of a new pane.
//! - [`sessions`]: the session layer: SSH sessions and local terminals on a
//!   Tokio runtime of its own (connecting, prompts, host keys, exit and
//!   drop, automatic reconnection, the connection log, flow control).
//! - [`i18n`]: English interface strings. `dev` (feature `dev-tools`): demo data.

// gpui-kit re-exports GPUI. Aliasing the kit as `gpui` lets GPUI's derive
// macros (IntoElement, Action, ...) resolve their `gpui::` paths through it.
extern crate gpui_kit as gpui;

pub mod app;
pub mod data_transfer;
pub mod dates;
#[cfg(feature = "dev-tools")]
pub mod dev;
pub mod dirs;
pub mod forwarding;
pub mod hardening;
pub mod hosts;
pub mod i18n;
pub mod import_sources;
pub mod keychain;
pub mod known_hosts;
pub mod lock;
pub mod logging;
pub mod logs;
pub mod new_tab;
pub mod panels;
pub mod panes;
pub mod platform;
pub mod profile;
pub mod session_ui;
pub mod sessions;
pub mod settings;
pub mod sftp;
pub mod shell;
pub mod snippets;
pub mod state;
pub mod terminal;
pub mod terminal_sidebar;
pub mod testing;
pub mod theme;

/// Environment variable that marks a test run: with `SHELLPP_TEST=1` the data
/// directory must come from `SHELLPP_DATA_DIR` (see [`dirs::resolve_current`]).
pub const ENV_TEST: &str = "SHELLPP_TEST";
