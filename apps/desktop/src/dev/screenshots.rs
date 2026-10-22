//! `--screenshots <out-dir> [--size WxH[,WxH...]]` (cargo feature
//! `screenshots`, owner's steps 4 and 5.2): one launch of the real app that
//! saves PNG screenshots of **our own window only** for each window size,
//! each screen state of that size ([`states_for`]) and each theme (dark,
//! light), then quits.
//!
//! - Data: a fresh demo profile (300 hosts, 5 groups, see [`super::demo`])
//!   created inside `SHELLPP_DATA_DIR` when that variable is set (the
//!   scripts point it at a temporary folder), otherwise in the OS temporary
//!   directory. The first-run, unlock and empty-state screens get their own
//!   empty profiles next to it ([`demo::open_fresh`]). Every profile keeps
//!   the device key in memory (never the OS keychain) and is deleted at quit.
//! - Capture: GPUI's own renderer draws the window's current scene into an
//!   offscreen texture and reads it back (`Window::render_to_image`, the
//!   DirectX renderer on Windows). Nothing is read from the screen, so other
//!   windows, the desktop or an overlapping window can never appear in a
//!   screenshot, and the window does not have to be fully on screen.
//! - Size: `WxH` is the window content size in logical pixels; the PNG has
//!   the device pixels of the window (`W * scale` x `H * scale`, the scale
//!   factor is printed). One window per size; its minimum size is raised to
//!   `WxH` so that Windows does not clamp a window larger than the display.
//!   Several sizes in one launch (`--size 1920x1200,1280x800,960x600`, what
//!   `scripts/ui-screens.ps1` passes) keep the number of app launches at one.
//! - Names: `<W>x<H>-<theme>-<state>.png`, e.g. `1280x800-dark-root.png`.
//! - States are reached the way a user gets there (clicks, typing, keys on
//!   the elements' ids, see [`ShotState`]); each demo state starts from the
//!   same reset (top level, no panel, no selection, empty search, tiles, no
//!   session tabs, no toasts). The pointer is then moved out of the window so
//!   that no hover state is captured, and the state is checked
//!   ([`check_state`]) before the capture.
//! - Exit code 0 when every PNG was written with the expected size, 1 on any
//!   failure. Each state has [`STATE_TIMEOUT`] per theme and is tried once
//!   more when it fails or runs out of time (D72); a watchdog ends a run
//!   that makes no progress at all for [`WATCHDOG`]. A full run takes far
//!   longer than either: the limits are per state, not per run.
//! - Smoke runs: `SHELLPP_SCREENSHOT_STATES=<name>[,<name>...]` ([`STATES_ENV`],
//!   `scripts/ui-screens.ps1 -States`) captures only those states (their
//!   [`ShotState::name`]s) at the sizes that have them, both themes.

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, InputEvent as _, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollDelta, Size, Window,
    WindowBounds, point, px, size,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use uuid::Uuid;

use super::demo::{self, FreshStage};
use crate::app::{self, RootView};
use crate::hosts::group_card_id;
use crate::hosts::tag_filter::tag_option_id;
use crate::profile::TempProfile;
use crate::shell::{Section, Shell, TopTab};
use crate::state::AppState;
use crate::state::hosts_view::HostsLayout;
use crate::theme::{self, Appearance};

/// One window content size in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

impl WindowSize {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Parses `1920x1200` (also `1920X1200`); both sides 320..=7680.
    pub fn parse(value: &str) -> Option<Self> {
        let (width, height) = value.trim().split_once(['x', 'X'])?;
        let width: u32 = width.trim().parse().ok()?;
        let height: u32 = height.trim().parse().ok()?;
        let valid = 320..=7680;
        (valid.contains(&width) && valid.contains(&height)).then_some(Self { width, height })
    }

    pub fn label(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    fn logical(&self) -> Size<Pixels> {
        size(px(self.width as f32), px(self.height as f32))
    }
}

impl std::fmt::Display for WindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// The large size of the original set.
pub const LARGE_SIZE: WindowSize = WindowSize::new(1920, 1200);
/// The app's default window size: every state is captured at this size.
pub const MAIN_SIZE: WindowSize = WindowSize::new(1280, 800);
/// The app's minimum window size (lead decision 4): the root and the host
/// panel are captured at this size.
pub const MIN_SIZE: WindowSize = WindowSize::new(960, 600);

/// The window sizes of one run (`--size`), in capture order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotSize {
    sizes: Cow<'static, [WindowSize]>,
}

/// Without `--size`: the app's default window size only.
pub const DEFAULT_SIZE: ShotSize = ShotSize {
    sizes: Cow::Borrowed(&[MAIN_SIZE]),
};

/// The full set of `scripts/ui-screens.ps1`.
pub const FULL_SET: ShotSize = ShotSize {
    sizes: Cow::Borrowed(&[LARGE_SIZE, MAIN_SIZE, MIN_SIZE]),
};

/// At most this many sizes in one launch.
const MAX_SIZES: usize = 8;

impl ShotSize {
    /// Parses one size (`1920x1200`) or a comma-separated list of distinct
    /// sizes (`1920x1200,1280x800,960x600`).
    pub fn parse(value: &str) -> Option<Self> {
        let mut sizes = Vec::new();
        for part in value.split(',') {
            let size = WindowSize::parse(part)?;
            if sizes.contains(&size) {
                return None;
            }
            sizes.push(size);
        }
        (!sizes.is_empty() && sizes.len() <= MAX_SIZES).then_some(Self {
            sizes: Cow::Owned(sizes),
        })
    }

    pub fn sizes(&self) -> &[WindowSize] {
        &self.sizes
    }
}

impl std::fmt::Display for ShotSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let labels: Vec<String> = self.sizes.iter().map(WindowSize::label).collect();
        f.write_str(&labels.join(","))
    }
}

/// A run that makes no progress (no state begun) for this long is ended
/// with exit code 3: the app's thread is stuck, so no timeout of its own
/// can fire. Longer than a state's two attempts.
pub const WATCHDOG: Duration = Duration::from_secs(300);
/// One attempt at a state in one theme (reaching it, settling, checking).
/// The slowest state (a connection through the test server's 45 s timeout)
/// stays well under it.
pub const STATE_TIMEOUT: Duration = Duration::from_secs(120);

/// When the run last made progress (ms since the Unix epoch) and the state
/// it was at, for the watchdog thread.
static PROGRESS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CURRENT: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The run is at `state` now (the watchdog counts from here).
fn mark_progress(state: Option<ShotState>) {
    PROGRESS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut current) = CURRENT.lock() {
        *current = state.map(ShotState::name);
    }
}
/// Settling time after a state change before the frame is captured (covers
/// the 180 ms panel motion of the design system and asset loading).
const SETTLE: Duration = Duration::from_millis(700);
/// Device-pixel tolerance when comparing the PNG with the requested size.
const SIZE_TOLERANCE: i64 = 2;

/// Names of the only states to capture, comma-separated (smoke runs);
/// unset or empty: every state of each size.
pub const STATES_ENV: &str = "SHELLPP_SCREENSHOT_STATES";

/// The text typed for the address states (owner's step 5.2).
pub const QUICK_CONNECT_TEXT: &str = "root@10.0.0.5:2222";
/// A search that matches some demo hosts (the "web" role).
pub const SEARCH_WITH_RESULTS: &str = "web";
/// A search that matches nothing in the demo data.
pub const SEARCH_WITHOUT_RESULTS: &str = "no-such-host";
/// The tags checked in the tag filter state (the first demo host has both).
pub const TAG_FILTER_EXAMPLE: [&str; 2] = ["prod", "linux"];
/// The tags checked in the tag chips state: every tag of the first demo
/// host, more than fit next to the host panel.
pub const TAG_CHIPS_EXAMPLE: [&str; 4] = demo::FIRST_HOST_TAGS;
/// The search of the new tab page state: the test server's hosts and the
/// open tab to it.
pub const NEW_TAB_SEARCH: &str = "Test";

/// The command-line options of a screenshot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenshotOptions {
    pub out_dir: PathBuf,
    pub size: ShotSize,
}

/// The themes captured, in order.
pub const THEMES: [(Appearance, &str); 2] =
    [(Appearance::Dark, "dark"), (Appearance::Light, "light")];

