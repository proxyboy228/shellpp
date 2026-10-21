//! The Settings window (UI 12.1): a modal window over the app (kit dialog,
//! `--bg-overlay`, radius 12, `--shadow-modal`), opened with Ctrl+, or the
//! Settings item of the app menu, closed with X or Esc. Navigation on the
//! left, the section on the right; both scroll on their own. Changes apply
//! at once; there is no Save button.
//!
//! Built: Terminal (UI 12.4, stage 2a lead decision 12; [`terminal`]) and
//! Security (UI 13.4, decision D29): the auto-lock interval (Never,
//! 5, 15, 30 or 60 minutes; Never by default) and "Remember on this device"
//! (hidden without an OS keychain). Every other section shows the "In
//! development" placeholder (audit decision 1). Change master password and
//! the recovery key of 13.4 are not part of this stage.

mod data;
pub mod highlight_rules;
mod shortcuts;
pub mod strings;
pub mod terminal;

#[cfg(test)]
mod tests;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement as _,
    Pixels, Render, SharedString, Styled as _, Task, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};

use crate::lock::{NoticeKind, notice};
use crate::panels::widgets;
use crate::shell::placeholder;
use crate::state::AppState;
use crate::state::security::AUTO_LOCK_CHOICES;
use crate::theme::{self, metrics};

/// Width of the Settings window; the kit dialog narrows it in a small
/// window (the minimum window is 960 x 600).
pub const WINDOW_WIDTH: Pixels = px(720.);
/// Height of the navigation and section area.
pub const BODY_HEIGHT: Pixels = px(420.);
/// Width of the navigation column.
pub const NAV_WIDTH: Pixels = px(200.);

/// Sections of UI 12.1, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    AccountSync,
    Security,
    SshId,
    Team,
    Terminal,
    Sftp,
    Logs,
    Shortcuts,
    Updates,
}

impl Section {
    pub const ALL: [Section; 9] = [
        Section::AccountSync,
        Section::Security,
        Section::SshId,
        Section::Team,
        Section::Terminal,
        Section::Sftp,
        Section::Logs,
        Section::Shortcuts,
        Section::Updates,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::AccountSync => strings::ACCOUNT_SYNC,
            Section::Security => strings::SECURITY,
            Section::SshId => strings::SSH_ID,
            Section::Team => strings::TEAM,
            Section::Terminal => strings::TERMINAL,
            Section::Sftp => strings::SFTP,
            Section::Logs => strings::LOGS,
            Section::Shortcuts => strings::SHORTCUTS,
            Section::Updates => strings::UPDATES,
        }
    }

    /// Element id of its navigation item.
    pub fn nav_id(self) -> &'static str {
        match self {
            Section::AccountSync => "settings-nav-account",
            Section::Security => "settings-nav-security",
            Section::SshId => "settings-nav-ssh-id",
            Section::Team => "settings-nav-team",
            Section::Terminal => "settings-nav-terminal",
            Section::Sftp => "settings-nav-sftp",
            Section::Logs => "settings-nav-logs",
            Section::Shortcuts => "settings-nav-shortcuts",
            Section::Updates => "settings-nav-updates",
        }
    }

    /// The sections with their content (stage 6 adds Account & Sync, SFTP
    /// and Logs, D67).
    pub fn is_built(self) -> bool {
        matches!(
            self,
            Section::AccountSync
                | Section::Security
                | Section::Terminal
                | Section::Sftp
                | Section::Logs
                | Section::Shortcuts
        )
    }

    /// SSH ID and Team stay hidden until accounts and teams (stages 7-8,
    /// plan of run 3, section 9).
    pub fn is_shown(self) -> bool {
        !matches!(self, Section::SshId | Section::Team)
    }
}

/// Opens the Settings window on the Security section. Only while unlocked:
/// settings are written by the unlocked core.
pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    if !state.read(cx).is_unlocked() {
        return;
    }
    let view = cx.new(|cx| SettingsView::new(state, window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme();
        dialog
            .w(WINDOW_WIDTH)
            .title(
                div()
                    .id("settings-title")
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(strings::TITLE),
            )
            .close_button(true)
            .keyboard(true)
            .bg(theme.popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .shadow(theme::shadow_modal(cx))
            .child(view.clone())
    });
}

