//! The terminal states of the screenshot run (owner's stage 2a, step 5.1):
//! session tabs of the demo window to an SSH test server, never a real host
//! ([`crate::dev::test_server`]).
//!
//! - Server: OpenSSH in Docker when `SHELLPP_TEST_SERVER_JSON` names the
//!   `server.json` of `tests/openssh/start.sh` (`scripts/ui-screens.ps1`
//!   starts it), otherwise the russh test server as a child process
//!   (`SHELLPP_TEST_SSHD_EXE`). The run prints which one it used.
//! - Hosts: three hosts of the test server ([`HOST_LABELS`]: address, port,
//!   the password user and its saved password) are added to the demo
//!   profile for the terminal states and removed afterwards, with the known
//!   host entries and the settings the states change. The session layer has
//!   no key files and no agent.
//! - Tabs open as the Connect action does (`Shell::open_session` with the
//!   saved host); dialogs are answered and buttons pressed by their element
//!   ids, commands are typed into the focused terminal (`Window::input`,
//!   Enter). Each state starts from the reset of the demo window (no session
//!   tabs) and is checked before the capture ([`check`]).
//! - The offer to save a quick-connect host needs a session that connected:
//!   the test server's `user@address:port` is typed into the search bar, and
//!   the password prompt of that tab (and only of that tab) gets the server's
//!   password.
//! - Light screenshots use the Paper terminal theme (the setting "Terminal
//!   theme", design-system 4.2), dark ones Graphite (4.1).
//! - The local terminal is the app's default shell (Windows PowerShell on
//!   Windows). Its prompt is replaced by `PS> ` (`$ ` elsewhere) so that no
//!   user directory appears in the picture; the script gives the app a
//!   temporary APPDATA, so the typed commands do not reach the user's
//!   PowerShell history.
//!
//! Nothing here logs addresses, user names, secrets or terminal text.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp, Entity, Focusable as _};
use gpui_kit::test::TestWindowExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use shellpp_core::Core;
use shellpp_core::service::dto::{RecordsRef, SettingsSetInput};
use uuid::Uuid;

use super::ids::SEARCH;
use super::{ScreenshotError, ShotState, click, press, settle, shell_of, type_into, visible};
use crate::app::RootView;
use crate::dev::test_server::{DevServer, ServerKind};
use crate::hosts::QuickTarget;
use crate::session_ui::{HostKeyAsk, Phase, Prompt, SessionPane};
use crate::settings::terminal::{KEY_AUTORECONNECT, KEY_DEFAULT_THEME, TerminalPrefs};
use crate::shell::{SessionId, SessionKind, SessionRequest, Shell, TopTab};
use crate::state::AppState;
use crate::theme::Appearance;

/// The `server.json` of an OpenSSH test server started by the caller.
pub const SERVER_JSON_ENV: &str = "SHELLPP_TEST_SERVER_JSON";

/// The labels of the test server's hosts (the tab titles).
pub const HOST_LABELS: [&str; 3] = ["Test server", "Test server 2", "Test server 3"];

/// Connecting, with the host key step.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
/// A command's output, a dialog, a phase.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// The screen is unchanged this long: the shell waits at its prompt.
const QUIET: Duration = Duration::from_millis(600);
/// htop draws its meters after its first refresh.
const HTOP_DRAW: Duration = Duration::from_millis(1600);
const POLL: Duration = Duration::from_millis(50);

/// What `ls --color` shows in the OpenSSH image's home directories.
const LS_MARKER: &str = "deploy.sh";
/// What `ls -l --color` shows (a symlink).
const LS_LONG_MARKER: &str = "current -> projects/website";
/// htop's header.
const HTOP_MARKER: &str = "Load average";
/// The russh test server's `color` output (SGR codes as text).
const RUSSH_COLOR_MARKER: &str = "30 31 32";
/// The russh test server's `alt-screen` text.
const RUSSH_ALT_MARKER: &str = "ALT SCREEN (press q)";
/// The password field of a session prompt.
const PROMPT_INPUT: &str = "session-prompt-input";
/// The action button of the offer to save a quick-connect host.
const SAVE_HOST_OFFER: &str = "toast-save-host";
/// The line the local terminal prints.
const LOCAL_MARKER: &str = "Shell++ local terminal";