/// The screen states captured, each in both themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotState {
    /// The Hosts screen at the top level (Groups and Hosts sections).
    Root,
    /// Inside the first group (breadcrumbs, its subgroups and hosts).
    Group,
    /// The first top-level host selected, its panel open.
    HostPanel,
    /// The context menu of the first host card (right click).
    ContextMenu,
    /// [`QUICK_CONNECT_TEXT`] in the search bar, its Connect button active.
    SearchAddress,
    /// A search with results ([`SEARCH_WITH_RESULTS`]).
    SearchResults,
    /// A search without results ([`SEARCH_WITHOUT_RESULTS`]).
    SearchNoResults,
    /// Three host cards selected (Ctrl+click on each).
    ThreeSelected,
    /// The Hosts screen of an empty profile (UI 2.3, empty state).
    Empty,
    /// The list view (UI 2.2 view toggle).
    ListView,
    /// New host: the empty panel, the focus in Address.
    NewHost,
    // Stage 3 (night plan, items 5-6): `transfer_shot`.
    /// Export Data as it opens: "Encrypt with a password" on (the
    /// default), the password and its repeat (UI 1.4).
    ExportData,
    /// Export Data with "Encrypt with a password" off: the clear-text
    /// warning and "I understand" (UI 1.4).
    ExportDataPlain,
    // Stage 4 (run 3, plan section 7): `sftp_shot`.
    /// The SFTP tab: a local folder on the left, the empty panel on the right.
    SftpLocal,
    /// The empty panel's host selection.
    SftpPicker,
    /// The transfer queue under the name conflict dialog.
    SftpQueue,
    /// The Actions menu of the local panel.
    SftpActions,
    /// The Delete confirmation.
    SftpDelete,
    /// The Edit Permissions dialog.
    SftpPermissions,
    /// Export to host over the first key's panel.
    KeychainExport,
    /// A key with an expired certificate: its block and warning.
    KeychainCertificate,
    /// Port Forwarding without rules (UI 8).
    ForwardingEmpty,
    /// Rule cards: running, stopped, an error and reconnecting.
    ForwardingCards,
    /// The full form of a Remote rule on all interfaces, with its status.
    ForwardingForm,
    /// The wizard's first step: a Local rule with its diagram (UI 8.3).
    ForwardingWizardType,
    /// The wizard's first step for Remote.
    ForwardingWizardRemote,
    /// The wizard's first step for Dynamic.
    ForwardingWizardDynamic,
    /// The wizard's host step.
    ForwardingWizardHost,
    /// The wizard's listen step.
    ForwardingWizardListen,
    /// The wizard's destination step.
    ForwardingWizardDest,
    /// The wizard's last step: the rule in one line.
    ForwardingWizardSummary,
    /// The Snippets screen: a package and snippet cards (UI 9).
    Snippets,
    /// A snippet's panel with its targets.
    SnippetPanel,
    /// The values of a run's `{{name}}` variables.
    SnippetVariables,
    /// The confirmation of a run on several hosts.
    SnippetConfirm,
    /// The results table of a run.
    SnippetResults,
    /// A snippet's panel with changes not saved and the question on
    /// leaving it (D68).
    SnippetLeave,
    /// Shell History: every host's commands (UI 9.1).
    ShellHistory,
    /// The Logs screen with an entry's details (UI 11).
    Logs,
    /// The Logs section of the Settings window (UI 12.6).
    SettingsLogs,
    /// The Shortcuts section of the Settings window (UI 12.5).
    SettingsShortcuts,
    /// Export Data with the format "OpenSSH files" (D43): key files and
    /// their passphrase.
    ExportOpenSsh,
    /// The summary of Import Data after a plain export of the demo itself:
    /// every record already here.
    ImportData,
    /// The import dialog's preview of an OpenSSH config (UI 2.5): new
    /// hosts, a key file with a passphrase, a missing one, an error.
    ImportSshConfig,
    /// A quick connect from the search bar to the SSH test server: once the
    /// session runs, the offer to save the host ("Save host?", UI 2.1).
    QuickConnectToast,
    /// The tag filter dropdown open (UI 2.2).
    TagFilter,
    /// The example tags on and the host panel open: the narrower action bar
    /// collapses their chips into "+N" (UI 2.2).
    TagChips,
    /// The bottom of the host panel with "Show more" expanded (UI 3.1).
    ShowMore,
    /// Agent Forwarding on in "Show more": the warning and both options
    /// (UI 3.1, D60).
    AgentForwarding,
    /// The first-run screen (UI 13.1).
    FirstRun,
    /// The unlock screen (UI 13.2).
    Unlock,
    /// The first connection to the test server: the host key dialog of
    /// UI 10.2 (TOFU) over the connection screen.
    TerminalTofu,
    /// An SSH session after `ls --color` and `ls -l --color` (OpenSSH; the
    /// russh test server's shell prints its color sample instead).
    TerminalLsColor,
    /// `htop` in an SSH session: the alternate screen (OpenSSH; the russh
    /// test server's `alt-screen` instead).
    TerminalHtop,
    /// Run 4, plan 9.2 (docs/compatibility.md, section 2): tmux with two
    /// panes (`ls --color` in one); OpenSSH only, like the other programs.
    TerminalTmux,
    /// tmux after the app's pane is split beside it: drawn again for half
    /// the columns, its status line on the last row.
    TerminalTmuxResize,
    /// Midnight Commander.
    TerminalMc,
    /// vim with a shell script (syntax colors).
    TerminalVim,
    /// nano with the same script.
    TerminalNano,
    /// less with a long text (`seq 1 1000 | less -M`).
    TerminalLess,
    /// emacs in the terminal (`emacs -nw`).
    TerminalEmacs,
    /// vttest: the first screen of its cursor movement test.
    TerminalVttest,
    /// Cyrillic, emoji, wide and combining characters, box drawing.
    TerminalUnicode,
    /// A dropped connection: the output stays, the reason with Reconnect
    /// and Close over it (UI 4.6; Autoreconnect off for the picture, so no
    /// attempt replaces the overlay).
    TerminalDrop,
    /// A local terminal (Windows PowerShell on Windows).
    TerminalLocal,
    /// Four session tabs (three SSH, one local), the first one active.
    TerminalTabs,
    /// The test server's host key changed: the blocking warning of UI 10.2
    /// with both fingerprints. Last: it rotates the server's host key.
    TerminalHostKeyChanged,
    // Stage 2b (owner's step 5.1); the session states are in
    // `terminal::stage2b`, Known Hosts in `known_hosts_shot`.
    /// The Known Hosts screen (UI 10.1) after an import of a temporary
    /// known_hosts file (plain, `[host]:port`, hashed and revoked entries),
    /// the details panel of the first entry open.
    KnownHosts,
    /// A tab while it connects (the test server holds new connections): the
    /// connecting screen and the tab's `--warning` dot (UI 1.1).
    TerminalConnecting,
    /// A tab with four panes (2 x 2) to the test server, split with
    /// Ctrl+Shift+D / Ctrl+Shift+E and Enter in the new pane (UI 4.7).
    TerminalSplits,
    /// A tab right after Ctrl+Shift+D: the new pane's host list with the
    /// split pane's host first (UI 4.7).
    SplitPicker,
    /// The tab menu (right click) of a tab with two panes (UI 4.4).
    TabMenu,
    /// The new tab page ("+", UI 6) next to an open session tab.
    NewTab,
    /// The new tab page with a search ([`NEW_TAB_SEARCH`]).
    NewTabSearch,
    /// The terminal sidebar's Tools (UI 4.2): a search with its matches
    /// highlighted (Ctrl+Shift+F).
    SidebarTools,
    /// The terminal sidebar's Snippets placeholder.
    SidebarSnippets,
    /// The terminal sidebar's History: commands typed in the session and
    /// the hint card.
    SidebarHistory,
    /// The terminal sidebar's Appearance: the theme list.
    SidebarAppearance,
    /// The terminal sidebar's Appearance: the Font page.
    SidebarFont,
    // Stage 3 (night plan, items 1-4): `keychain_shot`.
    /// The Keychain screen (UI 7.2) with the demo keys and identities.
    Keychain,
    /// The key panel of the first key (UI 7.3), the private key hidden.
    KeychainPanel,
    /// The Generate key form (UI 7.1) with a label typed.
    KeychainGenerate,
    /// The import form with a text that is not a private key: the type
    /// "unknown" with its reason (UI 7.2).
    KeychainImportUnknown,
    /// The Keychain of an empty profile (UI 7.2, empty state).
    KeychainEmpty,
}

/// Which app state a screen state is shown on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    /// The demo profile (300 hosts), shared by the demo states.
    Demo,
    /// A fresh empty profile at this lock stage, in its own window.
    Fresh(FreshStage),
    /// The demo profile with session tabs to the SSH test server
    /// ([`terminal`]).
    Terminal,
}

