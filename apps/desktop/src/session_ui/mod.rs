//! The content of a session tab (UI 4.1, 4.3, 4.6, 4.8, 10.2): the
//! connection screen with its steps and Cancel, the prompt and host key
//! dialogs, the failure state of an attempt, the terminal, and over the
//! kept output the end and drop overlays with Reconnect, Close and the
//! automatic reconnection countdown, plus the multi-line paste confirmation.
//!
//! A [`SessionPane`] owns its terminal view (`crate::terminal`) and starts
//! its session through the [`Launcher`] global ([`bridge::Bridge`], the
//! session layer of `crate::sessions`, in the app; a fake in tests). The
//! session reports to the pane through its setters ([`SessionPane::set_phase`],
//! [`SessionPane::ask_prompt`], ...), and the pane answers through the
//! [`SessionControl`] the launcher returned. The pane itself never touches
//! the network and never logs addresses, usernames, secrets or terminal
//! data; the output lives in the terminal.
//!
//! Exit and drop (UI 4.6): exit status 0 ends in [`Phase::Closed`] and the
//! pane asks the shell to close the tab ([`SessionPaneEvent::Close`]; the
//! shell applies the neighbor rule). A non-zero status keeps the output with
//! "Session ended with code N", Reconnect and Close, and never reconnects by
//! itself. A transport drop keeps the output with the reason, Reconnect and
//! Close; the session layer runs the automatic attempts (Autoreconnect,
//! 1, 2, 4, 8, 16, 32 s, then every 60 s; waiting while locked) and the pane
//! shows its countdown with a Cancel that stops them. A failed first attempt
//! shows the failure screen with Retry and Close (UI 4.8).

pub mod bridge;
pub mod dialog;
pub mod host_key;
pub mod model;
mod overlays;
mod paste;
pub mod prompt;
mod screens;
mod signature;
pub mod strings;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, Global, IntoElement, Pixels,
    SharedString, Size, Subscription, Task, Window, canvas, div, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};

pub use model::{
    ConnectStep, ExitStatus, Failure, HostKeyAsk, HostKeyInfo, HostKeyReply, Keep, Phase, Prompt,
    PromptReply, ReconnectInfo, SessionControl,
};
pub use prompt::PromptForm;

use crate::sessions::OpenRequest;
use crate::settings::terminal::{BuiltinTheme, TerminalPrefs};
use crate::terminal::input::BackspaceKey;
use crate::terminal::{PendingPaste, TerminalEvent, TerminalHandle, TerminalOptions, TerminalView};

/// Registers the keys of session dialogs.
pub fn bind_keys(cx: &mut App) {
    dialog::bind_keys(cx);
    host_key::bind_keys(cx);
}

/// Starts the session of a new tab. The app installs the session layer's
/// ([`bridge::Bridge`]); tests install a fake.
pub trait SessionLauncher: 'static {
    /// Starts `request` with `terminal` as the session's terminal and
    /// returns the commands of the session. The session reports to the
    /// pane of `cx` on the UI thread (tasks it spawns live in the returned
    /// control). `Err` when the session cannot start at all (the app is
    /// locked, the host is gone, the address is invalid).
    fn launch(
        &self,
        request: OpenRequest,
        terminal: TerminalHandle,
        cx: &mut Context<SessionPane>,
    ) -> Result<Rc<dyn SessionControl>, Failure>;
}

/// The launcher of new session tabs (an app global).
#[derive(Clone)]
pub struct Launcher(pub Rc<dyn SessionLauncher>);

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Launcher")
    }
}

impl Global for Launcher {}

/// How long a split run waits for the shell's prompt before it types the
/// script anyway, and how often it looks (D81).
const RUN_PROMPT_WAIT: Duration = Duration::from_secs(3);
const RUN_PROMPT_POLL: Duration = Duration::from_millis(100);

/// What the tab shows before the session layer knows more.
#[derive(Clone, PartialEq, Eq)]
pub struct TabInfo {
    /// The tab title: the host label, else its address.
    title: SharedString,
    /// What the connection screen shows under the title (`user@host:port`
    /// as known before connecting); `None` for a local terminal.
    address: Option<SharedString>,
    /// The host OS icon, or the kind's icon.
    icon: IconName,
    /// A local terminal.
    local: bool,
}