/// `SHELLPP_TEST_SERVER_JSON` when set and not empty.
pub fn server_json() -> Option<PathBuf> {
    std::env::var_os(SERVER_JSON_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The server the terminal states use, for the run's output.
pub fn server_label(real_shell: bool) -> &'static str {
    if real_shell {
        ServerKind::OpenSsh.label()
    } else {
        ServerKind::Russh.label()
    }
}

/// The test server and its hosts in the demo profile.
pub struct Rig {
    server: RefCell<DevServer>,
    vault_id: Uuid,
    hosts: Vec<Uuid>,
    /// Set when this run rotated the server's host keys; the next "Replace
    /// saved key" uses it up.
    rotated: Cell<bool>,
}

impl std::fmt::Debug for Rig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rig").finish_non_exhaustive()
    }
}

fn server_error(what: &str, error: impl std::fmt::Display) -> ScreenshotError {
    ScreenshotError::TestServer(format!("{what}: {error}"))
}

fn core_of(cx: &mut AsyncApp, state: &Entity<AppState>) -> Result<Arc<Core>, ScreenshotError> {
    cx.update(|cx| state.read(cx).core().cloned())
        .ok_or_else(|| ScreenshotError::TestServer("the demo profile is not open".to_owned()))
}

fn set_values(
    core: &Core,
    values: BTreeMap<String, Value>,
    reset: Vec<String>,
) -> Result<(), ScreenshotError> {
    core.settings_set(SettingsSetInput { values, reset })
        .map(|_| ())
        .map_err(|error| server_error("settings", error.code()))
}

impl Rig {
    /// Starts the server (OpenSSH of `server_json`, else the russh child
    /// process) and saves its hosts in the demo profile.
    pub fn start(
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
        server_json: Option<&Path>,
    ) -> Result<Self, ScreenshotError> {
        let server = DevServer::start(server_json).map_err(ScreenshotError::TestServer)?;
        let core = core_of(cx, state)?;
        let mut vault_id = None;
        let mut hosts = Vec::new();
        for label in HOST_LABELS {
            let (vault, record) = server
                .save_host(&core, label)
                .map_err(|error| server_error("saving its host", error))?;
            vault_id = Some(vault);
            hosts.push(record);
        }
        let vault_id = vault_id.ok_or_else(|| server_error("saving its host", "no vault"))?;
        Ok(Self {
            server: RefCell::new(server),
            vault_id,
            hosts,
            rotated: Cell::new(false),
        })
    }

    fn real_shell(&self) -> bool {
        self.server.borrow().has_real_shell()
    }

    /// Whether `fingerprint` is one of the test server's current host keys.
    fn is_server_key(&self, fingerprint: &str) -> bool {
        self.server
            .borrow()
            .host_key_fingerprints
            .iter()
            .any(|known| known == fingerprint)
    }

    /// Removes the hosts, the known host entries and the settings the states
    /// changed. The server stops when the rig is dropped.
    pub fn finish(
        &self,
        cx: &mut AsyncApp,
        state: &Entity<AppState>,
    ) -> Result<(), ScreenshotError> {
        let core = core_of(cx, state)?;
        let removed = core
            .record_delete(RecordsRef {
                vault_id: self.vault_id,
                record_ids: self.hosts.clone(),
            })
            .map(|_| ())
            .map_err(|error| server_error("removing its hosts", error.code()));
        let forgotten = self.forget_host_keys(&core);
        let reset = set_values(
            &core,
            BTreeMap::new(),
            vec![KEY_AUTORECONNECT.to_owned(), KEY_DEFAULT_THEME.to_owned()],
        );
        cx.update(|cx| TerminalPrefs::refresh(&core, cx));
        removed.and(forgotten).and(reset)
    }

    /// Whether a known host entry is the test server's.
    fn is_server_entry(&self, host: &str, port: Option<u16>) -> bool {
        let addr = self.server.borrow().addr;
        host == addr.ip().to_string() && port.unwrap_or(22) == addr.port()
    }

    /// The saved fingerprints of the test server.
    fn saved_fingerprints(&self, core: &Core) -> Result<Vec<String>, ScreenshotError> {
        Ok(core
            .known_hosts_list()
            .map_err(|error| server_error("known hosts", error.code()))?
            .into_iter()
            .filter(|known| self.is_server_entry(&known.host, known.port))
            .map(|known| known.fingerprint)
            .collect())
    }