impl ShotState {
    /// The original three states (every size but the minimum).
    pub const BASE: [Self; 3] = [Self::Root, Self::Group, Self::HostPanel];
    /// Every state, in capture order (the terminal states last: they run
    /// against the SSH test server, the host key change last of all).
    pub const ALL: [Self; 84] = [
        Self::Root,
        Self::Group,
        Self::HostPanel,
        Self::ContextMenu,
        Self::SearchAddress,
        Self::SearchResults,
        Self::SearchNoResults,
        Self::ThreeSelected,
        Self::Empty,
        Self::ListView,
        Self::NewHost,
        Self::ExportData,
        Self::ExportDataPlain,
        Self::SftpLocal,
        Self::SftpPicker,
        Self::SftpQueue,
        Self::SftpActions,
        Self::SftpDelete,
        Self::SftpPermissions,
        Self::KeychainExport,
        Self::KeychainCertificate,
        Self::ExportOpenSsh,
        Self::ImportData,
        Self::ImportSshConfig,
        Self::ForwardingEmpty,
        Self::ForwardingCards,
        Self::ForwardingForm,
        Self::ForwardingWizardType,
        Self::ForwardingWizardRemote,
        Self::ForwardingWizardDynamic,
        Self::ForwardingWizardHost,
        Self::ForwardingWizardListen,
        Self::ForwardingWizardDest,
        Self::ForwardingWizardSummary,
        Self::Snippets,
        Self::SnippetPanel,
        Self::SnippetVariables,
        Self::SnippetConfirm,
        Self::SnippetResults,
        Self::SnippetLeave,
        Self::ShellHistory,
        Self::Logs,
        Self::SettingsLogs,
        Self::SettingsShortcuts,
        Self::TagFilter,
        Self::TagChips,
        Self::ShowMore,
        Self::AgentForwarding,
        Self::KnownHosts,
        Self::Keychain,
        Self::KeychainPanel,
        Self::KeychainGenerate,
        Self::KeychainImportUnknown,
        Self::KeychainEmpty,
        Self::FirstRun,
        Self::Unlock,
        Self::TerminalTofu,
        Self::TerminalLsColor,
        Self::TerminalHtop,
        Self::TerminalDrop,
        Self::TerminalLocal,
        Self::TerminalTabs,
        Self::TerminalConnecting,
        Self::TerminalSplits,
        Self::SplitPicker,
        Self::TabMenu,
        Self::NewTab,
        Self::NewTabSearch,
        Self::SidebarTools,
        Self::SidebarSnippets,
        Self::SidebarHistory,
        Self::SidebarAppearance,
        Self::SidebarFont,
        Self::QuickConnectToast,
        Self::TerminalTmux,
        Self::TerminalTmuxResize,
        Self::TerminalMc,
        Self::TerminalVim,
        Self::TerminalNano,
        Self::TerminalLess,
        Self::TerminalEmacs,
        Self::TerminalVttest,
        Self::TerminalUnicode,
        Self::TerminalHostKeyChanged,
    ];
    /// The states with sessions to the SSH test server: the terminal states
    /// (owner's stage 2a and 2b, step 5.1) and the offer to save a
    /// quick-connect host, which needs a session that connected.
    pub const TERMINAL: [Self; 28] = [
        Self::TerminalTofu,
        Self::TerminalLsColor,
        Self::TerminalHtop,
        Self::TerminalDrop,
        Self::TerminalLocal,
        Self::TerminalTabs,
        Self::TerminalConnecting,
        Self::TerminalSplits,
        Self::SplitPicker,
        Self::TabMenu,
        Self::NewTab,
        Self::NewTabSearch,
        Self::SidebarTools,
        Self::SidebarSnippets,
        Self::SidebarHistory,
        Self::SidebarAppearance,
        Self::SidebarFont,
        Self::QuickConnectToast,
        Self::TerminalTmux,
        Self::TerminalTmuxResize,
        Self::TerminalMc,
        Self::TerminalVim,
        Self::TerminalNano,
        Self::TerminalLess,
        Self::TerminalEmacs,
        Self::TerminalVttest,
        Self::TerminalUnicode,
        Self::TerminalHostKeyChanged,
    ];
    /// The states of stage 2b (owner's step 5.1) that are new in this run;
    /// `terminal-htop` (light: after the minimum contrast change) and
    /// `terminal-drop` (with the tab's dot) belong to the list too.
    pub const STAGE_2B: [Self; 12] = [
        Self::KnownHosts,
        Self::TerminalConnecting,
        Self::TerminalSplits,
        Self::SplitPicker,
        Self::TabMenu,
        Self::NewTab,
        Self::NewTabSearch,
        Self::SidebarTools,
        Self::SidebarSnippets,
        Self::SidebarHistory,
        Self::SidebarAppearance,
        Self::SidebarFont,
    ];
    /// The Keychain states of stage 3 (night plan, items 1-4).
    pub const STAGE_3: [Self; 5] = [
        Self::Keychain,
        Self::KeychainPanel,
        Self::KeychainGenerate,
        Self::KeychainImportUnknown,
        Self::KeychainEmpty,
    ];
    /// The states at the minimum window size: the Hosts screen without and
    /// with the details panel, and the tallest lock screen.
    pub const MINIMUM: [Self; 3] = [Self::Root, Self::HostPanel, Self::FirstRun];

    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Group => "group",
            Self::HostPanel => "host-panel",
            Self::ContextMenu => "context-menu",
            Self::SearchAddress => "search-address",
            Self::SearchResults => "search-results",
            Self::SearchNoResults => "search-no-results",
            Self::ThreeSelected => "three-selected",
            Self::Empty => "empty",
            Self::ListView => "list-view",
            Self::NewHost => "new-host",
            Self::ExportData => "export-data",
            Self::ExportDataPlain => "export-data-plain",
            Self::SftpLocal => "sftp-local",
            Self::SftpPicker => "sftp-picker",
            Self::SftpQueue => "sftp-queue",
            Self::SftpActions => "sftp-actions",
            Self::SftpDelete => "sftp-delete",
            Self::SftpPermissions => "sftp-permissions",
            Self::KeychainExport => "keychain-export",
            Self::KeychainCertificate => "keychain-certificate",
            Self::ForwardingEmpty => "forwarding-empty",
            Self::ForwardingCards => "forwarding-cards",
            Self::ForwardingForm => "forwarding-form",
            Self::ForwardingWizardType => "forwarding-wizard-type",
            Self::ForwardingWizardRemote => "forwarding-wizard-remote",
            Self::ForwardingWizardDynamic => "forwarding-wizard-dynamic",
            Self::ForwardingWizardHost => "forwarding-wizard-host",
            Self::ForwardingWizardListen => "forwarding-wizard-listen",
            Self::ForwardingWizardDest => "forwarding-wizard-dest",
            Self::ForwardingWizardSummary => "forwarding-wizard-summary",
            Self::Snippets => "snippets",
            Self::SnippetPanel => "snippet-panel",
            Self::SnippetVariables => "snippet-variables",
            Self::SnippetConfirm => "snippet-confirm",
            Self::SnippetLeave => "snippet-leave",
            Self::SnippetResults => "snippet-results",
            Self::ShellHistory => "shell-history",
            Self::Logs => "logs",
            Self::SettingsLogs => "settings-logs",
            Self::SettingsShortcuts => "settings-shortcuts",
            Self::ExportOpenSsh => "export-openssh",
            Self::ImportData => "import-data",
            Self::ImportSshConfig => "import-ssh-config",
            Self::QuickConnectToast => "quick-connect-toast",
            Self::TagFilter => "tag-filter",
            Self::TagChips => "tag-chips",
            Self::ShowMore => "show-more",
            Self::AgentForwarding => "agent-forwarding",
            Self::FirstRun => "first-run",
            Self::Unlock => "unlock",
            Self::TerminalTofu => "terminal-tofu",
            Self::TerminalLsColor => "terminal-ls-color",
            Self::TerminalHtop => "terminal-htop",
            Self::TerminalTmux => "terminal-tmux",
            Self::TerminalTmuxResize => "terminal-tmux-resize",
            Self::TerminalMc => "terminal-mc",
            Self::TerminalVim => "terminal-vim",
            Self::TerminalNano => "terminal-nano",
            Self::TerminalLess => "terminal-less",
            Self::TerminalEmacs => "terminal-emacs",
            Self::TerminalVttest => "terminal-vttest",
            Self::TerminalUnicode => "terminal-unicode",
            Self::TerminalDrop => "terminal-drop",
            Self::TerminalLocal => "terminal-local",
            Self::TerminalTabs => "terminal-tabs",
            Self::TerminalHostKeyChanged => "terminal-host-key-changed",
            Self::KnownHosts => "known-hosts",
            Self::TerminalConnecting => "terminal-connecting",
            Self::TerminalSplits => "terminal-splits",
            Self::SplitPicker => "split-picker",
            Self::TabMenu => "tab-menu",
            Self::NewTab => "new-tab",
            Self::NewTabSearch => "new-tab-search",
            Self::SidebarTools => "sidebar-tools",
            Self::SidebarSnippets => "sidebar-snippets",
            Self::SidebarHistory => "sidebar-history",
            Self::SidebarAppearance => "sidebar-appearance",
            Self::SidebarFont => "sidebar-font",
            Self::Keychain => "keychain",
            Self::KeychainPanel => "keychain-key-panel",
            Self::KeychainGenerate => "keychain-generate",
            Self::KeychainImportUnknown => "keychain-import-unknown",
            Self::KeychainEmpty => "keychain-empty",
        }
    }

    /// The state named `name`: [`Self::name`], or the file label of a state
    /// on the russh test server ([`Self::file_label`]).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.name() == name || state.file_label(false) == name)
    }

    /// The programs of plan 9.2: only on the OpenSSH test image, left out
    /// of a run on the russh test server.
    pub fn openssh_only(self) -> bool {
        terminal::PROGRAM_STATES.contains(&self)
    }

    /// The name in file names: [`Self::name`], except the two states whose
    /// content differs on the russh test server (`real_shell` false: no
    /// bash, no `ls`, no `htop`).
    pub fn file_label(self, real_shell: bool) -> &'static str {
        match self {
            Self::TerminalLsColor if !real_shell => "terminal-colors",
            Self::TerminalHtop if !real_shell => "terminal-alt-screen",
            _ => self.name(),
        }
    }

    fn scene(self) -> Scene {
        match self {
            Self::Empty | Self::KeychainEmpty => Scene::Fresh(FreshStage::Unlocked),
            Self::FirstRun => Scene::Fresh(FreshStage::FirstRun),
            Self::Unlock => Scene::Fresh(FreshStage::Locked),
            Self::TerminalTofu
            | Self::TerminalLsColor
            | Self::TerminalHtop
            | Self::TerminalTmux
            | Self::TerminalTmuxResize
            | Self::TerminalMc
            | Self::TerminalVim
            | Self::TerminalNano
            | Self::TerminalLess
            | Self::TerminalEmacs
            | Self::TerminalVttest
            | Self::TerminalUnicode
            | Self::TerminalDrop
            | Self::TerminalLocal
            | Self::TerminalTabs
            | Self::QuickConnectToast
            | Self::TerminalHostKeyChanged
            | Self::TerminalConnecting
            | Self::TerminalSplits
            | Self::SplitPicker
            | Self::TabMenu
            | Self::NewTab
            | Self::NewTabSearch
            | Self::SidebarTools
            | Self::SidebarSnippets
            | Self::SidebarHistory
            | Self::SidebarAppearance
            | Self::SidebarFont => Scene::Terminal,
            _ => Scene::Demo,
        }
    }
}

