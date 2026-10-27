//! The prompt dialog of a session tab (UI 4.8): username, password, key
//! passphrase, or one keyboard-interactive question at a time with the
//! server's name and instruction above it.
//!
//! Server text (docs/security.md, 11.2): a keyboard-interactive dialog keeps
//! the app's own title and the host line; the server's name, instruction
//! and question appear only inside a bordered block captioned "Message from
//! <host>", in the monospace font, so a server cannot make the dialog look
//! like one of the app's own (for example the lock screen).
//!
//! - The value is masked for passwords, passphrases and questions the server
//!   marks as not echoed; the mask can be toggled.
//! - The password form of a saved host has "Save" (off by default); a quick
//!   connect has none. The passphrase form has "Remember until Shell++
//!   exits" (D30: kept in memory only).
//! - A refused value shows the error in the same form.
//! - "Continue" (Enter) sends the value, "Cancel" (Esc) stops the attempt.
//!
//! The field's text goes straight into a `SecretString` and the field is
//! cleared at once (as on the lock screens); nothing is logged.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, SharedString,
    Subscription, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, v_flex};

use super::dialog::{self, Confirm, Dismiss, Kind};
use super::model::{Keep, Prompt, PromptReply};
use super::strings;
use crate::panels::widgets;
use crate::theme::metrics;

/// Where the form sends the answer.
pub type ReplyHandler = Rc<dyn Fn(PromptReply, &mut Window, &mut App)>;

/// The prompt dialog.
pub struct PromptForm {
    prompt: Prompt,
    /// `user@host` (password title) or the host title (others).
    target: SharedString,
    input: Entity<InputState>,
    keep: bool,
    focus: FocusHandle,
    on_reply: ReplyHandler,
    _subscription: Subscription,
}

impl std::fmt::Debug for PromptForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptForm")
            .field("prompt", &self.prompt)
            .finish_non_exhaustive()
    }
}

impl Focusable for PromptForm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

/// Whom a password is for: `username@target`, unless the target already
/// names that user (a quick connect's `alice@host:22`, run 17).
pub(super) fn password_target(username: &str, target: &str) -> String {
    if username.is_empty() || target.starts_with(&format!("{username}@")) {
        target.to_owned()
    } else {
        format!("{username}@{target}")
    }
}

