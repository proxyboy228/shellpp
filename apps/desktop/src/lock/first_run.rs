//! The first-run screen (UI 13.1, decision D29): the app name and one line
//! about local encrypted storage; "Master password" and "Repeat password"
//! with a strength indicator (at least 8 characters, no composition rules);
//! the warning that a forgotten password cannot be recovered; "Remember on
//! this device" (on by default, shown only with an OS keychain); the accent
//! button "Create vault", enabled when the password has at least 8
//! characters and both fields match. The core then unlocks and the root
//! view shows the Hosts screen in its empty state.
//!
//! Messages: "At least 8 characters" under a too short password once the
//! user moves on (the field loses focus, the second field is used) or tries
//! to create the vault; "Passwords do not match" under the second field once
//! it loses focus, as soon as it is as long as the password and differs, or
//! on an attempt to create the vault (also while it is empty). Enter in
//! either field does the same as "Create vault".
//!
//! The fields are read in place (lengths, comparison); the typed text is
//! copied only into the `SecretString` for the core and, for the strength
//! estimate, into a buffer that is wiped when dropped (see [`super`]).

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Sizable as _, h_flex, v_flex,
};
use shellpp_core::{CoreError, ValidationReason};

use super::strength::Strength;
use super::{NoticeKind, error_line, heading_with, hint_line, notice, password_field, strings};
use crate::state::AppState;
use crate::state::security::ArchivedDataFile;
use crate::theme::controls::{ButtonColors as _, Filled};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// The minimum the screen checks before asking the core (which checks the
/// same rule after NFC normalization).
const MIN_CHARS: usize = shellpp_core::crypto::MIN_PASSWORD_CHARS;
/// Width of the strength label, so the bar does not move with the text.
const STRENGTH_LABEL_WIDTH: gpui::Rems = metrics::rpx(96.);

/// What the fields hold, without their text.
#[derive(Clone, Copy, Debug)]
struct Fields {
    password_chars: usize,
    repeat_chars: usize,
    matches: bool,
}