/// The content of the Settings window.
pub struct SettingsView {
    state: Entity<AppState>,
    section: Section,
    auto_lock_open: bool,
    /// A keychain call of "Remember on this device" is running.
    remember_busy: bool,
    /// Turned on in this window: the explanation becomes a warning (13.4).
    remember_turned_on: bool,
    error: Option<SharedString>,
    work: Option<Task<()>>,
    /// The Terminal section (UI 12.4).
    terminal: Entity<terminal::TerminalSection>,
    /// SFTP: the program for Open.
    open_program: Entity<gpui_kit::component::input::InputState>,
    /// Logs: how many entries "Clear the log" removed.
    cleared: Option<usize>,
    /// Shortcuts: the action whose keys are being recorded, and the focus
    /// that takes the keys.
    recording: Option<&'static str>,
    shortcuts_focus: gpui::FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl std::fmt::Debug for SettingsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsView")
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let terminal = cx.new(|cx| terminal::TerminalSection::new(state.clone(), window, cx));
        let program = state
            .read(cx)
            .core()
            .and_then(|core| core.settings_get().ok())
            .and_then(|values| {
                values
                    .get("sftp.open_program")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let open_program = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .default_value(program)
                .placeholder(strings::OPEN_PROGRAM_PLACEHOLDER)
        });
        let subscription = cx.subscribe(
            &open_program,
            |this: &mut Self, _, event: &gpui_kit::component::input::InputEvent, cx| {
                if matches!(event, gpui_kit::component::input::InputEvent::Change) {
                    this.open_program_changed(cx);
                }
            },
        );
        Self {
            state,
            section: Section::Security,
            auto_lock_open: false,
            remember_busy: false,
            remember_turned_on: false,
            error: None,
            work: None,
            terminal,
            open_program,
            cleared: None,
            recording: None,
            shortcuts_focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// The Terminal section (UI 12.4).
    pub fn terminal(&self) -> &Entity<terminal::TerminalSection> {
        &self.terminal
    }

    pub fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.recording = None;
        self.auto_lock_open = false;
        cx.notify();
    }

    /// Opens or closes the auto-lock list.
    pub fn toggle_auto_lock_list(&mut self, cx: &mut Context<Self>) {
        self.auto_lock_open = !self.auto_lock_open;
        cx.notify();
    }