    /// Removes the test server's known host entries (the next connection
    /// asks again).
    fn forget_host_keys(&self, core: &Core) -> Result<(), ScreenshotError> {
        let mut by_vault: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for known in core
            .known_hosts_list()
            .map_err(|error| server_error("known hosts", error.code()))?
        {
            if self.is_server_entry(&known.host, known.port) {
                by_vault
                    .entry(known.vault_id)
                    .or_default()
                    .push(known.record_id);
            }
        }
        for (vault_id, record_ids) in by_vault {
            core.known_host_remove(RecordsRef {
                vault_id,
                record_ids,
            })
            .map_err(|error| server_error("removing known hosts", error.code()))?;
        }
        Ok(())
    }
}

/// The terminal theme of `appearance`: Paper for light, Graphite for dark,
/// through the setting "Terminal theme" (the terminal views follow it).
pub fn set_terminal_theme(
    cx: &mut AsyncApp,
    state: &Entity<AppState>,
    appearance: Appearance,
) -> Result<(), ScreenshotError> {
    let core = core_of(cx, state)?;
    let slug = match appearance {
        Appearance::Light => "paper",
        Appearance::Dark | Appearance::Auto => "graphite",
    };
    let mut values = BTreeMap::new();
    values.insert(KEY_DEFAULT_THEME.to_owned(), json!({ "builtin": slug }));
    set_values(&core, values, Vec::new())?;
    cx.update(|cx| TerminalPrefs::refresh(&core, cx));
    Ok(())
}

fn set_autoreconnect(core: &Core, on: bool) -> Result<(), ScreenshotError> {
    if on {
        set_values(core, BTreeMap::new(), vec![KEY_AUTORECONNECT.to_owned()])
    } else {
        let mut values = BTreeMap::new();
        values.insert(KEY_AUTORECONNECT.to_owned(), json!(false));
        set_values(core, values, Vec::new())
    }
}

/// Draws a frame (the shell starts new panes at its render).
fn render(cx: &mut AsyncApp, handle: AnyWindowHandle) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .map_err(|_| ScreenshotError::WindowClosed)
}

/// Whether the element `id` is on screen in a fresh frame.
fn visible_now(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    id: &str,
) -> Result<bool, ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        visible(window, id)
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

async fn sleep(cx: &mut AsyncApp, duration: Duration) {
    cx.background_executor().timer(duration).await;
}

/// Polls `condition` after a fresh frame until it holds.
async fn wait_until(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
    what: &str,
    timeout: Duration,
    mut condition: impl FnMut(&mut App) -> Result<bool, ScreenshotError>,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + timeout;
    loop {
        render(cx, handle)?;
        if cx.update(|cx| condition(cx))? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(ScreenshotError::state(shot, &format!("timeout: {what}")));
        }
        sleep(cx, POLL).await;
    }
}

/// The visible rows of the pane's terminal.
fn screen(pane: &Entity<SessionPane>, cx: &App) -> Vec<String> {
    pane.read(cx)
        .terminal()
        .map(|terminal| terminal.read(cx).handle().screen_text())
        .unwrap_or_default()
}

fn shows(pane: &Entity<SessionPane>, text: &str, cx: &App) -> bool {
    screen(pane, cx).iter().any(|row| row.contains(text))
}

/// The last non-empty row (the prompt line when the shell waits).
fn last_row(rows: &[String]) -> Option<&str> {
    rows.iter()
        .rev()
        .map(|row| row.trim_end())
        .find(|row| !row.is_empty())
}

/// Whether `row` is a shell prompt: bash's `$`, PowerShell's `>`, or the
/// `#`/`%` of other shells.
fn is_prompt(row: &str) -> bool {
    row.ends_with(['$', '>', '#', '%'])
}

/// Waits until the screen stays unchanged for [`QUIET`] with a prompt on
/// its last line.
async fn wait_prompt(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut last: Vec<String> = Vec::new();
    let mut since = Instant::now();
    loop {
        render(cx, handle)?;
        let rows = cx.update(|cx| screen(pane, cx));
        if rows != last {
            last = rows;
            since = Instant::now();
        } else if since.elapsed() >= QUIET && last_row(&last).is_some_and(is_prompt) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(ScreenshotError::state(shot, "timeout: no shell prompt"));
        }
        sleep(cx, POLL).await;
    }
}