pub struct FirstRunScreen {
    state: Entity<AppState>,
    password: Entity<InputState>,
    repeat: Entity<InputState>,
    /// Focus of the screen itself: a click outside the fields takes it, so
    /// the field loses focus and its message appears.
    focus: FocusHandle,
    /// "Remember on this device" (on by default, D29).
    remember: bool,
    /// Updated on every change of the first field.
    strength: Option<Strength>,
    /// The field lost focus once (or creation was attempted): its message
    /// may show.
    password_checked: bool,
    repeat_checked: bool,
    /// "Create vault" (or Enter) was refused for invalid fields and nothing
    /// was typed since: every applicable message shows, also under an empty
    /// field (UI 13.1).
    submit_attempted: bool,
    /// The core refused the password as too short (after NFC).
    core_too_short: bool,
    busy: bool,
    error: Option<SharedString>,
    work: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for FirstRunScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the field contents.
        f.debug_struct("FirstRunScreen")
            .field("remember", &self.remember)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl FirstRunScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let repeat = cx.new(|cx| InputState::new(window, cx).masked(true));
        let subscriptions = vec![
            cx.subscribe_in(
                &password,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.fields_changed(true, cx),
                    InputEvent::Blur => {
                        this.password_checked = true;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Focus => {}
                },
            ),
            cx.subscribe_in(
                &repeat,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.fields_changed(false, cx),
                    InputEvent::Blur => {
                        this.repeat_checked = true;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    // Moving on to the second field checks the first one.
                    InputEvent::Focus => {
                        this.password_checked = true;
                        cx.notify();
                    }
                },
            ),
        ];
        password.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            password,
            repeat,
            focus: cx.focus_handle(),
            remember: true,
            strength: None,
            password_checked: false,
            repeat_checked: false,
            submit_attempted: false,
            core_too_short: false,
            busy: false,
            error: None,
            work: None,
            _subscriptions: subscriptions,
        }
    }

    /// Lengths of both fields and whether they match, read in place from
    /// the fields' text (no copy of it is made).
    fn fields(&self, cx: &gpui::App) -> Fields {
        let password = self.password.read(cx).text();
        let repeat = self.repeat.read(cx).text();
        Fields {
            password_chars: password.len_chars(),
            repeat_chars: repeat.len_chars(),
            matches: password == repeat,
        }
    }

    /// After an edit of either field.
    fn fields_changed(&mut self, password_edited: bool, cx: &mut Context<Self>) {
        self.submit_attempted = false;
        if password_edited {
            self.strength = super::strength_of(self.password.read(cx));
            self.core_too_short = false;
        } else {
            // Typing the second password means the first one is done.
            self.password_checked = true;
        }
        if self.fields(cx).repeat_chars == 0 {
            self.repeat_checked = false;
        }
        self.error = None;
        cx.notify();
    }

    /// "Remember on this device" as it will be applied (off without an OS
    /// keychain).
    pub fn remember(&self, cx: &gpui::App) -> bool {
        self.remember && self.state.read(cx).remember_available()
    }

    pub fn set_remember(&mut self, remember: bool, cx: &mut Context<Self>) {
        self.remember = remember;
        cx.notify();
    }

    /// The strength of the typed password (`None` while empty).
    pub fn strength(&self) -> Option<Strength> {
        self.strength
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// The message under the first field, if any.
    pub fn password_error(&self, cx: &gpui::App) -> Option<&'static str> {
        let fields = self.fields(cx);
        let short = fields.password_chars < MIN_CHARS;
        let shown = (self.password_checked && short && fields.password_chars > 0)
            || (self.submit_attempted && short)
            || self.core_too_short;
        shown.then_some(strings::TOO_SHORT)
    }

    /// The message under the second field, if any.
    pub fn repeat_error(&self, cx: &gpui::App) -> Option<&'static str> {
        let fields = self.fields(cx);
        let differs = !fields.matches;
        let shown = (differs
            && fields.repeat_chars > 0
            && (self.repeat_checked || fields.repeat_chars >= fields.password_chars))
            || (differs && self.submit_attempted);
        shown.then_some(strings::PASSWORDS_DIFFER)
    }

    /// "Create vault" is enabled: at least 8 characters, both fields match.
    pub fn can_create(&self, cx: &gpui::App) -> bool {
        let fields = self.fields(cx);
        !self.busy && fields.password_chars >= MIN_CHARS && fields.matches
    }

    /// "Create vault" (also Enter). With invalid fields it only shows why.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !self.can_create(cx) {
            self.password_checked = true;
            self.repeat_checked = true;
            self.submit_attempted = true;
            cx.notify();
            return;
        }
        self.submit_attempted = false;
        let password = super::secret_text(self.password.read(cx));
        for input in [&self.password, &self.repeat] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.busy = true;
        self.error = None;
        let remember = self.remember(cx);
        let create = self
            .state
            .update(cx, |state, cx| state.create_vault(password, remember, cx));
        self.work = Some(cx.spawn_in(window, async move |this, cx| {
            let result = create.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    // The root view replaces this screen with the shell.
                    Ok(_) => {}
                    Err(CoreError::Validation(fields))
                        if fields
                            .iter()
                            .any(|field| field.reason == ValidationReason::TooShort) =>
                    {
                        this.core_too_short = true;
                        this.password
                            .update(cx, |input, cx| input.focus(window, cx));
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "first run failed");
                        this.error = Some(strings::FAILED.into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_strength(&self, cx: &gpui::App) -> impl IntoElement {
        let theme = cx.theme();
        let track = theme::tokens(cx).border_subtle.hsla();
        let (filled, color) = match self.strength {
            None => (0, track),
            Some(level) => (
                level.segments(),
                match level {
                    Strength::Weak => theme.danger,
                    Strength::Fair => theme.warning,
                    Strength::Good | Strength::Strong => theme.success,
                },
            ),
        };
        h_flex()
            .id("first-run-strength")
            .test_support()
            .w_full()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(
                h_flex()
                    .flex_1()
                    .gap(metrics::SPACE_1)
                    .children((0..4).map(|index| {
                        div()
                            .flex_1()
                            .h(metrics::SPACE_1)
                            .rounded_full()
                            .bg(if index < filled { color } else { track })
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .w(STRENGTH_LABEL_WIDTH)
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(
                        self.strength
                            .map(|level| strings::strength_label(level.label()))
                            .unwrap_or_default(),
                    ),
            )
    }

    fn render_remember(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("first-run-remember-block")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(
                Switch::new("first-run-remember")
                    .checked(self.remember)
                    .label(strings::REMEMBER_DEVICE)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.set_remember(*checked, cx);
                    })),
            )
            .child(hint_line(
                "first-run-remember-hint",
                strings::REMEMBER_DEVICE_HINT,
                cx,
            ))
    }
}

impl Render for FirstRunScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = cx.theme().secondary;
        let archived = ArchivedDataFile::get(cx);
        let available = self.state.read(cx).remember_available();
        let enabled = self.can_create(cx);
        let password_error = self.password_error(cx);
        let repeat_error = self.repeat_error(cx);
        let password_below = match password_error {
            Some(error) => error_line("first-run-password-error", error, cx),
            None => hint_line("first-run-min-length", strings::TOO_SHORT, cx),
        };
        let create = Button::new("first-run-create")
            .accent_colors()
            .large()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
            .label(if self.busy {
                strings::WORKING
            } else {
                strings::CREATE_VAULT
            })
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .disabled_as_designed(!enabled, Filled::Accent, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit(window, cx)));
        let restore = crate::hosts::controls::ControlButton::new(
            "first-run-restore-sync",
            crate::hosts::controls::ControlKind::Ghost,
        )
        .label(crate::sync_ui::strings::RESTORE_FROM_SYNC)
        .on_click({
            let state = self.state.clone();
            move |_, window, cx| {
                crate::sync_ui::open_connect(state.clone(), true, window, cx);
            }
        });
        let content = v_flex()
            .id("first-run-form")
            .w_full()
            // 12 px between the blocks: the card fits a 960 x 600 window.
            .gap(metrics::SPACE_3)
            // A click outside the fields takes the focus from them, so their
            // messages appear (UI 13.1).
            .track_focus(&self.focus)
            // Stage 7 (UI 13.2): a new device can take its data from the
            // account instead of creating a vault; on the app name's line,
            // so the card keeps its height and the tagline its one line
            // (960 x 600, run 12).
            .child(heading_with(strings::FIRST_RUN_TAGLINE, restore, cx))
            .when_some(archived, |this, name| {
                this.child(notice(
                    "first-run-archived",
                    NoticeKind::Info,
                    strings::archived_note(&name),
                    cx,
                ))
            })
            .child(
                password_field(
                    strings::MASTER_PASSWORD,
                    &self.password,
                    "first-run-password",
                    password_error.is_some(),
                    self.busy,
                    window,
                    cx,
                )
                .child(self.render_strength(cx))
                .child(password_below),
            )
            .child(
                password_field(
                    strings::REPEAT_PASSWORD,
                    &self.repeat,
                    "first-run-repeat",
                    repeat_error.is_some(),
                    self.busy,
                    window,
                    cx,
                )
                .children(
                    repeat_error.map(|error| error_line("first-run-repeat-error", error, cx)),
                ),
            )
            // Always shown, not dismissible (UI 13.1).
            .child(notice(
                "first-run-warning",
                NoticeKind::Warning,
                strings::FIRST_RUN_WARNING,
                cx,
            ))
            .when(available, |this| this.child(self.render_remember(cx)))
            .children(
                self.error
                    .clone()
                    .map(|error| error_line("first-run-error", error, cx)),
            )
            .child(create);
        super::frame("first-run", content, None, cx)
    }
}
