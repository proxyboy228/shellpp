//! MINIMAL unlock screen (UI 13.2): the app name, "Master password" with the
//! focus in it, Unlock (also Enter). A wrong password shows an error under
//! the field; the field is cleared and keeps the focus.

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use secrecy::SecretString;
use shellpp_core::CoreError;

use crate::i18n;
use crate::state::AppState;
use crate::theme::metrics;

pub struct UnlockScreen {
    state: Entity<AppState>,
    password: Entity<InputState>,
    busy: bool,
    error: Option<SharedString>,
    work: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for UnlockScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockScreen").finish_non_exhaustive()
    }
}

impl UnlockScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(i18n::lock::MASTER_PASSWORD)
        });
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
        Self {
            state,
            password,
            busy: false,
            error: None,
            work: None,
            _subscriptions: vec![subscription],
        }
    }

    /// The error under the field, if any.
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let value = self.password.read(cx).value();
        if value.is_empty() {
            return;
        }
        let password = SecretString::from(value.to_string());
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
                    Ok(()) => {}
                    Err(CoreError::InvalidPassword) => {
                        this.error = Some(i18n::lock::WRONG_PASSWORD.into());
                        this.password
                            .update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "unlock failed");
                        this.error = Some(i18n::lock::FAILED.into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for UnlockScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let content = v_flex()
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(i18n::APP_NAME),
            )
            .child(
                Input::new(&self.password)
                    .id("unlock-password")
                    .mask_toggle(),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .id("unlock-error")
                        .test_support()
                        .text_color(danger)
                        .child(error),
                )
            })
            .child(
                Button::new("unlock-submit")
                    .primary()
                    .label(if self.busy {
                        i18n::lock::WORKING
                    } else {
                        i18n::lock::UNLOCK
                    })
                    .disabled(self.busy)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)),
                    ),
            )
            // TODO(lock): recovery flow of UI 13.2.
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(muted)
                    .child(i18n::lock::USE_RECOVERY_KEY),
            );
        super::frame("unlock", content, cx)
    }
}