/// Types `line` into the pane's terminal and presses Enter.
fn type_line(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    line: &str,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        let focus = pane.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.input(line, cx);
        window.press("enter", cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)
}

/// Runs `line` and waits for `marker` on the screen and the prompt after it.
async fn run(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    shot: ShotState,
    line: &str,
    marker: &str,
) -> Result<(), ScreenshotError> {
    type_line(cx, handle, pane, line)?;
    wait_until(
        cx,
        handle,
        shot,
        "the command's output",
        STEP_TIMEOUT,
        |cx| Ok(shows(pane, marker, cx)),
    )
    .await?;
    wait_prompt(cx, handle, pane, shot).await
}

/// Clears the screen (the login banner goes) and waits for the prompt.
async fn clear(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    shot: ShotState,
    command: &str,
) -> Result<(), ScreenshotError> {
    type_line(cx, handle, pane, command)?;
    wait_until(cx, handle, shot, "a cleared screen", STEP_TIMEOUT, |cx| {
        let rows = screen(pane, cx);
        let used = rows.iter().filter(|row| !row.trim().is_empty()).count();
        Ok(used == 1 && last_row(&rows).is_some_and(is_prompt))
    })
    .await?;
    wait_prompt(cx, handle, pane, shot).await
}

/// Opens a session tab like the Connect action and returns its id and pane.
async fn open_tab(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    request: SessionRequest,
    shot: ShotState,
) -> Result<(SessionId, Entity<SessionPane>), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    let id = cx.update(|cx| shell.update(cx, |shell, cx| shell.open_session(request, cx)));
    // The pane's session starts at the shell's next render.
    render(cx, handle)?;
    let pane = cx
        .update(|cx| shell.read(cx).pane(id).cloned())
        .ok_or_else(|| ScreenshotError::state(shot, "the tab has no session pane"))?;
    render(cx, handle)?;
    Ok((id, pane))
}

/// Opens an SSH tab to the test server's host `index`.
async fn open_ssh(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    index: usize,
    shot: ShotState,
) -> Result<(SessionId, Entity<SessionPane>), ScreenshotError> {
    let host = rig.hosts[index];
    // The app state learns about the saved hosts through the core's events.
    wait_until(
        cx,
        handle,
        shot,
        "the test server's host",
        STEP_TIMEOUT,
        |cx| Ok(state.read(cx).host(host).is_some()),
    )
    .await?;
    open_tab(
        cx,
        handle,
        root,
        SessionRequest::new(SessionKind::Ssh).host(host),
        shot,
    )
    .await
}

/// Waits until the session runs, answering a host key dialog the way the
/// user would, but only for the test server (`rig`; `None` for a local
/// terminal): see [`host_key_button`]. `password`: a quick connect to the
/// test server (no saved password) gets the server's password typed into
/// its password prompt; a refused password ends the run.
async fn connect(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    rig: Option<&Rig>,
    password: Option<&SecretString>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut answered: Option<HostKeyAsk> = None;
    loop {
        render(cx, handle)?;
        let (phase, ask, prompt) = cx.update(|cx| {
            let pane = pane.read(cx);
            (
                pane.phase().clone(),
                pane.host_key().cloned(),
                pane.prompt().cloned(),
            )
        });
        match (&prompt, password) {
            (None, _) => {}
            (Some(asked @ Prompt::Password { .. }), Some(password)) if !asked.has_error() => {
                // The form is created at the pane's render.
                if visible_now(cx, handle, PROMPT_INPUT)? {
                    type_into(cx, handle, PROMPT_INPUT, password.expose_secret())?;
                    press(cx, handle, "enter")?;
                }
            }
            (Some(_), _) => {
                return Err(ScreenshotError::state(
                    shot,
                    "an unexpected prompt (or a refused password)",
                ));
            }
        }
        match phase {
            Phase::Running => return Ok(()),
            Phase::Failed(failure) => {
                return Err(ScreenshotError::state(
                    shot,
                    &format!("the connection failed ({})", failure.code()),
                ));
            }
            Phase::Closed | Phase::Ended(_) | Phase::Dropped(_) => {
                return Err(ScreenshotError::state(shot, "the session ended"));
            }
            Phase::Connecting(_) | Phase::Reconnecting(_) => {}
        }
        if let Some(ask) = ask
            && answered.as_ref() != Some(&ask)
        {
            let button = host_key_button(rig, &ask)
                .map_err(|reason| ScreenshotError::state(shot, reason))?;
            click(cx, handle, button)?;
            answered = Some(ask);
        }
        if Instant::now() > deadline {
            return Err(ScreenshotError::state(shot, "timeout: connecting"));
        }
        sleep(cx, POLL).await;
    }
}

