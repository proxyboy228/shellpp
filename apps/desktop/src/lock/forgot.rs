//! "Forgot password?" on the unlock screen (UI 13.2, decision D29): a modal
//! card over the unlock screen that explains that the master password cannot
//! be recovered and offers
//!
//! - "Restore from sync": shown, inactive until a sync server exists;
//! - "Start with a new vault": an explicit confirmation in a second step
//!   that says the data file is renamed and kept, not deleted; "Cancel" is
//!   the default (Enter), the danger button "Start with a new vault"
//!   confirms. The core renames the data file to
//!   `shellpp.db.<UTC timestamp>.old` and the app returns to the first run.
//!
//! With "Remember on this device" on, the card also says that a restart
//! opens the vault without the password (the device key is used at launch
//! only), so nobody archives a vault they can still open.
//!
//! The card is the app's own overlay (scrim, `--bg-overlay`, radius 12,
//! `--shadow-modal`), not a kit dialog: it lives inside the lock screen,
//! which already covers the window. Esc and Cancel close it; Enter is Cancel.

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use super::{CARD_WIDTH, NoticeKind, error_line, hint_line, notice, strings};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::controls::{ButtonColors as _, Filled};
use crate::theme::{self, metrics};

/// The step on display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgotStep {
    /// The explanation and the two ways out.
    Explain,
    /// "Start with a new vault?".
    Confirm,
}

/// Emitted when the card closes itself (Cancel, Esc).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForgotEvent {
    Close,
}

pub struct ForgotDialog {
    state: Entity<AppState>,
    step: ForgotStep,
    busy: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
    work: Option<Task<()>>,
}

impl std::fmt::Debug for ForgotDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgotDialog")
            .field("step", &self.step)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<ForgotEvent> for ForgotDialog {}

impl Focusable for ForgotDialog {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ForgotDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            state,
            step: ForgotStep::Explain,
            busy: false,
            error: None,
            focus,
            work: None,
        }
    }

    pub fn step(&self) -> ForgotStep {
        self.step
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// "Start with a new vault": the confirmation step.
    pub fn ask_to_confirm(&mut self, cx: &mut Context<Self>) {
        self.step = ForgotStep::Confirm;
        self.error = None;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            cx.emit(ForgotEvent::Close);
        }
    }

    /// The confirmation's "Start with a new vault". The root view shows the
    /// first run when the core reports it.
    pub fn start_over(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.step != ForgotStep::Confirm {
            return;
        }
        self.busy = true;
        self.error = None;
        let reset = self.state.update(cx, |state, cx| state.start_new_vault(cx));
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = reset.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(error) = result {
                    tracing::warn!(code = error.code(), "new vault not started");
                    this.error = Some(strings::FAILED.into());
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn button(id: &'static str, label: &'static str) -> Button {
        Button::new(id)
            .large()
            .h(metrics::HEIGHT_CONTROL)
            .rounded(metrics::RADIUS_CONTROL)
            .label(label)
    }

    fn render_explain(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let overlay_hover = theme.accent;
        let danger = theme.danger;
        let danger_hover = tokens.danger_hover.hsla();
        let remembered = self.state.read(cx).remember_enabled();
        // Inactive until sync exists: the whole control at 0.45, no hover.
        let sync = option_button("forgot-restore-sync", strings::RESTORE_FROM_SYNC, cx)
            .text_color(theme.popover_foreground)
            .opacity(metrics::OPACITY_DISABLED);
        // Leads to the confirmation; a dangerous action is shown in
        // `--danger` (hovered: `--danger-hover` on `--bg-overlay-hover`).
        let new_vault = option_button("forgot-new-vault", strings::START_NEW_VAULT, cx)
            .text_color(danger)
            .cursor_pointer()
            .hover(move |style| style.bg(overlay_hover).text_color(danger_hover))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.ask_to_confirm(cx)));
        v_flex()
            .gap(metrics::SPACE_4)
            .child(title(strings::FORGOT_TITLE, cx))
            .child(body(strings::FORGOT_BODY, cx))
            .when(remembered, |this| {
                this.child(notice(
                    "forgot-remembered",
                    NoticeKind::Info,
                    strings::FORGOT_REMEMBERED,
                    cx,
                ))
            })
            .child(v_flex().gap(metrics::SPACE_1).child(sync).child(hint_line(
                "forgot-restore-sync-hint",
                strings::RESTORE_FROM_SYNC_HINT,
                cx,
            )))
            .child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(new_vault)
                    .child(hint_line(
                        "forgot-new-vault-hint",
                        strings::START_NEW_VAULT_HINT,
                        cx,
                    )),
            )
            .child(
                h_flex().justify_end().child(
                    Self::button("forgot-cancel", strings::CANCEL)
                        .neutral_colors()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
                ),
            )
    }

    fn render_confirm(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = !self.busy;
        v_flex()
            .gap(metrics::SPACE_4)
            .child(title(strings::CONFIRM_TITLE, cx))
            .child(body(strings::CONFIRM_BODY, cx))
            .children(
                self.error
                    .clone()
                    .map(|error| error_line("forgot-error", error, cx)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        Self::button("forgot-cancel", strings::CANCEL)
                            .neutral_colors()
                            .disabled_as_designed(self.busy, Filled::Neutral, cx)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
                    )
                    .child(
                        Self::button(
                            "forgot-start-over",
                            if self.busy {
                                strings::WORKING
                            } else {
                                strings::CONFIRM_START
                            },
                        )
                        .danger()
                        .disabled_as_designed(!enabled, Filled::Danger, cx)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start_over(cx))),
                    ),
            )
    }
}

/// A full-width option of the card: 36 px, 1 px `--border-strong`, radius 6.
fn option_button(id: &'static str, label: &'static str, cx: &gpui::App) -> widgets::Hit {
    widgets::hit(id)
        .w_full()
        .h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_3)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(cx.theme().input)
        .text_size(metrics::FONT_SIZE_BODY)
        .child(label)
}

/// A modal title: 20 px, 600.
fn title(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(cx.theme().popover_foreground)
        .child(text)
}

/// Body text: 14 px, `--text-secondary`.
fn body(text: &'static str, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl Render for ForgotDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let scrim = theme::tokens(cx).scrim.hsla();
        let surface = theme.popover;
        let border = theme.border;
        let content = match self.step {
            ForgotStep::Explain => self.render_explain(cx).into_any_element(),
            ForgotStep::Confirm => self.render_confirm(cx).into_any_element(),
        };
        div()
            .id("forgot-scrim")
            .test_support()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p(metrics::SPACE_4)
            .track_focus(&self.focus)
            // Esc closes; Enter is Cancel, the default of both steps (UI 13.2).
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "escape" | "enter") {
                    cx.stop_propagation();
                    this.close(cx);
                }
            }))
            .child(
                v_flex()
                    .id("forgot-dialog")
                    .test_support()
                    .w(CARD_WIDTH)
                    .max_h_full()
                    .overflow_y_scroll()
                    .p(metrics::SPACE_5)
                    .rounded(metrics::RADIUS_OVERLAY)
                    .bg(surface)
                    .border_1()
                    .border_color(border)
                    .shadow(theme::shadow_modal(cx))
                    .child(content),
            )
    }
}