impl std::fmt::Debug for TabInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabInfo")
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

impl TabInfo {
    /// An SSH session to `address` (the text shown under the title).
    pub fn remote(
        title: impl Into<SharedString>,
        address: impl Into<SharedString>,
        icon: IconName,
    ) -> Self {
        Self {
            title: title.into(),
            address: Some(address.into()),
            icon,
            local: false,
        }
    }

    /// A local terminal.
    pub fn local(title: impl Into<SharedString>, icon: IconName) -> Self {
        Self {
            title: title.into(),
            address: None,
            icon,
            local: true,
        }
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn address(&self) -> Option<&SharedString> {
        self.address.as_ref()
    }

    pub fn icon(&self) -> IconName {
        self.icon
    }

    pub fn is_local(&self) -> bool {
        self.local
    }

    /// The host for the dialogs (the title when there is no address).
    fn host_text(&self) -> SharedString {
        self.address
            .clone()
            .filter(|address| !address.is_empty())
            .unwrap_or_else(|| self.title.clone())
    }
}

/// The first step the connection screen shows.
fn first_step(info: &TabInfo) -> ConnectStep {
    if info.local {
        ConnectStep::OpeningSession
    } else {
        ConnectStep::Resolving
    }
}

/// What the pane asks of its owner (the shell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionPaneEvent {
    /// The tab should close: exit status 0 or the user's Close. The
    /// session is already told to end.
    Close,
    /// The tab's title or icon changed (the shell name of a local
    /// terminal, the detected OS of a host).
    Retitled,
    /// The session runs for the first time (once per tab; reconnects do
    /// not repeat it). A quick connect then offers to save the host (UI 2.1).
    Connected,
}

/// The pending question of the tab.
enum Ask {
    Prompt {
        prompt: Prompt,
        /// Created at the next render (it needs the window).
        form: Option<Entity<PromptForm>>,
    },
    HostKey(HostKeyAsk),
    /// A signature of the forwarded agent: the prompt and the key's
    /// fingerprint (D60).
    Signature {
        id: u64,
        fingerprint: String,
    },
}

/// The content of one session tab.
pub struct SessionPane {
    info: TabInfo,
    request: OpenRequest,
    control: Option<Rc<dyn SessionControl>>,
    phase: Phase,
    /// The presented key of a revoked-key failure (UI 10.2).
    revoked: Option<HostKeyInfo>,
    /// Where on the route the failure happened (a jump host, the proxy, the
    /// chain; stage 5, D57); `None` on the host itself.
    place: Option<SharedString>,
    /// A startup snippet waiting for its values, and the last login asked.
    startup_pending: Option<crate::sessions::StartupAsk>,
    startup_asked: u64,
    /// The login whose startup snippet the session reports now (`None`
    /// once that login ended).
    startup_live: Option<u64>,
    /// A snippet run in a split tab (UI 9.4, D81): typed once, when the
    /// session first runs.
    run_on_connect: Option<String>,
    /// Waits for the shell's prompt before it types that script.
    run_task: Option<Task<()>>,
    reconnect: Option<ReconnectInfo>,
    ask: Option<Ask>,
    paste: Option<PendingPaste>,
    terminal: Option<Entity<TerminalView>>,
    /// The pane's size in its last frame: the terminal is sized from it
    /// before the shell opens (the connection screen hides the terminal).
    area: Rc<Cell<Option<Size<Pixels>>>>,
    /// The built-in terminal theme the session reported last (its slug).
    theme_slug: Option<String>,
    /// The countdown repaints while a reconnect is scheduled.
    ticker: Option<Task<()>>,
    /// The session was told to end (once).
    closed: bool,
    /// [`SessionPaneEvent::Connected`] was sent.
    connected_once: bool,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    /// Move the keyboard focus into the pane at the next render.
    focus_request: bool,
    /// The active pane of its tab (UI 4.7): only it takes the keyboard
    /// focus; a request of another pane waits until it becomes active.
    active: bool,
    /// The window the pane was last drawn in.
    window: Option<gpui::AnyWindowHandle>,
    /// The signature question a system notification was shown for (D68).
    signature_notice: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SessionPane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionPane")
            .field("phase", &self.phase.name())
            .field("ask", &self.ask.is_some())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SessionPaneEvent> for SessionPane {}

impl Focusable for SessionPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.ask {
            Some(Ask::Prompt {
                form: Some(form), ..
            }) => return form.focus_handle(cx),
            Some(_) => return self.dialog_focus.clone(),
            None => {}
        }
        if self.paste.is_some() {
            return self.dialog_focus.clone();
        }
        match &self.terminal {
            Some(terminal) if self.phase.shows_terminal() => terminal.focus_handle(cx),
            _ => self.focus.clone(),
        }
    }
}