/// The states captured at `size`: every state at the app's default size,
/// the root, the host panel and the first run at the minimum size, the
/// original three at any other size.
pub fn states_for(size: WindowSize) -> &'static [ShotState] {
    if size == MAIN_SIZE {
        &ShotState::ALL
    } else if size == MIN_SIZE {
        &ShotState::MINIMUM
    } else {
        &ShotState::BASE
    }
}

/// The states of [`STATES_ENV`]: `None` for every state, or the listed
/// ones (distinct, known names).
pub fn parse_states(value: &str) -> Result<Option<Vec<ShotState>>, String> {
    let names: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        return Ok(None);
    }
    let mut states = Vec::new();
    for name in names {
        let state = ShotState::from_name(name).ok_or_else(|| format!("unknown state {name}"))?;
        if !states.contains(&state) {
            states.push(state);
        }
    }
    Ok(Some(states))
}

/// The states captured at `size`: [`states_for`], only those of `only`
/// when given (in capture order).
pub fn selected_states(size: WindowSize, only: Option<&[ShotState]>) -> Vec<ShotState> {
    states_for(size)
        .iter()
        .copied()
        .filter(|state| only.is_none_or(|only| only.contains(state)))
        .collect()
}

/// `<W>x<H>-<theme>-<state>.png` (the terminal states as on OpenSSH).
pub fn file_name(size: WindowSize, theme: &str, state: ShotState) -> String {
    file_name_on(size, theme, state, true)
}

/// [`file_name`] for a run on OpenSSH (`real_shell`) or on the russh test
/// server ([`ShotState::file_label`]).
pub fn file_name_on(size: WindowSize, theme: &str, state: ShotState, real_shell: bool) -> String {
    format!(
        "{}-{theme}-{}.png",
        size.label(),
        state.file_label(real_shell)
    )
}

/// Every file a run on OpenSSH writes, in capture order (size, state,
/// theme).
pub fn expected_files(sizes: &ShotSize) -> Vec<String> {
    expected_files_on(sizes, true)
}

/// [`expected_files`] for a run on OpenSSH (`real_shell`) or on the russh
/// test server.
pub fn expected_files_on(sizes: &ShotSize, real_shell: bool) -> Vec<String> {
    let mut files = Vec::new();
    for size in sizes.sizes() {
        for state in states_for(*size) {
            if !real_shell && state.openssh_only() {
                continue;
            }
            for (_, theme) in THEMES {
                files.push(file_name_on(*size, theme, *state, real_shell));
            }
        }
    }
    files
}

/// Why a screenshot run failed.
#[derive(Debug, thiserror::Error)]
pub enum ScreenshotError {
    #[error("the output directory could not be created: {0}")]
    OutDir(#[source] std::io::Error),
    #[error("the demo profile could not be created: {0}")]
    Profile(#[source] std::io::Error),
    #[error("the demo setup failed: {0}")]
    Demo(#[from] demo::DemoError),
    #[error("the main window did not open")]
    Window,
    #[error("the window closed during the run")]
    WindowClosed,
    #[error("{0}: the state was not reached")]
    State(String),
    #[error("{file}: rendering failed: {message}")]
    Render { file: String, message: String },
    #[error("{file}: saving failed: {message}")]
    Save { file: String, message: String },
    #[error(
        "{file}: got {got_width}x{got_height} device pixels, expected {want_width}x{want_height}"
    )]
    Size {
        file: String,
        got_width: u32,
        got_height: u32,
        want_width: u32,
        want_height: u32,
    },
    #[error("the run ended before all screenshots were taken")]
    Incomplete,
    #[error("the SSH test server: {0}")]
    TestServer(String),
    #[error("{STATES_ENV}: {0}")]
    States(String),
}

impl ScreenshotError {
    fn state(state: ShotState, reason: &str) -> Self {
        Self::State(format!("{}: {reason}", state.name()))
    }
}

/// The result of a run: every saved screenshot, or every failure.
type Outcome = Result<Vec<Shot>, Vec<ScreenshotError>>;

/// One saved screenshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// The folder for the run's profiles: `SHELLPP_DATA_DIR` when set (a
/// temporary folder chosen by the caller), else the OS temporary folder.
fn profiles_parent() -> Result<PathBuf, ScreenshotError> {
    match std::env::var_os(shellpp_core::ids::ENV_DATA_DIR) {
        Some(parent) if !parent.is_empty() => {
            let parent = PathBuf::from(parent);
            crate::dirs::create_private_dir(&parent).map_err(ScreenshotError::Profile)?;
            Ok(parent)
        }
        _ => Ok(std::env::temp_dir()),
    }
}

/// A new temporary profile `<parent>/shellpp-demo-<random>`.
fn create_profile(parent: &Path) -> Result<TempProfile, ScreenshotError> {
    demo::create_profile_in(parent).map_err(|error| match error {
        demo::DemoError::Profile(io) => ScreenshotError::Profile(io),
        other => ScreenshotError::Demo(other),
    })
}

#[allow(clippy::print_stdout)]
fn say(line: &str) {
    println!("{line}");
}

#[allow(clippy::print_stderr)]
fn complain(line: &str) {
    eprintln!("shellpp --screenshots: {line}");
}

/// Runs the app once, takes all screenshots and quits. Returns the process
/// exit code (0 success, 1 failure).
pub fn run(options: ScreenshotOptions) -> std::process::ExitCode {
    match run_inner(options) {
        Ok(shots) => {
            say(&format!("Saved {} screenshots.", shots.len()));
            std::process::ExitCode::SUCCESS
        }
        Err(errors) => {
            for error in &errors {
                complain(&error.to_string());
            }
            std::process::ExitCode::from(1)
        }
    }
}

fn start_watchdog(profile_path: PathBuf) {
    mark_progress(None);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let idle = now_ms().saturating_sub(PROGRESS.load(std::sync::atomic::Ordering::Relaxed));
            if u128::from(idle) < WATCHDOG.as_millis() {
                continue;
            }
            let at = CURRENT.lock().ok().and_then(|current| *current);
            complain(&format!(
                "no progress for {} s (at {}), giving up",
                WATCHDOG.as_secs(),
                at.unwrap_or("the start")
            ));
            // Best effort: the database may still be open.
            let _ = std::fs::remove_dir_all(&profile_path);
            std::process::exit(3);
        }
    });
}

