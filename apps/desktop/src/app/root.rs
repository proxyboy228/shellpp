//! The root view of a window: the minimal lock screens until the core is
//! unlocked, then the shell. It also reports user input to the auto-lock
//! timer and shows a startup error instead of the app when the core did not
//! open.

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, prelude::*,
};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use shellpp_core::service::dto::LockState;

use crate::lock::{FirstRunEvent, FirstRunScreen, UnlockScreen};
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
            |this: &mut Self, _, event: &AppStateEvent, window, cx| {
                if let AppStateEvent::LockChanged(_) = event {
                    this.sync_screens(window, cx);
                }
            },
        );
        // Auto appearance keeps following the system while the app runs.
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            theme::follow_system(window, cx);
        });
        let mut this = Self {
            state,
            shell: None,
            first_run: None,
            unlock: None,
            _subscriptions: vec![subscription, appearance],
        };
        this.sync_screens(window, cx);
        this
    }

    /// The app state this window shows.
    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    /// The shell, while unlocked.
    pub fn shell(&self) -> Option<&Entity<Shell>> {
        self.shell.as_ref()
    }

    /// The screen on display.
    pub fn screen(&self, cx: &App) -> Screen {
        let state = self.state.read(cx);
        if state.open_error().is_some() {
            return Screen::StartupError;
        }
        if self.first_run.is_some() {
            return Screen::FirstRun;
        }
        match state.lock_state() {
            LockState::Unlocked => Screen::Shell,
            LockState::FirstRun => Screen::FirstRun,
            LockState::Locked => Screen::Unlock,
        }
    }

    /// Creates and drops the screens for the current lock state. The shell
    /// is dropped while locked, so no view keeps record data.
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
                    let screen = cx.new(|cx| FirstRunScreen::new(self.state.clone(), window, cx));
                    let subscription = cx.subscribe_in(
                        &screen,
                        window,
                        |this: &mut Self, _, event, window, cx| {
                            let FirstRunEvent::Finished = event;
                            this.first_run = None;
                            this.sync_screens(window, cx);
                        },
                    );
                    self._subscriptions.push(subscription);
                    self.first_run = Some(screen);
                }
            }
            LockState::Locked => {
                self.shell = None;
                self.first_run = None;
                if self.unlock.is_none() {
                    self.unlock =
                        Some(cx.new(|cx| UnlockScreen::new(self.state.clone(), window, cx)));
                }
            }
            LockState::Unlocked => {
                self.unlock = None;
                // The first-run screen stays until the recovery key step is
                // finished (UI 13.1); it removes itself with `Finished`.
                if self.first_run.is_none() && self.shell.is_none() {
                    self.shell = Some(cx.new(|cx| Shell::new(self.state.clone(), window, cx)));
                }
            }
        }
        cx.notify();
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

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let root = v_flex()
            .id("root")
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            // Any input counts as activity for auto-lock (UI 13.3).
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                this.state.read(cx).activity();
            }))
            .capture_key_down(cx.listener(|this, _, _, cx| {
                this.state.read(cx).activity();
            }));

        if let Some(message) = self.state.read(cx).open_error() {
            let message = message.to_owned();
            return root
                .child(self.render_startup_error(message, cx))
                .into_any_element();
        }
        let content = if let Some(first_run) = &self.first_run {
            first_run.clone().into_any_element()
        } else if let Some(shell) = &self.shell {
            shell.clone().into_any_element()
        } else if let Some(unlock) = &self.unlock {
            unlock.clone().into_any_element()
        } else {
            div().into_any_element()
        };
        root.child(content).into_any_element()
    }
}