impl PromptForm {
    pub fn new(
        prompt: Prompt,
        target: SharedString,
        on_reply: ReplyHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let masked = prompt.is_masked();
        let input = cx.new(|cx| InputState::new(window, cx).masked(masked));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.submit(window, cx);
                }
            },
        );
        Self {
            prompt,
            target,
            input,
            keep: false,
            focus: cx.focus_handle(),
            on_reply,
            _subscription: subscription,
        }
    }

    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    /// The "Save" / "Remember" checkbox.
    pub fn keep(&self) -> bool {
        self.keep
    }

    pub fn set_keep(&mut self, keep: bool, cx: &mut Context<Self>) {
        self.keep = keep;
        cx.notify();
    }

    /// Whether the form offers the checkbox.
    pub fn offers_keep(&self) -> bool {
        self.prompt.keep() != Keep::None
    }

    /// Moves the keyboard focus into the field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn can_submit(&self, cx: &App) -> bool {
        // Keyboard-interactive answers and passwords may be empty on
        // purpose; a username may not.
        match self.prompt {
            Prompt::Username => !self.input.read(cx).value().trim().is_empty(),
            _ => true,
        }
    }

    /// "Continue" (Enter): the value goes out, the field is cleared.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return;
        }
        let value = crate::lock::secret_text(self.input.read(cx));
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let keep = self.keep && self.offers_keep();
        (self.on_reply)(PromptReply::Answer { value, keep }, window, cx);
    }

    /// "Cancel" (Esc).
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        (self.on_reply)(PromptReply::Cancel, window, cx);
    }

    /// The dialog title: always the app's own text.
    pub(super) fn title(&self) -> SharedString {
        match &self.prompt {
            Prompt::Username => strings::USERNAME_TITLE.into(),
            Prompt::Password { .. } => strings::PASSWORD_TITLE.into(),
            Prompt::Passphrase { .. } => strings::PASSPHRASE_TITLE.into(),
            Prompt::KeyboardInteractive { .. } => strings::KI_TITLE.into(),
        }
    }

    /// The line under the title: always the app's own text (the host for
    /// keyboard-interactive questions).
    pub(super) fn subtitle(&self) -> Option<SharedString> {
        match &self.prompt {
            Prompt::Username => Some(self.target.clone()),
            Prompt::Password { username, .. } => {
                Some(strings::password_for(&password_target(username, &self.target)).into())
            }
            Prompt::Passphrase { key_label, .. } => Some(strings::passphrase_for(key_label).into()),
            Prompt::KeyboardInteractive { .. } => Some(self.target.clone()),
        }
    }

    /// The field's label: always the app's own text.
    pub(super) fn label(&self) -> SharedString {
        match &self.prompt {
            Prompt::Username => strings::USERNAME_LABEL.into(),
            Prompt::Password { .. } => strings::PASSWORD_LABEL.into(),
            Prompt::Passphrase { .. } => strings::PASSPHRASE_LABEL.into(),
            Prompt::KeyboardInteractive { .. } => strings::KI_ANSWER.into(),
        }
    }

    /// The server's own lines (name, instruction, question), non-empty
    /// ones only; empty for the app's other prompts.
    pub(super) fn server_lines(&self) -> Vec<SharedString> {
        match &self.prompt {
            Prompt::KeyboardInteractive {
                name,
                instruction,
                prompt,
                ..
            } => [name, instruction, prompt]
                .into_iter()
                .map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .map(|text| SharedString::from(text.to_owned()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The server text block: a caption naming the host, then the
    /// server's lines as plain monospace text on the field surface.
    fn server_block(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let lines = self.server_lines();
        if lines.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let tokens = crate::theme::tokens(cx);
        Some(
            v_flex()
                .id("session-prompt-server")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_1)
                .p(metrics::SPACE_3)
                .rounded(metrics::RADIUS_CONTROL)
                .bg(tokens.bg_input.hsla())
                .border_1()
                .border_color(theme.input)
                .child(widgets::caption(strings::message_from(&self.target), cx))
                .children(lines.into_iter().map(|line| {
                    div()
                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                        .text_size(metrics::FONT_SIZE_DENSE)
                        .text_color(theme.popover_foreground)
                        .whitespace_normal()
                        .child(line)
                })),
        )
    }

    fn error(&self) -> Option<&'static str> {
        if !self.prompt.has_error() {
            return None;
        }
        Some(match self.prompt {
            Prompt::Password { .. } => strings::PASSWORD_ERROR,
            Prompt::Passphrase { .. } => strings::PASSPHRASE_ERROR,
            _ => strings::KI_ERROR,
        })
    }
}

impl Render for PromptForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let error = self.error();
        let masked = self.prompt.is_masked();
        let input = Input::new(&self.input).when(masked, |input| input.mask_toggle());
        let field = v_flex()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(widgets::caption(self.label(), cx))
            .child(widgets::text_input_with(
                input,
                &self.input,
                "session-prompt-input",
                error.is_some(),
                window,
                cx,
            ))
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .id("session-prompt-error")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.danger)
                        .child(error),
                )
            });
        let progress = match self.prompt {
            Prompt::KeyboardInteractive { index, count, .. } if count > 1 => {
                Some(strings::question_of(index, count))
            }
            _ => None,
        };
        let keep = self.offers_keep().then(|| {
            let (label, hint) = match (&self.prompt, self.prompt.keep()) {
                (_, Keep::ThisSession) => (
                    strings::PASSPHRASE_REMEMBER,
                    Some(strings::PASSPHRASE_REMEMBER_HINT),
                ),
                (Prompt::Passphrase { .. }, _) => (strings::PASSPHRASE_SAVE, None),
                _ => (strings::PASSWORD_SAVE, None),
            };
            v_flex()
                .gap(metrics::SPACE_1)
                .child(
                    Checkbox::new("session-prompt-keep")
                        .label(label)
                        .checked(self.keep)
                        .on_click(
                            cx.listener(|this, checked: &bool, _, cx| this.set_keep(*checked, cx)),
                        ),
                )
                .when_some(hint, |this, hint| this.child(widgets::caption(hint, cx)))
        });
        let enabled = self.can_submit(cx);
        let buttons = dialog::buttons([
            dialog::button(
                "session-prompt-cancel",
                strings::CANCEL,
                Kind::Neutral,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cancel(window, cx)))
            .into_any_element(),
            dialog::button(
                "session-prompt-continue",
                strings::CONTINUE,
                Kind::Accent,
                enabled,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)))
            .into_any_element(),
        ]);
        let content = v_flex()
            .gap(metrics::SPACE_4)
            .child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(dialog::title(self.title(), cx))
                    .children(self.subtitle().map(|text| {
                        dialog::body(text, cx)
                            .id("session-prompt-subtitle")
                            .test_support()
                    }))
                    .children(progress.map(|text| {
                        div()
                            .id("session-prompt-progress")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(theme.muted_foreground)
                            .child(text)
                    })),
            )
            .children(self.server_block(cx))
            .child(field)
            .children(keep)
            .child(buttons);
        dialog::frame("session-prompt", &self.focus, content, cx)
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.submit(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.cancel(window, cx)))
    }
}