/// The button that answers a host key dialog: "Accept and save" for one of
/// the test server's own keys, "Replace saved key" only for one of its keys
/// right after this run rotated them. Anything else ends the run: the
/// screenshots never trust (and never send the password to) another server.
fn host_key_button(rig: Option<&Rig>, ask: &HostKeyAsk) -> Result<&'static str, &'static str> {
    let Some(rig) = rig else {
        return Err("a host key question for a local terminal");
    };
    match ask {
        HostKeyAsk::Unknown(key) if rig.is_server_key(&key.fingerprint) => Ok("host-key-accept"),
        HostKeyAsk::Changed { key, .. }
            if rig.is_server_key(&key.fingerprint) && rig.rotated.replace(false) =>
        {
            Ok("host-key-replace")
        }
        _ => Err("a host key that is not the test server's"),
    }
}

/// An SSH tab to host `index`, connected, at its prompt.
async fn ssh_session(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    index: usize,
    shot: ShotState,
) -> Result<(SessionId, Entity<SessionPane>), ScreenshotError> {
    let (id, pane) = open_ssh(cx, handle, root, state, rig, index, shot).await?;
    connect(cx, handle, &pane, Some(rig), None, shot).await?;
    wait_prompt(cx, handle, &pane, shot).await?;
    Ok((id, pane))
}

/// Colored output: `ls --color` and `ls -l --color` on OpenSSH, the color
/// sample and wide characters on the russh test server.
async fn colored_output(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    pane: &Entity<SessionPane>,
    rig: &Rig,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    if rig.real_shell() {
        clear(cx, handle, pane, shot, "clear").await?;
        run(cx, handle, pane, shot, "ls --color", LS_MARKER).await?;
        run(cx, handle, pane, shot, "ls -l --color", LS_LONG_MARKER).await
    } else {
        run(cx, handle, pane, shot, "color", RUSSH_COLOR_MARKER).await?;
        run(cx, handle, pane, shot, "unicode", "wide:").await
    }
}

/// The local terminal's commands: a short prompt and a clear screen, then a
/// colored line and the shell's version.
fn local_commands() -> (&'static str, &'static str) {
    if cfg!(windows) {
        (
            "function prompt { 'PS> ' }; Clear-Host",
            "Write-Host 'Shell++ local terminal' -ForegroundColor Green; $PSVersionTable.PSVersion",
        )
    } else {
        (
            "PS1='$ '; clear",
            "printf '\\033[32mShell++ local terminal\\033[0m\\n'; uname -sr",
        )
    }
}

/// A local terminal tab with its colored line, at the prompt.
async fn local_session(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    shot: ShotState,
) -> Result<(SessionId, Entity<SessionPane>), ScreenshotError> {
    let (id, pane) = open_tab(
        cx,
        handle,
        root,
        SessionRequest::new(SessionKind::Local),
        shot,
    )
    .await?;
    connect(cx, handle, &pane, None, None, shot).await?;
    wait_prompt(cx, handle, &pane, shot).await?;
    let (prompt, demo) = local_commands();
    clear(cx, handle, &pane, shot, prompt).await?;
    run(cx, handle, &pane, shot, demo, LOCAL_MARKER).await?;
    Ok((id, pane))
}

/// The active session pane of the window.
fn active_pane(shell: &Entity<Shell>, cx: &App) -> Option<Entity<SessionPane>> {
    match shell.read(cx).top_tab() {
        TopTab::Session(id) => shell.read(cx).pane(id).cloned(),
        TopTab::Vaults | TopTab::Sftp => None,
    }
}