impl SessionPane {
    /// A tab for `request`, showing the connection screen at its first step
    /// until [`Self::start`] (the shell calls it at its next render, with
    /// the window the terminal needs).
    pub fn new(info: TabInfo, request: OpenRequest, cx: &mut Context<Self>) -> Self {
        let first = first_step(&info);
        Self {
            info,
            request,
            control: None,
            phase: Phase::Connecting(first),
            revoked: None,
            place: None,
            startup_pending: None,
            startup_asked: 0,
            startup_live: None,
            run_on_connect: None,
            run_task: None,
            window: None,
            signature_notice: None,
            reconnect: None,
            ask: None,
            paste: None,
            terminal: None,
            area: Rc::new(Cell::new(None)),
            theme_slug: None,
            ticker: None,
            closed: false,
            connected_once: false,
            focus: cx.focus_handle(),
            dialog_focus: cx.focus_handle(),
            focus_request: true,
            active: true,
            _subscriptions: Vec::new(),
        }
    }

    /// Creates the terminal and starts the session through the launcher.
    /// Does nothing once started.
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.is_some() || self.closed {
            return;
        }
        let terminal = cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx));
        // The shell opens at the size the tab has: the pane's last frame,
        // else the window below the top bar (a tab opened in the background).
        let area = self.area.get().unwrap_or_else(|| {
            let viewport = window.viewport_size();
            size(
                viewport.width,
                (viewport.height - crate::theme::metrics::HEIGHT_TOP_BAR).max(gpui::px(0.)),
            )
        });
        terminal.read(cx).fit(area, window);
        self._subscriptions
            .push(cx.subscribe_in(&terminal, window, Self::on_terminal_event));
        self.terminal = Some(terminal);
        self.launch(cx);
    }

    /// Starts the session through the launcher with the tab's terminal;
    /// a launch error shows the failure screen.
    fn launch(&mut self, cx: &mut Context<Self>) {
        let Some(terminal) = &self.terminal else {
            return;
        };
        let handle = terminal.read(cx).handle();
        let launcher = cx.try_global::<Launcher>().cloned();
        let result = match launcher {
            Some(launcher) => launcher.0.launch(self.request.clone(), handle, cx),
            None => {
                tracing::warn!("no session layer; the tab cannot connect");
                Err(Failure::Other)
            }
        };
        match result {
            Ok(control) => self.control = Some(control),
            Err(failure) => {
                self.phase = Phase::Failed(failure);
                cx.notify();
            }
        }
    }

    /// The size the pane will have (a new split pane before its first
    /// frame): the terminal starts at it instead of the whole window. A
    /// size measured in a frame wins.
    pub fn set_area_hint(&mut self, area: Size<Pixels>) {
        if self.area.get().is_none() {
            self.area.set(Some(area));
        }
    }

    /// Whether [`Self::start`] ran.
    pub fn is_started(&self) -> bool {
        self.terminal.is_some()
    }

    pub fn info(&self) -> &TabInfo {
        &self.info
    }

    pub fn request(&self) -> &OpenRequest {
        &self.request
    }

    /// A local terminal opened with the default shell learns which shell
    /// that was (`shell_id` of the session view): the pane's request names
    /// it from then on, so Duplicate, a new split pane and Restore open the
    /// same shell (UI 4.4) even when the default changes meanwhile.
    pub fn pin_local_shell(&mut self, shell_id: &str) {
        if let OpenRequest::Local {
            shell_id: id @ None,
        } = &mut self.request
        {
            *id = Some(shell_id.to_owned());
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The tab's terminal, once started.
    pub fn terminal(&self) -> Option<&Entity<TerminalView>> {
        self.terminal.as_ref()
    }

    /// The pending prompt, if any.
    pub fn prompt(&self) -> Option<&Prompt> {
        match &self.ask {
            Some(Ask::Prompt { prompt, .. }) => Some(prompt),
            _ => None,
        }
    }

    /// The prompt form, once rendered.
    pub fn prompt_form(&self) -> Option<&Entity<PromptForm>> {
        match &self.ask {
            Some(Ask::Prompt { form, .. }) => form.as_ref(),
            _ => None,
        }
    }

    /// The pending host key question, if any.
    pub fn host_key(&self) -> Option<&HostKeyAsk> {
        match &self.ask {
            Some(Ask::HostKey(ask)) => Some(ask),
            _ => None,
        }
    }

    /// The paste waiting for confirmation, if any.
    pub fn pending_paste(&self) -> Option<&PendingPaste> {
        self.paste.as_ref()
    }

    /// The scheduled automatic reconnection, if any.
    pub fn reconnect_info(&self) -> Option<&ReconnectInfo> {
        self.reconnect.as_ref()
    }

    /// Seconds until the next automatic attempt (rounded up), if one is
    /// scheduled.
    pub fn reconnect_countdown(&self) -> Option<u64> {
        self.reconnect
            .as_ref()
            .and_then(|info| info.seconds_left(std::time::Instant::now()))
    }

    /// The tab's title and icon changed (the shell name of a local
    /// terminal, the detected OS).
    pub fn set_info(&mut self, title: SharedString, icon: IconName, cx: &mut Context<Self>) {
        if self.info.title != title || self.info.icon != icon {
            self.info.title = title;
            self.info.icon = icon;
            cx.emit(SessionPaneEvent::Retitled);
            cx.notify();
        }
    }

    /// The terminal theme of the session (UI 4.1): the host's own theme, or
    /// the default theme of Settings > Terminal (its built-in slug). A
    /// theme equal to the current default follows later changes of the
    /// default; another one stays with the tab.
    pub fn set_theme_slug(&mut self, slug: &str, cx: &mut Context<Self>) {
        if self.theme_slug.as_deref() == Some(slug) {
            return;
        }
        self.theme_slug = Some(slug.to_owned());
        let default = TerminalPrefs::current(cx).default_theme;
        let theme = BuiltinTheme::from_slug(slug)
            .filter(|theme| *theme != default)
            .and_then(BuiltinTheme::theme);
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, cx| terminal.set_theme(theme, cx));
        }
    }

    /// What plain Backspace sends in the terminal (the host's setting,
    /// UI 3.1).
    pub fn set_backspace(&mut self, backspace: BackspaceKey, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, _| terminal.set_backspace(backspace));
        }
    }

    /// The theme slug last reported by the session, if any.
    pub fn theme_slug(&self) -> Option<&str> {
        self.theme_slug.as_deref()
    }

    /// Types `script` (and Enter) once, when the session first runs: a
    /// snippet run in a split tab (UI 9.4, D81).
    pub fn run_on_connect(&mut self, script: String) {
        self.run_on_connect = Some(script);
    }

    /// The session layer reports a new phase (with the drop's automatic
    /// reconnection, if any).
    pub fn set_phase(
        &mut self,
        phase: Phase,
        reconnect: Option<ReconnectInfo>,
        cx: &mut Context<Self>,
    ) {
        if self.closed {
            return;
        }
        if phase == Phase::Closed {
            self.phase = phase;
            self.close(cx);
            return;
        }
        if !matches!(phase, Phase::Failed(Failure::HostKeyRevoked)) {
            self.revoked = None;
        }
        if !matches!(phase, Phase::Connecting(_) | Phase::Reconnecting(_)) {
            self.ask = None;
        }
        if phase.shows_terminal() != self.phase.shows_terminal() || phase == Phase::Running {
            self.focus_request = true;
        }
        self.phase = phase;
        // UI 1.1: a dropped session is dimmed without a cursor until it runs
        // again.
        let dimmed = matches!(self.phase, Phase::Dropped(_) | Phase::Reconnecting(_));
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, cx| terminal.set_dimmed(dimmed, cx));
        }
        let first_run = self.phase == Phase::Running && !self.connected_once;
        if first_run {
            self.connected_once = true;
            cx.emit(SessionPaneEvent::Connected);
            // A snippet run in a split tab: the script and Enter, as the
            // terminal sidebar's Run sends them, once the shell waits at its
            // prompt (bracketed paste on), else after RUN_PROMPT_WAIT.
            if let Some(script) = self.run_on_connect.take() {
                self.run_task = Some(cx.spawn(async move |this, cx| {
                    let polls = RUN_PROMPT_WAIT.as_millis() / RUN_PROMPT_POLL.as_millis();
                    for _ in 0..polls {
                        let ready = this
                            .update(cx, |this, cx| {
                                this.terminal
                                    .as_ref()
                                    .is_some_and(|terminal| terminal.read(cx).at_prompt())
                            })
                            .unwrap_or(true);
                        if ready {
                            break;
                        }
                        cx.background_executor().timer(RUN_PROMPT_POLL).await;
                    }
                    let _ = this.update(cx, |this, cx| {
                        if this.phase != Phase::Running {
                            return;
                        }
                        if let Some(terminal) = &this.terminal {
                            terminal.update(cx, |terminal, cx| {
                                terminal.insert_snippet(&script, true, cx);
                            });
                        }
                    });
                }));
            }
        }
        self.set_reconnect(reconnect, cx);
        cx.notify();
    }

    /// The presented key of a revoked-key failure (shown on the failure
    /// screen).
    /// Where on the route the shown failure happened.
    /// The startup snippet of a login waits for its values: asked once per
    /// login, while the pane is active (see `render`).
    pub fn set_startup_ask(
        &mut self,
        ask: Option<&crate::sessions::StartupAsk>,
        cx: &mut Context<Self>,
    ) {
        self.startup_live = ask.map(|ask| ask.login);
        match ask {
            Some(ask)
                if ask.login > self.startup_asked
                    && self
                        .startup_pending
                        .as_ref()
                        .is_none_or(|pending| pending.login != ask.login) =>
            {
                self.startup_pending = Some(ask.clone());
                cx.notify();
            }
            // The login ended (a drop, an end, a new attempt): not asked.
            None => self.startup_pending = None,
            Some(_) => {}
        }
    }

    /// Whether a startup snippet waits for its values form.
    pub fn startup_pending(&self) -> bool {
        self.startup_pending.is_some()
    }

    /// The shell of login `login` still runs here (its values may be sent).
    fn startup_current(&self, login: u64) -> bool {
        self.phase == Phase::Running && self.startup_live == Some(login)
    }

    pub fn set_place(&mut self, place: Option<SharedString>, cx: &mut Context<Self>) {
        if self.place != place {
            self.place = place;
            cx.notify();
        }
    }

    pub fn place(&self) -> Option<&SharedString> {
        self.place.as_ref()
    }

    pub fn set_revoked_key(&mut self, key: Option<HostKeyInfo>, cx: &mut Context<Self>) {
        self.revoked = key;
        cx.notify();
    }

    fn set_reconnect(&mut self, reconnect: Option<ReconnectInfo>, cx: &mut Context<Self>) {
        let ticking = reconnect
            .as_ref()
            .is_some_and(|info| info.next_at.is_some());
        self.reconnect = reconnect;
        if !ticking {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        // Repaints the countdown every second while it runs.
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this.update(cx, |this, cx| {
                    cx.notify();
                    this.reconnect
                        .as_ref()
                        .is_some_and(|info| info.next_at.is_some())
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// The session layer asks the user a [`Prompt`] (UI 4.8).
    pub fn ask_prompt(&mut self, prompt: Prompt, cx: &mut Context<Self>) {
        if let Some(Ask::Prompt { prompt: shown, .. }) = &self.ask
            && *shown == prompt
        {
            return;
        }
        self.ask = Some(Ask::Prompt { prompt, form: None });
        self.focus_request = true;
        cx.notify();
    }

    /// The session layer asks about the host key (UI 10.2).
    pub fn ask_host_key(&mut self, ask: HostKeyAsk, cx: &mut Context<Self>) {
        if matches!(&self.ask, Some(Ask::HostKey(shown)) if *shown == ask) {
            return;
        }
        self.ask = Some(Ask::HostKey(ask));
        self.focus_request = true;
        cx.notify();
    }

    /// The forwarded agent asks to sign with the key of `fingerprint`
    /// (Agent Forwarding, "Ask before every signature", D60).
    pub fn ask_signature(&mut self, id: u64, fingerprint: String, cx: &mut Context<Self>) {
        // Each request is its own dialog, even for the same key.
        if matches!(&self.ask, Some(Ask::Signature { id: shown, .. }) if *shown == id) {
            return;
        }
        self.ask = Some(Ask::Signature { id, fingerprint });
        self.focus_request = true;
        self.notify_signature(id, cx);
        cx.notify();
    }

    /// While the pane's window is not focused, the signature question is
    /// also a system notification naming the host (D68): nothing of the
    /// key is in it. With no window known yet, any focused window of the
    /// app counts. Once per question: the same question shown again (the
    /// session's view changed meanwhile) notifies nothing new.
    fn notify_signature(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.signature_notice == Some(id) {
            return;
        }
        let active = cx.active_window();
        let focused = match self.window {
            Some(window) => active.is_some_and(|active| active.window_id() == window.window_id()),
            None => active.is_some(),
        };
        if focused {
            return;
        }
        cx.show_system_notification(gpui::SystemNotification {
            tag: signature_tag(cx.entity_id()),
            title: strings::sign_notice_title(self.info.title()).into(),
            body: strings::SIGN_NOTICE_BODY.into(),
            actions: Vec::new(),
        });
        self.signature_notice = Some(id);
    }

    /// The question was answered or withdrawn, or the tab closes: its
    /// notification goes too.
    fn withdraw_signature_notice(&mut self, cx: &mut Context<Self>) {
        if self.signature_notice.take().is_some() {
            cx.dismiss_system_notification(&signature_tag(cx.entity_id()));
        }
    }

    /// The fingerprint of the signature question on display.
    pub fn signature_asked(&self) -> Option<&str> {
        match &self.ask {
            Some(Ask::Signature { fingerprint, .. }) => Some(fingerprint),
            _ => None,
        }
    }

    /// Allow or Deny of the signature question.
    pub fn answer_signature(&mut self, allow: bool, cx: &mut Context<Self>) {
        if !matches!(self.ask, Some(Ask::Signature { .. })) {
            return;
        }
        self.ask = None;
        self.focus_request = true;
        self.withdraw_signature_notice(cx);
        if let Some(control) = self.control.clone() {
            let reply = if allow {
                PromptReply::Allow
            } else {
                PromptReply::Cancel
            };
            control.reply_prompt(reply, cx);
        }
        cx.notify();
    }

    /// The session layer withdrew its question.
    pub fn clear_ask(&mut self, cx: &mut Context<Self>) {
        self.withdraw_signature_notice(cx);
        if self.ask.take().is_some() {
            self.focus_request = true;
            cx.notify();
        }
    }

    /// Whether this is the active pane of its tab (UI 4.7). An inactive
    /// pane never takes the keyboard focus by itself (a session that
    /// connects or asks a question in another pane leaves the focus where
    /// it is).
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            cx.notify();
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Asks for the focus at the next render (the tab became active).
    pub fn request_focus(&mut self, cx: &mut Context<Self>) {
        self.focus_request = true;
        cx.notify();
    }

    /// Moves the keyboard focus into the tab now: the dialog when one is
    /// open, else the terminal (UI 4.1), else the pane.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Ends the session (the tab is closing). Safe to call more than once.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.ticker = None;
        self.ask = None;
        self.withdraw_signature_notice(cx);
        self.paste = None;
        if let Some(control) = self.control.clone() {
            control.close(cx);
        }
    }

    /// The tab closes or the app quits: ends the session and lets go of it
    /// (the session's commands and the terminal's link to it), so nothing
    /// keeps the core open after the session layer shuts down.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.shutdown(cx);
        self.control = None;
        if let Some(terminal) = &self.terminal {
            let handle = terminal.read(cx).handle();
            handle.set_sink(None);
            // The history recorder holds the session too (stage 2b).
            handle.set_history_sink(None);
        }
    }

    /// Close: ends the session and asks the shell to close the tab.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.shutdown(cx);
        cx.emit(SessionPaneEvent::Close);
        cx.notify();
    }

    /// Cancel on the connection screen, on a reconnect attempt, or of the
    /// countdown: stops the attempt and the automatic attempts.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.ask.take().is_some() {
            self.focus_request = true;
        }
        if let Some(control) = self.control.clone() {
            control.cancel(cx);
        }
        cx.notify();
    }

    /// Retry / Reconnect / Reconnect now. When the session never started
    /// (the launch failed, for example while the app was locked), Retry
    /// launches it again.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        match self.control.clone() {
            Some(control) => control.reconnect(cx),
            None => match self.phase {
                Phase::Failed(failure)
                    if failure.can_retry() && !self.closed && self.terminal.is_some() =>
                {
                    self.phase = Phase::Connecting(first_step(&self.info));
                    self.focus_request = true;
                    self.launch(cx);
                }
                _ => tracing::debug!("reconnect without a session"),
            },
        }
        cx.notify();
    }

    fn answer_prompt(&mut self, reply: PromptReply, cx: &mut Context<Self>) {
        if !matches!(self.ask, Some(Ask::Prompt { .. })) {
            return;
        }
        self.ask = None;
        self.focus_request = true;
        if let Some(control) = self.control.clone() {
            control.reply_prompt(reply, cx);
        }
        cx.notify();
    }

    /// The choice in a host key dialog.
    pub fn answer_host_key(&mut self, reply: HostKeyReply, cx: &mut Context<Self>) {
        let Some(Ask::HostKey(ask)) = &self.ask else {
            return;
        };
        // A first connection never "replaces"; a changed key continues only
        // by "Replace saved key" (UI 10.2).
        let allowed = matches!(
            (ask, reply),
            (_, HostKeyReply::Cancel)
                | (
                    HostKeyAsk::Unknown(_),
                    HostKeyReply::AcceptAndSave | HostKeyReply::ConnectOnce
                )
                | (HostKeyAsk::Changed { .. }, HostKeyReply::Replace)
        );
        if !allowed {
            return;
        }
        self.ask = None;
        self.focus_request = true;
        if let Some(control) = self.control.clone() {
            control.reply_host_key(reply, cx);
        }
        cx.notify();
    }

    /// Shows the paste confirmation (UI 4.3).
    pub fn confirm_paste(&mut self, paste: PendingPaste, cx: &mut Context<Self>) {
        self.paste = Some(paste);
        self.focus_request = true;
        cx.notify();
    }

    /// "Paste" of the confirmation: the terminal sends it (bracketed when
    /// the program asked for it) and takes the focus back.
    pub fn accept_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paste) = self.paste.take() else {
            return;
        };
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, cx| {
                terminal.paste_confirmed(&paste, window, cx)
            });
        }
        self.focus_request = true;
        cx.notify();
    }

    /// "Cancel" of the confirmation: nothing is pasted.
    pub fn cancel_paste(&mut self, cx: &mut Context<Self>) {
        if self.paste.take().is_some() {
            self.focus_request = true;
            cx.notify();
        }
    }

    fn on_terminal_event(
        &mut self,
        _: &Entity<TerminalView>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::ConfirmPaste(paste) => self.confirm_paste(paste.clone(), cx),
            TerminalEvent::Bell => {
                if crate::terminal::settings::settings(cx).bell_sound {
                    window.play_system_bell();
                }
            }
            TerminalEvent::TitleChanged(_) => {}
        }
    }

    fn render_content(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        match &self.phase {
            Phase::Connecting(step) => {
                screens::connecting(&self.info, *step, self.focus.clone(), cx).into_any_element()
            }
            Phase::Failed(reason) => screens::failed(
                &self.info,
                *reason,
                self.place.clone(),
                self.revoked.as_ref(),
                self.focus.clone(),
                cx,
            )
            .into_any_element(),
            _ => div()
                .id("session-terminal")
                .test_support()
                .size_full()
                .track_focus(&self.focus)
                .children(self.terminal.clone())
                .into_any_element(),
        }
    }
}

