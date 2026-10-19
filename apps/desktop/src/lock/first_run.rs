//! MINIMAL first-run screen (UI 13.1): create the master password, then show
//! the recovery key until "I saved the recovery key" is checked.

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, FontWeight, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use secrecy::{ExposeSecret as _, SecretString};
use shellpp_core::{CoreError, ValidationReason};

use crate::i18n;
use crate::state::AppState;
use crate::theme::metrics;

/// Emitted when the user finished the recovery key step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstRunEvent {
    Finished,
}

enum Step {
    Password,
    RecoveryKey { key: SecretString, saved: bool },
}

pub struct FirstRunScreen {
    state: Entity<AppState>,
    password: Entity<InputState>,
    confirm: Entity<InputState>,
    step: Step,
    busy: bool,
    error: Option<SharedString>,
    work: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for FirstRunScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirstRunScreen").finish_non_exhaustive()
    }
}

impl EventEmitter<FirstRunEvent> for FirstRunScreen {}

impl FirstRunScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(i18n::lock::PASSWORD)
        });
        let confirm = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(i18n::lock::CONFIRM_PASSWORD)
        });
        let mut subscriptions = Vec::new();
        for input in [&password, &confirm] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    _ => {}
                },
            ));
        }
        password.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            password,
            confirm,
            step: Step::Password,
            busy: false,
            error: None,
            work: None,
            _subscriptions: subscriptions,
        }
    }

    /// Whether the screen shows the recovery key step.
    pub fn is_recovery_step(&self) -> bool {
        matches!(self.step, Step::RecoveryKey { .. })
    }

    fn can_continue(&self, cx: &gpui::App) -> bool {
        let password = self.password.read(cx).value();
        let confirm = self.confirm.read(cx).value();
        !self.busy && !password.is_empty() && password == confirm
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.step, Step::Password) {
            return;
        }
        if !self.can_continue(cx) {
            let differ = self.password.read(cx).value() != self.confirm.read(cx).value();
            if differ {
                self.error = Some(i18n::lock::PASSWORDS_DIFFER.into());
                cx.notify();
            }
            return;
        }
        let password = SecretString::from(self.password.read(cx).value().to_string());
        for input in [&self.password, &self.confirm] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.busy = true;
        self.error = None;
        let setup = self
            .state
            .update(cx, |state, cx| state.first_run_setup(password, cx));
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = setup.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(result) => {
                        this.step = Step::RecoveryKey {
                            key: result.recovery_key,
                            saved: false,
                        };
                    }
                    Err(CoreError::Validation(fields))
                        if fields
                            .iter()
                            .any(|field| field.reason == ValidationReason::TooShort) =>
                    {
                        this.error = Some("The master password is too short.".into());
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "first run failed");
                        this.error = Some(i18n::lock::FAILED.into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_password_step(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(i18n::lock::FIRST_RUN_TITLE),
            )
            // Always shown, not dismissible (UI 13.1).
            .child(
                div()
                    .id("first-run-warning")
                    .test_support()
                    .text_color(theme.warning)
                    .child(i18n::lock::FIRST_RUN_WARNING),
            )
            .child(
                Input::new(&self.password)
                    .id("first-run-password")
                    .mask_toggle(),
            )
            .child(
                Input::new(&self.confirm)
                    .id("first-run-confirm")
                    .mask_toggle(),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_color(theme.danger).child(error))
            })
            .child(
                Button::new("first-run-continue")
                    .primary()
                    .label(if self.busy {
                        i18n::lock::WORKING
                    } else {
                        i18n::lock::CONTINUE
                    })
                    .disabled(!self.can_continue(cx))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)),
                    ),
            )
    }

    fn render_recovery_step(
        &self,
        key: &str,
        saved: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(i18n::lock::RECOVERY_TITLE),
            )
            .child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(i18n::lock::RECOVERY_BODY),
            )
            .child(
                div()
                    .id("recovery-key")
                    .test_support()
                    .p(metrics::SPACE_3)
                    .rounded(metrics::RADIUS_CONTROL)
                    .bg(theme.background)
                    .font_family(theme.mono_font_family.clone())
                    .child(key.to_owned()),
            )
            .child(
                Button::new("recovery-copy")
                    .outline()
                    .label(i18n::lock::COPY)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        // The core copies it and clears the clipboard after 30 s.
                        if let Err(error) = this.state.read(cx).copy_recovery_key() {
                            tracing::warn!(code = error.code(), "recovery key not copied");
                        }
                    })),
            )
            .child(
                Checkbox::new("recovery-saved")
                    .label(i18n::lock::SAVED_CHECKBOX)
                    .checked(saved)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if let Step::RecoveryKey { saved, .. } = &mut this.step {
                            *saved = *checked;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("first-run-finish")
                    .primary()
                    .label(i18n::lock::FINISH)
                    .disabled(!saved)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Step::RecoveryKey { saved: true, .. } = this.step {
                            cx.emit(FirstRunEvent::Finished);
                        }
                    })),
            )
    }
}

impl Render for FirstRunScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.step {
            Step::Password => self.render_password_step(cx).into_any_element(),
            Step::RecoveryKey { key, saved } => {
                let key = key.expose_secret().to_owned();
                let saved = *saved;
                self.render_recovery_step(&key, saved, cx)
                    .into_any_element()
            }
        };
        super::frame("first-run", content, cx)
    }
}