    /// Sets the auto-lock interval (`None`: Never).
    pub fn set_auto_lock(&mut self, minutes: Option<u64>, cx: &mut Context<Self>) {
        self.auto_lock_open = false;
        let result = self
            .state
            .update(cx, |state, cx| state.set_auto_lock_minutes(minutes, cx));
        self.error = match result {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!(code = error.code(), "auto-lock not saved");
                Some(strings::FAILED.into())
            }
        };
        cx.notify();
    }

    /// Turns "Remember on this device" on or off (the keychain call runs in
    /// the background).
    pub fn set_remember(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.remember_busy {
            return;
        }
        self.remember_busy = true;
        self.error = None;
        let task = self
            .state
            .update(cx, |state, cx| state.set_remember_device(enabled, cx));
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.remember_busy = false;
                match result {
                    Ok(status) => {
                        this.remember_turned_on = enabled && status.remember_device_enabled;
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "remember on this device not changed");
                        this.error = Some(strings::FAILED.into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let active_bg = tokens.accent_subtle.hsla();
        let hover = theme.accent;
        let items: Vec<AnyElement> =
            Section::ALL
                .iter()
                .filter(|section| section.is_shown())
                .map(|&section| {
                    let active = section == self.section;
                    widgets::pressable(section.nav_id(), true, theme.background, cx)
                        .h(metrics::HEIGHT_CONTROL)
                        .px(metrics::SPACE_3)
                        .flex()
                        .items_center()
                        .rounded(metrics::RADIUS_CONTROL)
                        .text_size(metrics::FONT_SIZE_BODY)
                        .cursor_pointer()
                        .map(|this| {
                            if active {
                                this.bg(active_bg).text_color(theme.foreground)
                            } else {
                                this.text_color(theme.muted_foreground)
                                    .hover(move |style| style.bg(hover))
                            }
                        })
                        .child(section.label())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_section(section, cx)
                        }))
                        .into_any_element()
                })
                .collect();
        v_flex()
            .id("settings-nav")
            .test_support()
            .flex_none()
            .w(NAV_WIDTH)
            .h_full()
            .gap(metrics::SPACE_1)
            .pr(metrics::SPACE_3)
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .id("settings-nav-items")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(metrics::SPACE_1)
                    .children(items),
            )
            .child(
                div()
                    .px(metrics::SPACE_3)
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(strings::version()),
            )
    }

    fn render_security(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let state = self.state.read(cx);
        let minutes = state.auto_lock_minutes();
        let available = state.remember_available();
        let remembered = state.remember_enabled();
        let trigger = widgets::select_trigger(
            "settings-auto-lock",
            None,
            strings::auto_lock_label(minutes),
            self.auto_lock_open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_auto_lock_list(cx)));
        let list = self.auto_lock_open.then(|| {
            widgets::dropdown_list("settings-auto-lock-list", cx).children(
                AUTO_LOCK_CHOICES.iter().map(|&choice| {
                    let id = SharedString::from(format!(
                        "settings-auto-lock-{}",
                        choice.map_or("never".to_owned(), |minutes| minutes.to_string())
                    ));
                    widgets::dropdown_item(
                        id,
                        None,
                        strings::auto_lock_label(choice),
                        None,
                        choice == minutes,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.set_auto_lock(choice, cx),
                    ))
                }),
            )
        });
        let auto_lock = v_flex()
            .id("settings-auto-lock-row")
            .test_support()
            .gap(metrics::SPACE_2)
            .child(label(strings::AUTO_LOCK, cx))
            .child(widgets::caption(strings::AUTO_LOCK_HINT, cx))
            .child(div().w(px(240.)).child(trigger))
            .children(list.map(|list| div().w(px(240.)).child(list)));
        let remember = available.then(|| {
            let explanation = if self.remember_turned_on && remembered {
                notice(
                    "settings-remember-warning",
                    NoticeKind::Warning,
                    strings::REMEMBER_DEVICE_HINT,
                    cx,
                )
                .into_any_element()
            } else {
                widgets::caption(strings::REMEMBER_DEVICE_HINT, cx).into_any_element()
            };
            v_flex()
                .id("settings-remember-row")
                .test_support()
                .gap(metrics::SPACE_2)
                .child(
                    h_flex()
                        .justify_between()
                        .gap(metrics::SPACE_3)
                        .child(label(strings::REMEMBER_DEVICE, cx))
                        .child(
                            Switch::new("settings-remember")
                                .checked(remembered)
                                .disabled(self.remember_busy)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_remember(*checked, cx)
                                })),
                        ),
                )
                .child(explanation)
        });
        v_flex()
            .id("settings-security")
            .test_support()
            .gap(metrics::SPACE_4)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(theme.popover_foreground)
                    .child(strings::SECURITY),
            )
            .child(auto_lock)
            .when_some(remember, |this, remember| {
                this.child(widgets::divider(cx)).child(remember)
            })
            .children(self.error.clone().map(|error| {
                div()
                    .id("settings-error")
                    .test_support()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.danger)
                    .child(error)
            }))
    }
}

/// A setting's name: 14 px, 500, `--text-primary`.
fn label(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(cx.theme().popover_foreground)
        .child(text)
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = match self.section {
            Section::Terminal => self.terminal.clone().into_any_element(),
            Section::Security => self.render_security(cx).into_any_element(),
            Section::AccountSync => self.render_account(cx),
            Section::Sftp => self.render_sftp(window, cx),
            Section::Logs => self.render_logs(cx),
            Section::Shortcuts => self.render_shortcuts(cx),
            _ => placeholder::placeholder("settings-placeholder", self.section.label(), cx)
                .into_any_element(),
        };
        h_flex()
            .id("settings")
            .test_support()
            .w_full()
            .h(BODY_HEIGHT)
            .items_start()
            .gap(metrics::SPACE_4)
            .child(self.render_nav(cx))
            .child(
                div()
                    .id("settings-section")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .child(section),
            )
    }
}