fn run_inner(options: ScreenshotOptions) -> Outcome {
    let only = parse_states(&std::env::var(STATES_ENV).unwrap_or_default())
        .map_err(|error| vec![ScreenshotError::States(error)])?;
    if let Some(only) = &only {
        let names: Vec<&str> = only.iter().map(|state| state.name()).collect();
        say(&format!("States: {} ({STATES_ENV})", names.join(", ")));
    }
    std::fs::create_dir_all(&options.out_dir)
        .map_err(|error| vec![ScreenshotError::OutDir(error)])?;
    let out_dir = std::path::absolute(&options.out_dir).unwrap_or(options.out_dir.clone());
    let parent = profiles_parent().map_err(|error| vec![error])?;
    let profile = create_profile(&parent).map_err(|error| vec![error])?;
    say(&format!("Demo profile: {}", profile.path().display()));
    start_watchdog(profile.path().to_path_buf());
    crate::logging::init(&profile.dirs().logs);
    let (opened, summary) = demo::open(profile).map_err(|error| vec![error.into()])?;
    tracing::info!(hosts = summary.hosts, "screenshot run");
    let first_group = summary.groups.first().copied();

    let outcome: Rc<RefCell<Option<Outcome>>> = Rc::new(RefCell::new(None));
    let outcome_in_app = outcome.clone();
    let sizes = options.size;
    let server_json = terminal::server_json();
    let real_shell = server_json.is_some();
    if sizes.sizes().iter().any(|size| {
        selected_states(*size, only.as_deref())
            .iter()
            .any(|state| state.scene() == Scene::Terminal)
    }) {
        say(&format!(
            "Terminal states: server = {}",
            terminal::server_label(real_shell)
        ));
    }

    gpui_kit::application()
        .with_assets(app::assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            app::root::bind_keys(cx);
            crate::terminal::init(cx);
            // Never the user's ~/.ssh in the Keychain either.
            crate::keychain::set_ssh_folder(None, cx);
            let state = cx.new(|cx| AppState::new(opened, cx));
            // The session tabs of the terminal states: never the user's
            // ~/.ssh or agent (the test server's hosts have a password).
            let sessions = crate::sessions::ui::install(
                &state,
                crate::sessions::ManagerOptions::with_keys(crate::sessions::KeySources::none()),
                cx,
            );
            if !sessions {
                complain("the session layer did not start; the terminal states will fail");
            }

            let quit_state = state.clone();
            cx.on_app_quit(move |cx| {
                // Sessions end (their connection log entries need the core)
                // and let go of the core before it closes.
                crate::sessions::ui::shutdown(cx);
                if cx.has_global::<crate::sessions::ui::Sessions>() {
                    drop(cx.remove_global::<crate::sessions::ui::Sessions>());
                }
                quit_state.update(cx, |state, _| state.shutdown());
                async {}
            })
            .detach();

            cx.activate(true);
            let run = Run {
                demo: state,
                first_group,
                parent,
                out_dir,
                server_json,
                real_shell,
                rig: RefCell::new(None),
                only,
            };
            cx.spawn(async move |cx: &mut AsyncApp| {
                let result = run.capture_all(cx, &sizes).await;
                *outcome_in_app.borrow_mut() = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });

    let result = outcome.borrow_mut().take();
    result.unwrap_or_else(|| Err(vec![ScreenshotError::Incomplete]))
}

/// The options of a screenshot window of `size`.
fn window_options(size: WindowSize, cx: &App) -> gpui::WindowOptions {
    let mut options = app::main_window_options(cx);
    options.window_bounds = Some(WindowBounds::centered(size.logical(), cx));
    // Lifts Windows' maximum track size (the display size) to the requested
    // size, so a 1920x1200 window opens on a smaller display.
    options.window_min_size = Some(size.logical());
    options
}

/// Opens a window of `size` over `state`.
fn open_window(
    cx: &mut AsyncApp,
    state: &Entity<AppState>,
    size: WindowSize,
) -> Result<(AnyWindowHandle, Entity<RootView>), ScreenshotError> {
    cx.update(|cx| {
        let options = window_options(size, cx);
        app::open_window_with(state.clone(), options, cx)
    })
    .map_err(|_| ScreenshotError::Window)
}

fn close_window(cx: &mut AsyncApp, handle: AnyWindowHandle) {
    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
}

/// One screenshot run inside the app.
struct Run {
    demo: Entity<AppState>,
    first_group: Option<Uuid>,
    /// Where the fresh profiles go (next to the demo profile).
    parent: PathBuf,
    out_dir: PathBuf,
    /// `server.json` of the OpenSSH test server, else the russh test server
    /// runs as a child process ([`terminal::server_json`]).
    server_json: Option<PathBuf>,
    /// The terminal states run on OpenSSH (bash, `ls`, `htop`).
    real_shell: bool,
    /// The test server and its hosts, from the first terminal state of a
    /// window size to the end of its states.
    rig: RefCell<Option<Rc<terminal::Rig>>>,
    /// The only states to capture ([`STATES_ENV`]); `None`: every state.
    only: Option<Vec<ShotState>>,
}

impl Run {
    /// Takes every screenshot; continues after a failed one and reports all.
    async fn capture_all(&self, cx: &mut AsyncApp, sizes: &ShotSize) -> Outcome {
        let mut shots = Vec::new();
        let mut errors = Vec::new();
        let mut previous: Option<AnyWindowHandle> = None;
        for size in sizes.sizes().iter().copied() {
            let mut states = selected_states(size, self.only.as_deref());
            // The programs of plan 9.2 need the OpenSSH test image.
            if !self.real_shell && states.iter().any(|state| state.openssh_only()) {
                say("Terminal programs (tmux, mc, vim, ...): left out on the russh test server");
                states.retain(|state| !state.openssh_only());
            }
            if states.is_empty() {
                continue;
            }
            // The next window opens before the previous one closes: on some
            // platforms closing the last window ends the app.
            let opened = open_window(cx, &self.demo, size);
            if let Some(previous) = previous.take() {
                close_window(cx, previous);
            }
            let (handle, root) = match opened {
                Ok(window) => window,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };
            previous = Some(handle);
            if let Err(error) = prepare_window(cx, handle, size).await {
                errors.push(error);
                continue;
            }
            for state in states {
                let result = match state.scene() {
                    Scene::Demo | Scene::Terminal => {
                        self.capture_demo_state(cx, handle, &root, size, state)
                            .await
                    }
                    Scene::Fresh(stage) => self.capture_fresh_state(cx, size, state, stage).await,
                };
                match result {
                    Ok(mut saved) => shots.append(&mut saved),
                    Err(mut failed) => errors.append(&mut failed),
                }
            }
            // The session tabs close, the test server's hosts and settings
            // go, the server stops: the next size starts from the demo data.
            if let Err(error) = self.finish_terminal(cx, handle, &root).await {
                errors.push(error);
            }
        }
        if let Some(previous) = previous {
            close_window(cx, previous);
        }
        if errors.is_empty() {
            Ok(shots)
        } else {
            Err(errors)
        }
    }

    /// A demo state in both themes; the state is reached again for each.
    async fn capture_demo_state(
        &self,
        cx: &mut AsyncApp,
        handle: AnyWindowHandle,
        root: &Entity<RootView>,
        size: WindowSize,
        state: ShotState,
    ) -> Outcome {
        let mut shots = Vec::new();
        let mut errors = Vec::new();
        let terminal = state.scene() == Scene::Terminal;
        if terminal {
            self.start_terminal(cx).map_err(|error| vec![error])?;
        }
        for (appearance, theme_name) in THEMES {
            set_theme(cx, handle, appearance).map_err(|error| vec![error])?;
            let file = file_name_on(size, theme_name, state, self.real_shell);
            // One attempt runs out after STATE_TIMEOUT; a failed or timed
            // out state is tried once more from the demo data (D72).
            let mut attempt = 0;
            let reached = loop {
                attempt += 1;
                mark_progress(Some(state));
                let timeout = cx.background_executor().timer(STATE_TIMEOUT);
                let try_reach = async {
                    if terminal {
                        let rig = self
                            .rig
                            .borrow()
                            .clone()
                            .ok_or_else(|| ScreenshotError::state(state, "no test server"))?;
                        terminal::set_terminal_theme(cx, &self.demo, appearance)?;
                        reset_demo(cx, handle, root, &self.demo).await?;
                        terminal::reach(cx, handle, root, &self.demo, &rig, state).await?;
                        settle(cx, handle).await?;
                        terminal::check(cx, handle, root, &rig, state)
                    } else {
                        reach_state(cx, handle, root, &self.demo, self.first_group, state).await?;
                        settle(cx, handle).await?;
                        check_state(cx, handle, root, state)
                    }
                };
                let outcome =
                    match futures::future::select(std::pin::pin!(try_reach), timeout).await {
                        futures::future::Either::Left((outcome, _)) => outcome,
                        futures::future::Either::Right(_) => Err(ScreenshotError::state(
                            state,
                            &format!("not reached in {} s", STATE_TIMEOUT.as_secs()),
                        )),
                    };
                match outcome {
                    Err(error) if attempt == 1 => {
                        complain(&format!("{file}: {error}; trying once more"));
                    }
                    outcome => break outcome,
                }
            };
            if let Err(error) = reached {
                errors.push(error);
                continue;
            }
            match capture(cx, handle, size, &self.out_dir, &file) {
                Ok(shot) => {
                    report(&shot);
                    shots.push(shot);
                }
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() {
            Ok(shots)
        } else {
            Err(errors)
        }
    }

    /// A state of a fresh empty profile: its own window and app state, both
    /// themes, then the window closes and the profile is deleted.
    async fn capture_fresh_state(
        &self,
        cx: &mut AsyncApp,
        size: WindowSize,
        state: ShotState,
        stage: FreshStage,
    ) -> Outcome {
        let fresh = open_fresh_state(cx, &self.parent, stage).map_err(|error| vec![error])?;
        let result = async {
            let (handle, root) = open_window(cx, &fresh, size).map_err(|error| vec![error])?;
            let mut shots = Vec::new();
            let mut errors = Vec::new();
            if let Err(error) = prepare_window(cx, handle, size).await {
                errors.push(error);
            } else {
                for (appearance, theme_name) in THEMES {
                    let file = file_name(size, theme_name, state);
                    // The same limit and retry as a demo state (D72).
                    let mut attempt = 0;
                    let ready = loop {
                        attempt += 1;
                        mark_progress(Some(state));
                        let timeout = cx.background_executor().timer(STATE_TIMEOUT);
                        let try_reach = async {
                            set_theme(cx, handle, appearance)?;
                            reach_fresh_state(cx, handle, state).await?;
                            settle(cx, handle).await?;
                            check_state(cx, handle, &root, state)
                        };
                        let outcome =
                            match futures::future::select(std::pin::pin!(try_reach), timeout).await
                            {
                                futures::future::Either::Left((outcome, _)) => outcome,
                                futures::future::Either::Right(_) => Err(ScreenshotError::state(
                                    state,
                                    &format!("not reached in {} s", STATE_TIMEOUT.as_secs()),
                                )),
                            };
                        match outcome {
                            Err(error) if attempt == 1 => {
                                complain(&format!("{file}: {error}; trying once more"));
                            }
                            outcome => break outcome,
                        }
                    };
                    if let Err(error) = ready {
                        errors.push(error);
                        continue;
                    }
                    match capture(cx, handle, size, &self.out_dir, &file) {
                        Ok(shot) => {
                            report(&shot);
                            shots.push(shot);
                        }
                        Err(error) => errors.push(error),
                    }
                }
            }
            close_window(cx, handle);
            if errors.is_empty() {
                Ok(shots)
            } else {
                Err(errors)
            }
        }
        .await;
        cx.update(|cx| fresh.update(cx, |fresh, _| fresh.shutdown()));
        result
    }

    /// Starts the test server and saves its hosts in the demo profile,
    /// once per window size (the first terminal state).
    fn start_terminal(&self, cx: &mut AsyncApp) -> Result<(), ScreenshotError> {
        if self.rig.borrow().is_some() {
            return Ok(());
        }
        let rig = terminal::Rig::start(cx, &self.demo, self.server_json.as_deref())?;
        *self.rig.borrow_mut() = Some(Rc::new(rig));
        Ok(())
    }

    /// After the states of a window size: closes the session tabs, removes
    /// the test server's hosts, known hosts and settings, stops the server.
    async fn finish_terminal(
        &self,
        cx: &mut AsyncApp,
        handle: AnyWindowHandle,
        root: &Entity<RootView>,
    ) -> Result<(), ScreenshotError> {
        let Some(rig) = self.rig.borrow_mut().take() else {
            return Ok(());
        };
        let reset = reset_demo(cx, handle, root, &self.demo).await;
        let finished = rig.finish(cx, &self.demo);
        drop(rig);
        reset.and(finished)
    }
}

/// The steps of a state on a fresh profile after its window opened: the
/// Keychain's empty state is one click away from the empty Hosts screen.
pub async fn reach_fresh_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    match shot_state {
        ShotState::KeychainEmpty => keychain_shot::reach_empty(cx, handle).await,
        _ => Ok(()),
    }
}

/// An app state over a fresh empty profile at `stage` (see
/// [`demo::open_fresh`]).
pub fn open_fresh_state(
    cx: &mut AsyncApp,
    parent: &Path,
    stage: FreshStage,
) -> Result<Entity<AppState>, ScreenshotError> {
    let profile = create_profile(parent)?;
    let opened = demo::open_fresh(profile, stage)?;
    Ok(cx.update(|cx| cx.new(|cx| AppState::new(opened, cx))))
}

fn report(shot: &Shot) {
    say(&format!(
        "Saved {} ({}x{})",
        shot.path.display(),
        shot.width,
        shot.height
    ));
}

/// The first frames of a new window (layout, fonts, icons), and the size
/// check: a window the platform clamped is asked once more.
async fn prepare_window(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    size: WindowSize,
) -> Result<(), ScreenshotError> {
    settle(cx, handle).await?;
    let scale = cx
        .update_window(handle, |_, window, _| {
            if window.viewport_size() != size.logical() {
                window.resize(size.logical());
            }
            window.scale_factor()
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    say(&format!("Window {size} at scale factor {scale}"));
    settle(cx, handle).await
}

fn set_theme(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    appearance: Appearance,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        theme::set_appearance(appearance, Some(window), cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Waits for the UI to settle and draws a fresh frame.
pub async fn settle(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
    cx.background_executor().timer(SETTLE / 2).await;
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .map_err(|_| ScreenshotError::WindowClosed)?;
    cx.background_executor().timer(SETTLE / 2).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Pointer out of the window: no hover state in the capture.
        window.dispatch_event(
            MouseMoveEvent {
                position: point(px(-10.), px(-10.)),
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// The shell of the window (the demo windows are always unlocked).
fn shell_of(
    cx: &mut AsyncApp,
    root: &Entity<RootView>,
    state: ShotState,
) -> Result<Entity<Shell>, ScreenshotError> {
    cx.update(|cx| root.read(cx).shell().cloned())
        .ok_or_else(|| ScreenshotError::state(state, "the shell is not shown"))
}

/// Brings the demo window back to the top level of the Hosts screen: no open
/// menu, panel, selection, search text, tag filter, session tab or toast;
/// tiles; the focus in the shell.
pub async fn reset_demo(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, ShotState::Root)?;
    // Open menus and dropdowns close on Escape.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
        window.press("escape", cx);
        // The dialogs of the data transfer states.
        window.close_all_dialogs(cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    let screen = cx.update(|cx| shell.read(cx).hosts_screen().clone());
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            let sessions: Vec<_> = shell.sessions().tabs().iter().map(|tab| tab.id()).collect();
            for id in sessions {
                shell.close_session(id, cx);
            }
            shell.set_section(Section::Hosts, cx);
            shell.set_top_tab(TopTab::Vaults, cx);
            shell.close_panel(cx);
            let toasts = shell.toasts().clone();
            toasts.update(cx, |toasts, cx| {
                let ids: Vec<u64> = toasts.items().iter().map(|toast| toast.id).collect();
                for id in ids {
                    toasts.dismiss(id, cx);
                }
            });
        });
        screen.update(cx, |screen, cx| {
            screen.set_layout(HostsLayout::Tiles, cx);
            let active: Vec<String> = screen.filter().tags().to_vec();
            for tag in active {
                screen.toggle_tag(&tag, cx);
            }
        });
        // The Keychain's own panel (its states open one).
        let keychain = shell.read(cx).keychain_screen().clone();
        keychain.update(cx, |keychain, cx| keychain.close_panel(cx));
        state.update(cx, |state, cx| {
            state.clear_selection(cx);
            state.set_location(None, cx);
        });
    });
    // The search bar is on screen again (Vaults tab): clear it like a user.
    let has_text = cx.update(|cx| !screen.read(cx).filter().text().is_empty());
    if has_text {
        type_into(cx, handle, SEARCH, "")?;
    }
    cx.update_window(handle, |_, window, cx| {
        // The terminal sidebar of the sidebar states: hidden, on Tools with
        // an empty search, Appearance on its theme list.
        let sidebar = shell.read(cx).terminal_sidebar().clone();
        let (tools, appearance) = {
            let sidebar = sidebar.read(cx);
            (sidebar.tools().clone(), sidebar.appearance().clone())
        };
        sidebar.update(cx, |sidebar, cx| {
            sidebar.set_visible(false, cx);
            sidebar.show_tab(crate::terminal_sidebar::SidebarTab::Tools, cx);
        });
        tools.update(cx, |tools, cx| tools.set_query("", window, cx));
        appearance.update(cx, |appearance, cx| appearance.show_main_page(window, cx));
        let focus = shell.read(cx).focus_handle().clone();
        focus.focus(window, cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await
}

/// Element ids the states are reached through (UI 0: stable ids).
pub mod ids {
    pub const SEARCH: &str = "search";
    pub const QUICK_CONNECT: &str = "quick-connect";
    pub const VIEW_LIST: &str = "view-list";
    pub const NEW_HOST: &str = "new-host";
    pub const HOST_PANEL: &str = "host-panel";
    pub const HOST_PANEL_BODY: &str = "host-panel-body";
    pub const HOST_ADDRESS: &str = "host-address";
    pub const HOST_SHOW_MORE: &str = "host-show-more";
    pub const HOST_MORE: &str = "host-more";
    /// Agent Forwarding's switch, warning and "Ask before every signature".
    pub const HOST_AGENT: &str = "host-agent-forwarding";
    pub const HOST_AGENT_WARNING: &str = "host-agent-warning";
    pub const HOST_AGENT_CONFIRM: &str = "host-agent-confirm";
    pub const POPUP_MENU: &str = "popup-menu";
    pub const BREADCRUMBS: &str = "breadcrumbs";
    pub const HOSTS_SCREEN: &str = "hosts-screen";
    pub const HOSTS_EMPTY: &str = "hosts-empty";
    pub const HOSTS_NO_MATCHES: &str = "hosts-no-matches";
    pub const TAG_FILTER: &str = "tag-filter";
    pub const TAG_FILTER_MENU: &str = "tag-filter-menu";
    pub const TAG_CHIP_MORE: &str = "tag-chip-more";
    pub const FIRST_RUN: &str = "first-run";
    pub const UNLOCK: &str = "unlock";
    /// Host cards: `host-card-<uuid>`.
    pub const HOST_CARD_PREFIX: &str = "host-card-";
}

use ids::*;

/// Brings the demo window into `shot_state` the way a user would, from the
/// reset state.
pub async fn reach_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    first_group: Option<Uuid>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    reset_demo(cx, handle, root, state).await?;
    let not_reached = |reason: &str| ScreenshotError::state(shot_state, reason);
    match shot_state {
        ShotState::Root => Ok(()),
        ShotState::Group => {
            let group = cx.update(|cx| {
                first_group
                    .filter(|id| state.read(cx).group(*id).is_some())
                    .or_else(|| {
                        state
                            .read(cx)
                            .groups_in(None)
                            .first()
                            .map(|group| group.record_id)
                    })
            });
            let Some(group) = group else {
                return Err(not_reached("no groups"));
            };
            if !click_if_visible(cx, handle, group_card_id(group))? {
                // Not on screen: navigate the way the card would.
                cx.update(|cx| state.update(cx, |state, cx| state.set_location(Some(group), cx)));
            }
            Ok(())
        }
        ShotState::HostPanel => click_first_card(cx, handle, shot_state, ClickKind::Left),
        ShotState::ContextMenu => click_first_card(cx, handle, shot_state, ClickKind::Right),
        ShotState::SearchAddress => type_into(cx, handle, SEARCH, QUICK_CONNECT_TEXT),
        ShotState::SearchResults => type_into(cx, handle, SEARCH, SEARCH_WITH_RESULTS),
        ShotState::SearchNoResults => type_into(cx, handle, SEARCH, SEARCH_WITHOUT_RESULTS),
        ShotState::ThreeSelected => {
            let cards = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    visible_cards(window)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            if cards.len() < 3 {
                return Err(not_reached("fewer than 3 host cards on screen"));
            }
            for card in cards.into_iter().take(3) {
                cx.update_window(handle, |_, window, cx| {
                    click_with(window, card, Modifiers::secondary_key(), cx)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            }
            Ok(())
        }
        ShotState::ListView => click(cx, handle, VIEW_LIST),
        ShotState::NewHost => click(cx, handle, NEW_HOST),
        ShotState::ExportData
        | ShotState::ExportDataPlain
        | ShotState::ExportOpenSsh
        | ShotState::ImportData
        | ShotState::ImportSshConfig => transfer_shot::reach(cx, handle, state, shot_state).await,
        ShotState::TagFilter => {
            // The dropdown open with two tags checked (they also show as
            // chips next to the button); both are on the first demo host.
            check_example_tags(cx, handle, &TAG_FILTER_EXAMPLE).await?;
            open_tag_filter(cx, handle)
        }
        ShotState::TagChips => {
            // The first demo host's four tags, the dropdown closed, then its
            // panel: the bar narrows, the chips that fit stay and the rest
            // collapse into "+N" (stage 2a step 0.1).
            check_example_tags(cx, handle, &TAG_CHIPS_EXAMPLE).await?;
            press(cx, handle, "escape")?;
            settle(cx, handle).await?;
            click_first_card(cx, handle, shot_state, ClickKind::Left)
        }
        ShotState::AgentForwarding => {
            click_first_card(cx, handle, shot_state, ClickKind::Left)?;
            settle(cx, handle).await?;
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)?;
            settle(cx, handle).await?;
            click(cx, handle, HOST_SHOW_MORE)?;
            settle(cx, handle).await?;
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)?;
            // Up from the bottom until the switch is on screen.
            scroll_until_visible(cx, handle, HOST_PANEL_BODY, HOST_AGENT)?;
            // The demo host keeps what an earlier run (the other theme)
            // saved: switch on only what is off.
            let shell = shell_of(cx, root, shot_state)?;
            let draft = |cx: &mut AsyncApp| {
                cx.update(|cx| match shell.read(cx).panel() {
                    Some(crate::shell::Panel::Host(panel)) => {
                        let draft = panel.read(cx).draft();
                        Some((draft.agent_forwarding, draft.agent_confirm))
                    }
                    _ => None,
                })
            };
            if draft(cx).is_some_and(|(on, _)| !on) {
                click(cx, handle, HOST_AGENT)?;
                settle(cx, handle).await?;
            }
            if draft(cx).is_some_and(|(_, confirm)| !confirm) {
                click(cx, handle, HOST_AGENT_CONFIRM)?;
                settle(cx, handle).await?;
            }
            Ok(())
        }
        ShotState::ShowMore => {
            click_first_card(cx, handle, shot_state, ClickKind::Left)?;
            settle(cx, handle).await?;
            // "Show more" sits below the SSH fields: scroll to it, expand,
            // then scroll to the new bottom of the panel.
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)?;
            settle(cx, handle).await?;
            click(cx, handle, HOST_SHOW_MORE)?;
            settle(cx, handle).await?;
            scroll_to_bottom(cx, handle, HOST_PANEL_BODY)
        }
        ShotState::KnownHosts => known_hosts_shot::reach(cx, handle, root, state).await,
        ShotState::SftpLocal
        | ShotState::SftpPicker
        | ShotState::SftpQueue
        | ShotState::SftpActions
        | ShotState::SftpDelete
        | ShotState::SftpPermissions
        | ShotState::KeychainExport
        | ShotState::KeychainCertificate => {
            sftp_shot::reach(cx, handle, root, state, shot_state).await
        }
        ShotState::Keychain
        | ShotState::KeychainPanel
        | ShotState::KeychainGenerate
        | ShotState::KeychainImportUnknown => {
            keychain_shot::reach(cx, handle, root, shot_state).await
        }
        ShotState::ForwardingEmpty
        | ShotState::ForwardingCards
        | ShotState::ForwardingForm
        | ShotState::ForwardingWizardType
        | ShotState::ForwardingWizardRemote
        | ShotState::ForwardingWizardDynamic
        | ShotState::ForwardingWizardHost
        | ShotState::ForwardingWizardListen
        | ShotState::ForwardingWizardDest
        | ShotState::ForwardingWizardSummary => {
            forwarding_shot::reach(cx, handle, root, state, shot_state).await
        }
        ShotState::Snippets
        | ShotState::SnippetPanel
        | ShotState::SnippetVariables
        | ShotState::SnippetConfirm
        | ShotState::SnippetResults
        | ShotState::SnippetLeave
        | ShotState::ShellHistory => {
            snippets_shot::reach(cx, handle, root, state, shot_state).await
        }
        ShotState::Logs | ShotState::SettingsLogs | ShotState::SettingsShortcuts => {
            logs_shot::reach(cx, handle, root, state, shot_state).await
        }
        ShotState::Empty | ShotState::FirstRun | ShotState::Unlock | ShotState::KeychainEmpty => {
            Err(not_reached("shown on a fresh profile, not on the demo"))
        }
        ShotState::TerminalTofu
        | ShotState::TerminalLsColor
        | ShotState::TerminalHtop
        | ShotState::TerminalTmux
        | ShotState::TerminalTmuxResize
        | ShotState::TerminalMc
        | ShotState::TerminalVim
        | ShotState::TerminalNano
        | ShotState::TerminalLess
        | ShotState::TerminalEmacs
        | ShotState::TerminalVttest
        | ShotState::TerminalUnicode
        | ShotState::TerminalDrop
        | ShotState::TerminalLocal
        | ShotState::TerminalTabs
        | ShotState::QuickConnectToast
        | ShotState::TerminalHostKeyChanged
        | ShotState::TerminalConnecting
        | ShotState::TerminalSplits
        | ShotState::SplitPicker
        | ShotState::TabMenu
        | ShotState::NewTab
        | ShotState::NewTabSearch
        | ShotState::SidebarTools
        | ShotState::SidebarSnippets
        | ShotState::SidebarHistory
        | ShotState::SidebarAppearance
        | ShotState::SidebarFont => Err(not_reached("needs the SSH test server (terminal::reach)")),
    }
}

/// Checks that the window shows `shot_state` (the evidence a reviewer looks
/// for in the PNG), so a missed click fails the run instead of saving a
/// wrong picture.
pub fn check_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    let failed = |reason: &str| Err(ScreenshotError::state(shot_state, reason));
    if shot_state == ShotState::KnownHosts {
        return known_hosts_shot::check(cx, handle, root);
    }
    if ShotState::STAGE_3.contains(&shot_state) {
        return keychain_shot::check(cx, handle, root, shot_state);
    }
    if sftp_shot::STATES.contains(&shot_state) {
        return sftp_shot::check(cx, handle, shot_state);
    }
    if forwarding_shot::STATES.contains(&shot_state) {
        return forwarding_shot::check(cx, handle, shot_state);
    }
    if snippets_shot::STATES.contains(&shot_state) {
        return snippets_shot::check(cx, handle, shot_state);
    }
    if logs_shot::STATES.contains(&shot_state) {
        return logs_shot::check(cx, handle, shot_state);
    }
    let shell = cx.update(|cx| root.read(cx).shell().cloned());
    let facts = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let screen = shell
                .as_ref()
                .map(|shell| shell.read(cx).hosts_screen().clone());
            Facts {
                visible: |id: &str| visible(window, id),
                cards: visible_cards(window).len(),
                selected: root.read(cx).state().read(cx).selection().ids().len(),
                active_tags: screen
                    .as_ref()
                    .map_or(0, |screen| screen.read(cx).filter().tags().len()),
                list_layout: screen
                    .as_ref()
                    .is_some_and(|screen| screen.read(cx).layout() == HostsLayout::List),
                connect_enabled: screen
                    .as_ref()
                    .is_some_and(|screen| screen.read(cx).connect_enabled()),
                address_focused: window
                    .try_find(HOST_ADDRESS)
                    .and_then(|snapshot| snapshot.focused())
                    == Some(true),
                address_empty: window
                    .try_find(HOST_ADDRESS)
                    .and_then(|snapshot| snapshot.value().map(str::is_empty))
                    == Some(true),
            }
            .evaluate(shot_state)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    match facts {
        Ok(()) => Ok(()),
        Err(reason) => failed(reason),
    }
}

/// What [`check_state`] reads from a frame.
struct Facts<V: Fn(&str) -> bool> {
    visible: V,
    cards: usize,
    selected: usize,
    active_tags: usize,
    list_layout: bool,
    connect_enabled: bool,
    address_focused: bool,
    address_empty: bool,
}

impl<V: Fn(&str) -> bool> Facts<V> {
    fn evaluate(&self, state: ShotState) -> Result<(), &'static str> {
        let visible = &self.visible;
        let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
        match state {
            ShotState::Root => {
                need(visible(HOSTS_SCREEN) && self.cards > 0, "no host cards")?;
                need(!visible(HOST_PANEL), "a panel is open")
            }
            ShotState::Group => need(visible(BREADCRUMBS), "no breadcrumbs"),
            ShotState::HostPanel => need(visible(HOST_PANEL), "no host panel"),
            ShotState::ContextMenu => need(visible(POPUP_MENU), "no context menu"),
            ShotState::SearchAddress => need(self.connect_enabled, "Connect is not enabled"),
            ShotState::SearchResults => need(self.cards > 0, "no results"),
            ShotState::SearchNoResults => {
                need(self.cards == 0, "results shown")?;
                need(visible(HOSTS_NO_MATCHES), "no \"no matches\" text")
            }
            ShotState::ThreeSelected => need(self.selected == 3, "not 3 cards selected"),
            ShotState::Empty => need(visible(HOSTS_EMPTY), "no empty state"),
            ShotState::ListView => need(self.list_layout && self.cards > 0, "not the list view"),
            ShotState::NewHost => {
                need(visible(HOST_PANEL), "no host panel")?;
                need(self.address_empty, "Address is not empty")?;
                need(self.address_focused, "Address has no focus")
            }
            // `transfer_shot::reach` checked the dialog's content.
            ShotState::ExportData => {
                need(visible("data-export-dialog"), "no Export Data dialog")?;
                need(visible("data-export-password"), "no password field")
            }
            ShotState::ExportDataPlain => {
                need(visible("data-export-dialog"), "no Export Data dialog")?;
                need(visible("data-export-warning"), "no clear-text warning")
            }
            ShotState::ExportOpenSsh => {
                need(visible("data-export-keys"), "no OpenSSH files form")?;
                need(visible("data-export-password"), "no passphrase field")
            }
            ShotState::ImportData => need(visible("data-import-summary"), "no import summary"),
            ShotState::ImportSshConfig => need(visible("import-preview"), "no import preview"),
            ShotState::TagFilter => {
                need(visible(TAG_FILTER_MENU), "no tag filter dropdown")?;
                need(
                    self.active_tags == TAG_FILTER_EXAMPLE.len(),
                    "the example tags are not on",
                )
            }
            ShotState::TagChips => {
                need(visible(HOST_PANEL), "no host panel")?;
                need(
                    self.active_tags == TAG_CHIPS_EXAMPLE.len(),
                    "the example tags are not on",
                )?;
                need(!visible(TAG_FILTER_MENU), "the tag filter dropdown is open")?;
                need(
                    visible(TAG_CHIP_MORE),
                    "the chips did not collapse into \"+N\"",
                )?;
                // Step 0.1: the chips that fit stay, "+N" is not alone.
                need(
                    TAG_CHIPS_EXAMPLE
                        .iter()
                        .any(|tag| visible(crate::hosts::tag_chip_id(tag).as_ref())),
                    "no chip is shown next to \"+N\"",
                )
            }
            ShotState::ShowMore => need(visible(HOST_MORE), "\"Show more\" is not expanded"),
            ShotState::AgentForwarding => need(
                visible(HOST_AGENT_WARNING) && visible(HOST_AGENT_CONFIRM),
                "no Agent Forwarding warning",
            ),
            ShotState::FirstRun => need(visible(FIRST_RUN), "no first-run screen"),
            ShotState::Unlock => need(visible(UNLOCK), "no unlock screen"),
            // Checked by `known_hosts_shot::check`.
            ShotState::KnownHosts => Err("checked with the Known Hosts screen"),
            // Checked by `keychain_shot::check`.
            ShotState::Keychain
            | ShotState::KeychainPanel
            | ShotState::KeychainGenerate
            | ShotState::KeychainImportUnknown
            | ShotState::KeychainEmpty => Err("checked with the Keychain screen"),
            // Checked by `sftp_shot::check`.
            ShotState::SftpLocal
            | ShotState::SftpPicker
            | ShotState::SftpQueue
            | ShotState::SftpActions
            | ShotState::SftpDelete
            | ShotState::SftpPermissions
            | ShotState::KeychainExport
            | ShotState::KeychainCertificate => Err("checked with the stage 4 screens"),
            // Checked by `forwarding_shot::check`.
            ShotState::ForwardingEmpty
            | ShotState::ForwardingCards
            | ShotState::ForwardingForm
            | ShotState::ForwardingWizardType
            | ShotState::ForwardingWizardRemote
            | ShotState::ForwardingWizardDynamic
            | ShotState::ForwardingWizardHost
            | ShotState::ForwardingWizardListen
            | ShotState::ForwardingWizardDest
            | ShotState::ForwardingWizardSummary
            | ShotState::Snippets
            | ShotState::SnippetPanel
            | ShotState::SnippetVariables
            | ShotState::SnippetConfirm
            | ShotState::SnippetResults
            | ShotState::SnippetLeave
            | ShotState::ShellHistory => Err("checked with the stage 5 screens"),
            ShotState::Logs | ShotState::SettingsLogs | ShotState::SettingsShortcuts => {
                Err("checked with the stage 6 screens")
            }
            // Checked with their session (`terminal::check`).
            ShotState::TerminalTofu
            | ShotState::TerminalLsColor
            | ShotState::TerminalHtop
            | ShotState::TerminalTmux
            | ShotState::TerminalTmuxResize
            | ShotState::TerminalMc
            | ShotState::TerminalVim
            | ShotState::TerminalNano
            | ShotState::TerminalLess
            | ShotState::TerminalEmacs
            | ShotState::TerminalVttest
            | ShotState::TerminalUnicode
            | ShotState::TerminalDrop
            | ShotState::TerminalLocal
            | ShotState::TerminalTabs
            | ShotState::QuickConnectToast
            | ShotState::TerminalHostKeyChanged
            | ShotState::TerminalConnecting
            | ShotState::TerminalSplits
            | ShotState::SplitPicker
            | ShotState::TabMenu
            | ShotState::NewTab
            | ShotState::NewTabSearch
            | ShotState::SidebarTools
            | ShotState::SidebarSnippets
            | ShotState::SidebarHistory
            | ShotState::SidebarAppearance
            | ShotState::SidebarFont => Err("a terminal state needs its session"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClickKind {
    Left,
    Right,
}

/// Clicks the first host card on screen (top left of the Hosts section).
fn click_first_card(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot_state: ShotState,
    kind: ClickKind,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let Some(card) = visible_cards(window).into_iter().next() else {
            return Err(ScreenshotError::state(shot_state, "no host card on screen"));
        };
        match kind {
            ClickKind::Left => window.click(card, cx),
            ClickKind::Right => window.right_click(card, cx),
        }
        Ok(())
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

fn click(cx: &mut AsyncApp, handle: AnyWindowHandle, id: &str) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if !visible(window, id) {
            return Err(ScreenshotError::State(format!("{id} is not on screen")));
        }
        window.click(gpui::SharedString::from(id.to_owned()), cx);
        Ok(())
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

/// Checks `tags` in the tag filter dropdown, one at a time.
async fn check_example_tags(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    tags: &[&str],
) -> Result<(), ScreenshotError> {
    for tag in tags.iter().copied() {
        open_tag_filter(cx, handle)?;
        settle(cx, handle).await?;
        click(cx, handle, &tag_option_id(tag))?;
        settle(cx, handle).await?;
    }
    Ok(())
}

/// Opens the tag filter dropdown unless it is open.
fn open_tag_filter(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
    let open = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            visible(window, TAG_FILTER_MENU)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    if open {
        Ok(())
    } else {
        click(cx, handle, TAG_FILTER)
    }
}

/// Scrolls the container `id` far past its end (it stops at the bottom).
fn scroll_to_bottom(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &'static str,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(id, ScrollDelta::Pixels(point(px(0.), px(-20_000.))), cx);
        window.render_frame(cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Scrolls `id` up in steps until `target` is on screen.
fn scroll_until_visible(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &'static str,
    target: &'static str,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..40 {
            window.render_frame(cx);
            if visible(window, target) {
                return Ok(());
            }
            window.scroll(id, ScrollDelta::Pixels(point(px(0.), px(120.))), cx);
        }
        Err(ScreenshotError::State(format!("{target} is not on screen")))
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

fn press(cx: &mut AsyncApp, handle: AnyWindowHandle, key: &str) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .map_err(|_| ScreenshotError::WindowClosed)
}

/// Replaces the text of the input `id` (click, select all, delete, type).
fn type_into(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &'static str,
    text: &str,
) -> Result<(), ScreenshotError> {
    click(cx, handle, id)?;
    cx.update_window(handle, |_, window, cx| {
        window.press("secondary-a", cx);
        window.press("backspace", cx);
        if !text.is_empty() {
            window.input(text, cx);
        }
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Whether the element `id` is on screen.
fn visible(window: &Window, id: &str) -> bool {
    window
        .try_find(gpui::SharedString::from(id.to_owned()))
        .is_some_and(|snapshot| snapshot.visible())
}

/// The ids of the host cards on screen, top to bottom, then left to right.
fn visible_cards(window: &Window) -> Vec<gpui::SharedString> {
    let mut cards: Vec<(f32, f32, String)> = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.visible())
        .filter_map(|snapshot| {
            let id = snapshot.path().last()?.to_string();
            let uuid = id.strip_prefix(HOST_CARD_PREFIX)?;
            Uuid::parse_str(uuid).ok()?;
            let origin = snapshot.bounds().origin;
            Some((origin.y.as_f32(), origin.x.as_f32(), id))
        })
        .collect();
    cards.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    cards.dedup_by(|a, b| a.2 == b.2);
    cards.into_iter().map(|(_, _, id)| id.into()).collect()
}

/// Clicks the element `id` (a real pointer event sequence through GPUI's
/// dispatch) when it is on screen.
fn click_if_visible(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: gpui::SharedString,
) -> Result<bool, ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let visible = window
            .try_find(id.clone())
            .is_some_and(|snapshot| snapshot.visible());
        if visible {
            window.click(id, cx);
        }
        visible
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Clicks the element `id` with `modifiers` held (Ctrl+click selection,
/// which gpui-kit's `click` cannot send).
fn click_with(window: &mut Window, id: gpui::SharedString, modifiers: Modifiers, cx: &mut App) {
    window.render_frame(cx);
    let position = window.find(id).bounds().center();
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

/// Renders the current scene offscreen and writes it as a PNG.
fn capture(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    size: WindowSize,
    out_dir: &Path,
    file: &str,
) -> Result<Shot, ScreenshotError> {
    let path = out_dir.join(file);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let scale = window.scale_factor();
        let image = window
            .render_to_image()
            .map_err(|error| ScreenshotError::Render {
                file: file.to_owned(),
                message: format!("{error:#}"),
            })?;
        let (width, height) = (image.width(), image.height());
        image.save(&path).map_err(|error| ScreenshotError::Save {
            file: file.to_owned(),
            message: error.to_string(),
        })?;
        let want_width = (size.width as f32 * scale).round() as u32;
        let want_height = (size.height as f32 * scale).round() as u32;
        let off = |got: u32, want: u32| (i64::from(got) - i64::from(want)).abs() > SIZE_TOLERANCE;
        if off(width, want_width) || off(height, want_height) {
            return Err(ScreenshotError::Size {
                file: file.to_owned(),
                got_width: width,
                got_height: height,
                want_width,
                want_height,
            });
        }
        Ok(Shot {
            path: path.clone(),
            width,
            height,
        })
    })
    .map_err(|_| ScreenshotError::WindowClosed)?
}

mod forwarding_shot;
mod keychain_shot;
mod known_hosts_shot;
mod logs_shot;
mod sftp_shot;
mod snippets_shot;
mod terminal;
mod transfer_shot;

#[cfg(test)]
mod tests;