/// Brings the demo window (after the reset) into the terminal state `shot`.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let core = core_of(cx, state)?;
    set_autoreconnect(&core, true)?;
    match shot {
        ShotState::TerminalTofu => {
            rig.forget_host_keys(&core)?;
            let (_, pane) = open_ssh(cx, handle, root, state, rig, 0, shot).await?;
            wait_until(
                cx,
                handle,
                shot,
                "the host key dialog",
                CONNECT_TIMEOUT,
                |cx| {
                    Ok(matches!(
                        pane.read(cx).host_key(),
                        Some(HostKeyAsk::Unknown(_))
                    ))
                },
            )
            .await
        }
        ShotState::TerminalLsColor => {
            let (_, pane) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
            colored_output(cx, handle, &pane, rig, shot).await
        }
        ShotState::TerminalHtop => {
            let (_, pane) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
            let (command, marker) = if rig.real_shell() {
                ("htop", HTOP_MARKER)
            } else {
                ("alt-screen", RUSSH_ALT_MARKER)
            };
            type_line(cx, handle, &pane, command)?;
            wait_until(
                cx,
                handle,
                shot,
                "the full-screen program",
                STEP_TIMEOUT,
                |cx| Ok(shows(&pane, marker, cx)),
            )
            .await?;
            sleep(cx, HTOP_DRAW).await;
            Ok(())
        }
        ShotState::TerminalDrop => {
            // Autoreconnect off: the overlay stays (UI 4.6); the session
            // layer reads the setting when the connection drops.
            set_autoreconnect(&core, false)?;
            let result = async {
                let (_, pane) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
                colored_output(cx, handle, &pane, rig, shot).await?;
                rig.server
                    .borrow_mut()
                    .drop_connections()
                    .map_err(|error| server_error("dropping connections", error))?;
                wait_until(
                    cx,
                    handle,
                    shot,
                    "the dropped session",
                    STEP_TIMEOUT,
                    |cx| Ok(matches!(pane.read(cx).phase(), Phase::Dropped(_))),
                )
                .await
            }
            .await;
            set_autoreconnect(&core, true)?;
            result
        }
        ShotState::TerminalLocal => local_session(cx, handle, root, shot).await.map(|_| ()),
        ShotState::TerminalTabs => {
            let (first, pane) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
            colored_output(cx, handle, &pane, rig, shot).await?;
            for index in 1..HOST_LABELS.len() {
                ssh_session(cx, handle, root, state, rig, index, shot).await?;
            }
            local_session(cx, handle, root, shot).await?;
            let shell = shell_of(cx, root, shot)?;
            cx.update(|cx| {
                shell.update(cx, |shell, cx| {
                    shell.set_top_tab(TopTab::Session(first), cx)
                })
            });
            wait_prompt(cx, handle, &pane, shot).await
        }
        ShotState::QuickConnectToast => {
            // `user@address:port` of the test server in the search bar,
            // Enter: a quick connect (UI 2.1). It has no saved password, so
            // the password prompt gets the server's; once the session runs,
            // the shell offers to save the host.
            let (target, password) = {
                let server = rig.server.borrow();
                let target = QuickTarget {
                    username: Some(server.user.clone()),
                    host: server.addr.ip().to_string(),
                    port: Some(server.addr.port()),
                };
                (target, server.password.clone())
            };
            let text = crate::shell::display_address(
                target.username.as_deref(),
                &target.host,
                target.port,
            );
            type_into(cx, handle, SEARCH, &text)?;
            press(cx, handle, "enter")?;
            settle(cx, handle).await?;
            let shell = shell_of(cx, root, shot)?;
            let pane = cx
                .update(|cx| active_pane(&shell, cx))
                .ok_or_else(|| ScreenshotError::state(shot, "no quick-connect tab"))?;
            let address = cx.update(|cx| pane.read(cx).info().address().cloned());
            if address.as_deref() != Some(text.as_str()) {
                return Err(ScreenshotError::state(shot, "not a tab to the test server"));
            }
            connect(cx, handle, &pane, Some(rig), Some(&password), shot).await?;
            // The toast goes after 5 seconds: no waiting for a prompt here.
            if visible_now(cx, handle, SAVE_HOST_OFFER)? {
                Ok(())
            } else {
                Err(ScreenshotError::state(shot, "no offer to save the host"))
            }
        }
        ShotState::TerminalHostKeyChanged => {
            // A saved key first (the first connection's dialog), then new
            // keys on the server.
            if rig.saved_fingerprints(&core)?.is_empty() {
                let (id, _) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
                let shell = shell_of(cx, root, shot)?;
                cx.update(|cx| shell.update(cx, |shell, cx| shell.close_session(id, cx)));
                settle(cx, handle).await?;
            }
            let saved = rig.saved_fingerprints(&core)?;
            let current = rig.server.borrow().host_key_fingerprints.clone();
            if saved
                .iter()
                .any(|fingerprint| current.contains(fingerprint))
            {
                rig.server
                    .borrow_mut()
                    .rotate_host_key()
                    .map_err(|error| server_error("rotating the host key", error))?;
                rig.rotated.set(true);
            }
            let (_, pane) = open_ssh(cx, handle, root, state, rig, 0, shot).await?;
            wait_until(
                cx,
                handle,
                shot,
                "the changed key warning",
                CONNECT_TIMEOUT,
                |cx| {
                    Ok(matches!(
                        pane.read(cx).host_key(),
                        Some(HostKeyAsk::Changed { .. })
                    ))
                },
            )
            .await
        }
        _ => Err(ScreenshotError::state(shot, "not a terminal state")),
    }
}

