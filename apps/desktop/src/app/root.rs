//! The root view of a window: the lock screens until the core is unlocked,
//! then the shell. It also reports user input to the auto-lock timer and
//! shows a startup error instead of the app when the core did not open.
//!
//! Input for auto-lock (UI 13.3): every key press (an app-wide keystroke
//! interceptor, so kit dialogs count too), mouse button and scroll wheel
//! anywhere in the window (window-level listeners in the capture phase,
//! which need no hit test, so overlays, menus and the title bar count too).
//! When the interval has already passed (the first input after a sleep),
//! the input locks instead, through [`shell::lock_all`], so pending panel
//! edits are saved first; a due auto-lock from the tick goes the same way.
//!
//! Locking (UI 13.3, lead decision 8): the shell is kept, so session tabs
//! stay alive, but the window shows only the unlock screen: the shell is not
//! rendered, its details panel is closed (the record data in it is
//! dropped) and open kit dialogs (Settings, confirmations) are closed. After
//! unlock the same shell comes back with its tabs, the open group and the
//! details panel of the same record. A new vault ("Forgot password?") or a
//! first run drops the shell.

use gpui::{
    App, AppContext as _, Context, DispatchPhase, Entity, IntoElement, MouseDownEvent,
    ParentElement as _, Render, ScrollWheelEvent, Styled as _, Subscription, Window, canvas, div,
    prelude::*,
};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use shellpp_core::service::dto::LockState;

use crate::lock::{FirstRunScreen, UnlockScreen};
use crate::shell::{self, Shell};
use crate::state::{AppState, AppStateEvent};
use crate::theme::{self, metrics};

/// Registers the key bindings of every view.
pub fn bind_keys(cx: &mut App) {
    shell::bind_keys(cx);
}

/// What the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    StartupError,
    FirstRun,
    Unlock,
    Shell,
}

pub struct RootView {
    state: Entity<AppState>,
    /// Kept while locked (session tabs stay alive), dropped at first run.
    shell: Option<Entity<Shell>>,
    first_run: Option<Entity<FirstRunScreen>>,
    unlock: Option<Entity<UnlockScreen>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for RootView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootView").finish_non_exhaustive()
    }
}