/// The tag of the signature notification of the pane `entity`.
pub fn signature_tag(entity: gpui::EntityId) -> SharedString {
    format!("agent-signature-{}", entity.as_u64()).into()
}

impl Render for SessionPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window = Some(window.window_handle());
        // The prompt form needs the window: created here, once per prompt.
        if let Some(Ask::Prompt { prompt, form }) = &mut self.ask
            && form.is_none()
        {
            let target = self.info.host_text();
            let this = cx.weak_entity();
            let on_reply: prompt::ReplyHandler = Rc::new(move |reply, _, cx| {
                let _ = this.update(cx, |this, cx| this.answer_prompt(reply, cx));
            });
            let prompt = prompt.clone();
            *form = Some(cx.new(|cx| PromptForm::new(prompt, target, on_reply, window, cx)));
        }
        // A startup snippet's values (UI 3.1): asked in the active pane,
        // not over another dialog; the filled script goes to the terminal
        // as injected input with Enter.
        if self.active
            && self.phase == Phase::Running
            && !window.has_active_dialog(cx)
            && let Some(ask) = self.startup_pending.take()
            && let Some(terminal) = self.terminal.clone()
        {
            self.startup_asked = ask.login;
            let pane = cx.weak_entity();
            let login = ask.login;
            window.defer(cx, move |window, cx| {
                crate::snippets::ask_script_values(
                    ask.script.clone(),
                    &ask.variables,
                    window,
                    cx,
                    move |script, _, cx| {
                        // Only into the shell of the login it was asked for.
                        let current = pane
                            .upgrade()
                            .is_some_and(|pane| pane.read(cx).startup_current(login));
                        if current {
                            terminal.update(cx, |terminal, cx| {
                                terminal.insert_snippet(&script, true, cx);
                            });
                        }
                    },
                );
            });
        }
        // Not from under a modal kit dialog (Settings, a confirmation), nor
        // from the Rename field of the tab: the request waits until it is
        // closed.
        if self.focus_request
            && self.active
            && !window.has_active_dialog(cx)
            && !crate::shell::pane_focus_held(window, cx)
        {
            self.focus_request = false;
            let handle = self.focus_handle(cx);
            // The changed-key dialog starts on Cancel: from its frame, the
            // next focus stop (the frame drawn this frame) is its first
            // button (UI 10.2).
            let first_button = self.host_key().is_some_and(host_key::focuses_first_button);
            window.defer(cx, move |window, cx| {
                window.focus(&handle, cx);
                if first_button {
                    window.focus_next(cx);
                }
            });
        }
        let content = self.render_content(cx);
        let overlay = overlays::render(self, cx);
        let dialog = match &self.ask {
            Some(Ask::Prompt { form, .. }) => form.clone().map(|form| form.into_any_element()),
            Some(Ask::HostKey(ask)) => {
                Some(host_key::render(ask, &self.dialog_focus, cx).into_any_element())
            }
            Some(Ask::Signature { fingerprint, .. }) => Some(
                signature::render(fingerprint.clone(), &self.dialog_focus, cx).into_any_element(),
            ),
            None => None,
        };
        let paste = self
            .paste
            .as_ref()
            .filter(|_| self.ask.is_none())
            .map(|paste| paste::render(paste, &self.dialog_focus, cx).into_any_element());
        // Measures the pane; while a screen hides the terminal, the terminal
        // is sized from it, so the shell opens at the tab's size.
        let area = self.area.clone();
        let hidden_terminal = self
            .terminal
            .clone()
            .filter(|_| !self.phase.shows_terminal());
        let measure = canvas(
            move |bounds, window, cx| {
                area.set(Some(bounds.size));
                if let Some(terminal) = &hidden_terminal {
                    terminal.read(cx).fit(bounds.size, window);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("session-pane")
            .test_support()
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .child(measure)
            .child(content)
            .children(overlay)
            .children(dialog)
            .children(paste)
    }
}