/// Checks that the window shows the terminal state `shot` (the evidence in
/// the picture), so a missed step fails the run.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    rig: &Rig,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    let real_shell = rig.real_shell();
    let result = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let Some(pane) = active_pane(&shell, cx) else {
                return Err("no session tab is active");
            };
            let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
            let phase = pane.read(cx).phase().clone();
            let running = phase == Phase::Running;
            match shot {
                ShotState::TerminalTofu => {
                    need(
                        matches!(pane.read(cx).host_key(), Some(HostKeyAsk::Unknown(_))),
                        "no first-connection question",
                    )?;
                    need(visible(window, "host-key-unknown"), "no host key dialog")?;
                    need(visible(window, "host-key-accept"), "no \"Accept and save\"")
                }
                ShotState::TerminalLsColor => {
                    need(running, "the session does not run")?;
                    let marker = if real_shell { LS_LONG_MARKER } else { "wide:" };
                    need(shows(&pane, marker, cx), "the output is not on screen")
                }
                ShotState::TerminalHtop => {
                    need(running, "the session does not run")?;
                    let marker = if real_shell {
                        HTOP_MARKER
                    } else {
                        RUSSH_ALT_MARKER
                    };
                    need(shows(&pane, marker, cx), "the program is not on screen")
                }
                ShotState::TerminalDrop => {
                    need(
                        matches!(phase, Phase::Dropped(_)),
                        "the session did not drop",
                    )?;
                    need(visible(window, "session-dropped"), "no drop overlay")?;
                    need(visible(window, "session-dropped-reconnect"), "no Reconnect")?;
                    need(visible(window, "session-dropped-close"), "no Close")
                }
                ShotState::TerminalLocal => {
                    need(pane.read(cx).info().is_local(), "not a local terminal")?;
                    need(running, "the shell does not run")?;
                    need(
                        shows(&pane, LOCAL_MARKER, cx),
                        "the output is not on screen",
                    )
                }
                ShotState::TerminalTabs => {
                    let tabs = shell.read(cx).sessions().len();
                    need(tabs == HOST_LABELS.len() + 1, "not four session tabs")?;
                    need(
                        shell.read(cx).sessions().tabs().first().map(|tab| tab.id())
                            == match shell.read(cx).top_tab() {
                                TopTab::Session(id) => Some(id),
                                _ => None,
                            },
                        "the first tab is not active",
                    )?;
                    need(running, "the session does not run")
                }
                ShotState::QuickConnectToast => {
                    need(running, "the session does not run")?;
                    need(
                        visible(window, SAVE_HOST_OFFER),
                        "no offer to save the host",
                    )
                }
                ShotState::TerminalHostKeyChanged => {
                    need(
                        matches!(pane.read(cx).host_key(), Some(HostKeyAsk::Changed { .. })),
                        "no changed key question",
                    )?;
                    need(
                        visible(window, "host-key-changed"),
                        "no changed key warning",
                    )?;
                    need(
                        visible(window, "host-key-replace"),
                        "no \"Replace saved key\"",
                    )
                }
                _ => Err("not a terminal state"),
            }
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    result.map_err(|reason| ScreenshotError::state(shot, reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Host key questions are answered only for the test server's own keys,
    /// and "Replace saved key" only once, right after the run rotated them.
    #[test]
    fn host_key_answers_only_for_the_test_server() {
        use crate::session_ui::HostKeyInfo;

        let dir = crate::profile::TempProfile::create("shellpp-shots-host-key").unwrap();
        let path = dir.path().join("server.json");
        let password = crate::profile::random_suffix().unwrap();
        let text = json!({
            "host": "127.0.0.1",
            "port": 49153,
            "host_keys": {"ssh-ed25519": "SHA256:server"},
            "users": {"password": {"name": "pwuser", "password": password}},
        });
        std::fs::write(&path, text.to_string()).unwrap();
        let rig = Rig {
            server: RefCell::new(DevServer::from_server_json(&path).unwrap()),
            vault_id: Uuid::nil(),
            hosts: Vec::new(),
            rotated: Cell::new(false),
        };
        let info = |fingerprint: &str| HostKeyInfo {
            address: "[127.0.0.1]:49153".into(),
            key_type: "ssh-ed25519".into(),
            fingerprint: fingerprint.into(),
        };
        let unknown = |fingerprint| HostKeyAsk::Unknown(info(fingerprint));
        let changed = |fingerprint| HostKeyAsk::Changed {
            key: info(fingerprint),
            old_key_type: "ssh-ed25519".into(),
            old_fingerprint: "SHA256:old".into(),
        };

        assert_eq!(
            host_key_button(Some(&rig), &unknown("SHA256:server")),
            Ok("host-key-accept")
        );
        assert!(host_key_button(Some(&rig), &unknown("SHA256:other")).is_err());
        assert!(host_key_button(None, &unknown("SHA256:server")).is_err());
        assert!(
            host_key_button(Some(&rig), &changed("SHA256:server")).is_err(),
            "no rotation by this run"
        );
        rig.rotated.set(true);
        assert!(host_key_button(Some(&rig), &changed("SHA256:other")).is_err());
        assert_eq!(
            host_key_button(Some(&rig), &changed("SHA256:server")),
            Ok("host-key-replace")
        );
        assert!(
            host_key_button(Some(&rig), &changed("SHA256:server")).is_err(),
            "one rotation, one replace"
        );
    }

    #[test]
    fn prompts_and_rows() {
        let rows =
            |list: &[&str]| -> Vec<String> { list.iter().map(|row| (*row).to_owned()).collect() };
        assert_eq!(
            last_row(&rows(&["a", "pwuser@shellpp-openssh:~$", "", "  "])),
            Some("pwuser@shellpp-openssh:~$")
        );
        assert_eq!(last_row(&rows(&["", ""])), None);
        assert!(is_prompt("pwuser@shellpp-openssh:~$"));
        assert!(is_prompt("PS>"));
        assert!(is_prompt("PS C:\\x>"));
        assert!(!is_prompt("deploy.sh"));
        let (prompt, demo) = local_commands();
        assert!(!prompt.is_empty());
        assert!(demo.contains(LOCAL_MARKER));
    }

    #[test]
    fn terminal_files_depend_on_the_server() {
        use super::super::{MAIN_SIZE, file_name_on};
        assert_eq!(
            file_name_on(MAIN_SIZE, "dark", ShotState::TerminalLsColor, true),
            "1280x800-dark-terminal-ls-color.png"
        );
        assert_eq!(
            file_name_on(MAIN_SIZE, "light", ShotState::TerminalLsColor, false),
            "1280x800-light-terminal-colors.png"
        );
        assert_eq!(
            file_name_on(MAIN_SIZE, "dark", ShotState::TerminalHtop, false),
            "1280x800-dark-terminal-alt-screen.png"
        );
        assert_eq!(
            file_name_on(MAIN_SIZE, "dark", ShotState::TerminalTabs, false),
            "1280x800-dark-terminal-tabs.png"
        );
    }
}
