//! The unlock screen (UI 13.2, decision D29): the app name, "Master
//! password" with the focus in it, the accent button "Unlock" (Enter does the
//! same) and "Forgot password?". A wrong password shows a message under the
//! field and clears it. After 5 wrong passwords in a row the core pauses
//! unlocking (5 s, doubling, at most 5 minutes, kept across restarts); the
//! screen counts the pause down and keeps Unlock inactive meanwhile.

use std::time::Duration;

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, prelude::*,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, Sizable as _, h_flex, v_flex};
use shellpp_core::CoreError;
use shellpp_core::lock::lockout::whole_seconds;

use super::forgot::{ForgotDialog, ForgotEvent};
use super::{error_line, heading, password_field, strings};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::controls::{ButtonColors as _, Filled};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::metrics;

pub struct UnlockScreen {
    state: Entity<AppState>,
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    /// Whole seconds left in the pause after 5 wrong passwords (D29).
    retry_after: Option<u64>,
    /// The countdown task runs while this is set.
    counting: bool,
    forgot: Option<Entity<ForgotDialog>>,
    work: Option<Task<()>>,
    countdown: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    forgot_subscription: Option<Subscription>,
}

impl std::fmt::Debug for UnlockScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the field contents.
        f.debug_struct("UnlockScreen")
            .field("busy", &self.busy)
            .field("retry_after", &self.retry_after)
            .finish_non_exhaustive()
    }
}

impl UnlockScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscription = cx.subscribe_in(
            &password,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                InputEvent::Change if this.error.take().is_some() => cx.notify(),
                _ => {}
            },
        );
        password.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            state,
            password,
            busy: false,
            error: None,
            retry_after: None,
            counting: false,
            forgot: None,
            work: None,
            countdown: None,
            _subscriptions: vec![subscription],
            forgot_subscription: None,
        };
        // A pause from an earlier run is still counted down (D29).
        this.refresh_pause(window, cx);
        this
    }

    /// The message under the field, if any ("Wrong password.").
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Whole seconds left in the pause after 5 wrong passwords, if any.
    pub fn retry_after(&self) -> Option<u64> {
        self.retry_after
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// The "Forgot password?" card, while open.
    pub fn forgot(&self) -> Option<&Entity<ForgotDialog>> {
        self.forgot.as_ref()
    }

    fn can_submit(&self) -> bool {
        !self.busy && self.retry_after.is_none()
    }

    /// Re-reads the pause from the core and runs the countdown while it lasts.
    pub fn refresh_pause(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.retry_after = self.state.read(cx).unlock_retry_after().map(whole_seconds);
        cx.notify();
        if self.retry_after.is_none() || self.counting {
            return;
        }
        self.counting = true;
        self.countdown = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this.update_in(cx, |this, window, cx| {
                    this.retry_after = this.state.read(cx).unlock_retry_after().map(whole_seconds);
                    if this.retry_after.is_none() {
                        this.counting = false;
                        this.password
                            .update(cx, |input, cx| input.focus(window, cx));
                    }
                    cx.notify();
                    this.retry_after.is_some()
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// "Unlock" (also Enter).
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_submit() || self.forgot.is_some() {
            return;
        }
        if self.password.read(cx).text().len() == 0 {
            return;
        }
        let password = super::secret_text(self.password.read(cx));
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.busy = true;
        self.error = None;
        let unlock = self
            .state
            .update(cx, |state, cx| state.unlock(password, cx));
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let result = unlock.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    // The root view replaces this screen with the shell.
                    Ok(()) => {}
                    Err(CoreError::InvalidPassword) => {
                        this.error = Some(strings::WRONG_PASSWORD.into());
                        this.password
                            .update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(CoreError::TooManyAttempts { .. }) => {}
                    Err(error) => {
                        tracing::warn!(code = error.code(), "unlock failed");
                        this.error = Some(strings::FAILED.into());
                    }
                }
                this.refresh_pause(window, cx);
            });
        }));
        cx.notify();
    }

    /// "Forgot password?": opens the card over the screen.
    pub fn open_forgot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.forgot.is_some() || self.busy {
            return;
        }
        let dialog = cx.new(|cx| ForgotDialog::new(self.state.clone(), window, cx));
        self.forgot_subscription = Some(cx.subscribe_in(
            &dialog,
            window,
            |this: &mut Self, _, event: &ForgotEvent, window, cx| {
                let ForgotEvent::Close = event;
                this.close_forgot(window, cx);
            },
        ));
        self.forgot = Some(dialog);
        cx.notify();
    }

    fn close_forgot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forgot = None;
        self.forgot_subscription = None;
        self.password
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}

impl Render for UnlockScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = cx.theme().secondary;
        let enabled = self.can_submit();
        let below = match (self.retry_after, &self.error) {
            (Some(seconds), _) => Some(error_line("unlock-retry", strings::retry_in(seconds), cx)),
            (None, Some(error)) => Some(error_line("unlock-error", error.clone(), cx)),
            (None, None) => None,
        };
        let unlock = Button::new("unlock-submit")
            .accent_colors()
            .large()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
            .label(if self.busy {
                strings::WORKING
            } else {
                strings::UNLOCK
            })
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .disabled_as_designed(!enabled, Filled::Accent, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)));
        let forgot = widgets::text_button(
            "unlock-forgot",
            strings::FORGOT_PASSWORD,
            !self.busy,
            cx,
        )
        .when(!self.busy, |this| {
            this.on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.open_forgot(window, cx)),
            )
        });
        let content = v_flex()
            .w_full()
            .gap(metrics::SPACE_4)
            .child(heading(strings::UNLOCK_SUBTITLE, cx))
            .child(
                password_field(
                    strings::MASTER_PASSWORD,
                    &self.password,
                    "unlock-password",
                    below.is_some(),
                    // Read-only while checking and during a pause (UI 13.2).
                    !enabled,
                    window,
                    cx,
                )
                .children(below),
            )
            .child(unlock)
            .child(h_flex().w_full().justify_center().child(forgot));
        let overlay = self.forgot.clone().map(|dialog| dialog.into_any_element());
        super::frame("unlock", content, overlay, cx)
    }
}