impl RootView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe_in(
            &state,
            window,
            |this: &mut Self, _, event: &AppStateEvent, window, cx| match event {
                AppStateEvent::LockChanged(state) => {
                    if *state != LockState::Unlocked {
                        // Nothing stays over the lock screens.
                        Self::close_overlays(window, cx);
                    }
                    this.sync_screens(window, cx);
                }
                AppStateEvent::AutoLockDue => shell::lock_all(&this.state, cx),
                _ => {}
            },
        );
        // Key presses in any window, before any view handles them.
        let keys = {
            let state = state.downgrade();
            cx.intercept_keystrokes(move |_, _, cx| {
                if let Some(state) = state.upgrade()
                    && user_input(&state, cx)
                {
                    cx.stop_propagation();
                }
            })
        };
        // Auto appearance keeps following the system while the app runs.
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            theme::follow_system(window, cx);
        });
        let mut this = Self {
            state,
            shell: None,
            first_run: None,
            unlock: None,
            _subscriptions: vec![subscription, appearance, keys],
        };
        this.sync_screens(window, cx);
        this
    }

    /// The app state this window shows.
    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    /// The shell: on display while unlocked, kept hidden while locked.
    pub fn shell(&self) -> Option<&Entity<Shell>> {
        self.shell.as_ref()
    }

    /// The unlock screen, while locked.
    pub fn unlock_screen(&self) -> Option<&Entity<UnlockScreen>> {
        self.unlock.as_ref()
    }

    /// The first-run screen, at first run.
    pub fn first_run_screen(&self) -> Option<&Entity<FirstRunScreen>> {
        self.first_run.as_ref()
    }

    /// The screen on display.
    pub fn screen(&self, cx: &App) -> Screen {
        let state = self.state.read(cx);
        if state.open_error().is_some() {
            return Screen::StartupError;
        }
        match state.lock_state() {
            LockState::Unlocked => Screen::Shell,
            LockState::FirstRun => Screen::FirstRun,
            LockState::Locked => Screen::Unlock,
        }
    }

    /// Creates and drops the screens for the current lock state.
    fn sync_screens(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if state.open_error().is_some() {
            return;
        }
        match state.lock_state() {
            LockState::FirstRun => {
                self.shell = None;
                self.unlock = None;
                if self.first_run.is_none() {
                    self.first_run =
                        Some(cx.new(|cx| FirstRunScreen::new(self.state.clone(), window, cx)));
                }
            }
            LockState::Locked => {
                self.first_run = None;
                if let Some(shell) = &self.shell {
                    // Its record data goes; its session tabs stay.
                    shell.update(cx, |shell, cx| shell.suspend_panel(cx));
                }
                if self.unlock.is_none() {
                    self.unlock =
                        Some(cx.new(|cx| UnlockScreen::new(self.state.clone(), window, cx)));
                }
            }
            LockState::Unlocked => {
                self.unlock = None;
                self.first_run = None;
                match &self.shell {
                    Some(shell) => {
                        let focus = shell.read(cx).focus_handle().clone();
                        window.focus(&focus, cx);
                        shell.update(cx, |shell, cx| shell.resume_panel(window, cx));
                    }
                    None => {
                        self.shell = Some(cx.new(|cx| Shell::new(self.state.clone(), window, cx)));
                    }
                }
            }
        }
        cx.notify();
    }

    /// Closes kit dialogs and sheets: nothing stays over the lock screens.
    /// Only after the window exists (the kit's root is created after this view).
    fn close_overlays(window: &mut Window, cx: &mut App) {
        if window.has_active_dialog(cx) {
            window.close_all_dialogs(cx);
        }
        if window.has_active_sheet(cx) {
            window.close_sheet(cx);
        }
    }

    fn render_startup_error(&self, message: String, cx: &App) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(shell::title_bar::bare_title_bar(cx))
            .child(
                v_flex()
                    .id("startup-error")
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p(metrics::SPACE_6)
                    .text_color(cx.theme().foreground)
                    .child(div().max_w(gpui::px(480.)).child(message)),
            )
    }
}

/// Keyboard or mouse input (UI 13.3): restarts the auto-lock timer, or,
/// when the interval has already passed, locks every window (pending panel
/// edits are saved first). Returns `true` when it locked, so the input
/// goes no further.
fn user_input(state: &Entity<AppState>, cx: &mut App) -> bool {
    if state.read(cx).activity() {
        shell::lock_all(state, cx);
        return true;
    }
    false
}

/// A zero-size element whose paint registers window-level listeners for
/// mouse buttons and the scroll wheel in the capture phase: they run for
/// every such event in the window, whatever element is under the pointer.
fn input_watch(state: Entity<AppState>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let on_button = state.clone();
            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && user_input(&on_button, cx) {
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |_: &ScrollWheelEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && user_input(&state, cx) {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = v_flex()
            .id("root")
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(input_watch(self.state.clone()));

        if let Some(message) = self.state.read(cx).open_error() {
            let message = message.to_owned();
            return root
                .child(self.render_startup_error(message, cx))
                .into_any_element();
        }
        // The lock screens win over the shell: while locked the shell lives
        // on (session tabs) but is not drawn (UI 13.3).
        let content = if let Some(first_run) = &self.first_run {
            first_run.clone().into_any_element()
        } else if let Some(unlock) = &self.unlock {
            unlock.clone().into_any_element()
        } else if let Some(shell) = &self.shell {
            shell.clone().into_any_element()
        } else {
            div().into_any_element()
        };
        root.child(content).into_any_element()
    }
}
