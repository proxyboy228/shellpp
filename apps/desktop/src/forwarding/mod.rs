//! The Port Forwarding screen (UI 8; stage 5, D66): the action bar (split
//! New forwarding with Local, Remote and Dynamic in its menu; search), the
//! grid of rule cards, and the rule panel on the right.
//!
//! - A card shows the type letter (L, R, D), the label (the summary when
//!   there is none), the summary line and the status: stopped, starting,
//!   running, reconnecting, or the error in words (a busy port names the
//!   port, UI 8.4).
//! - A click selects the card and opens the rule in the panel: the full
//!   form of 8.4 with Start or Stop and Remove. A double click starts or
//!   stops the rule. Right click: Start/Stop, Edit, Duplicate, Remove.
//!   Duplicate opens a new rule filled from the card ("<label> Copy"); it
//!   is saved only by Create.
//! - New forwarding opens the form of a Local rule; its menu picks the
//!   type. The step-by-step wizard of 8.3 is not built yet (D66): the
//!   form is what "Skip wizard" shows.
//! - Remove asks first, stops the rule and offers Undo for 5 s.
//!
//! The rules are those of the personal vault (the only vault whose
//! autostart flags count, data-model 11). Statuses live in the session
//! layer's forwarding service; the screen reads them once a second while
//! any rule is started. Nothing here logs addresses or ports.

mod form;
pub mod strings;
mod view;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Pixels, SharedString,
    Subscription, Task, Window,
};
use gpui_kit::component::input::{InputEvent, InputState};
use shellpp_core::service::snippets::{ForwardType, ForwardingRuleView};
use shellpp_ssh::{ForwardError, RuleStatus};
use uuid::Uuid;

pub use form::{FormMode, RuleForm};

use crate::shell::Toast;
use crate::state::{AppState, AppStateEvent};
use crate::theme::metrics;

/// How often the statuses of started rules are read again.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Stable element id of a rule card.
pub fn card_id(id: Uuid) -> SharedString {
    format!("fwd-card-{id}").into()
}

/// Stable element id of a card's status line.
pub fn status_id(id: Uuid) -> SharedString {
    format!("fwd-status-{id}").into()
}

/// The badge letter of a rule type (UI 8.2).
pub fn type_letter(kind: ForwardType) -> &'static str {
    match kind {
        ForwardType::Local => "L",
        ForwardType::Remote => "R",
        ForwardType::Dynamic => "D",
    }
}

/// The title of a card: the label, else the summary.
pub fn card_title(rule: &ForwardingRuleView) -> String {
    rule.label.clone().unwrap_or_else(|| rule.summary.clone())
}

/// What a card's status indicator shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Stopped,
    Busy,
    Running,
    Error,
}

/// The tone and the words of a rule's status (`None`: not started).
pub fn status_text(status: Option<&RuleStatus>) -> (StatusTone, String) {
    match status {
        None => (StatusTone::Stopped, strings::STOPPED.to_owned()),
        Some(RuleStatus::Starting) => (StatusTone::Busy, strings::STARTING.to_owned()),
        Some(RuleStatus::Running(_)) => (StatusTone::Running, strings::RUNNING.to_owned()),
        Some(RuleStatus::Reconnecting { attempt }) => {
            (StatusTone::Busy, strings::reconnecting(*attempt))
        }
        Some(RuleStatus::Failed(error)) => (StatusTone::Error, forward_error_text(*error)),
        Some(RuleStatus::ConnectFailed(kind)) => (
            StatusTone::Error,
            crate::session_ui::bridge::failure_of(crate::sessions::SessionError::Ssh(*kind))
                .text()
                .to_owned(),
        ),
    }
}

/// Why a rule did not start, in words (UI 8.4: a busy port names the port
/// and asks for another).
pub fn forward_error_text(error: ForwardError) -> String {
    match error {
        ForwardError::PortBusy(port) => strings::port_busy(port),
        ForwardError::PortDenied(port) => strings::port_denied(port),
        ForwardError::AddressUnavailable => strings::ADDRESS_UNAVAILABLE.to_owned(),
        ForwardError::Refused => strings::REFUSED.to_owned(),
        ForwardError::Disconnected => strings::DISCONNECTED.to_owned(),
        ForwardError::Timeout => strings::TIMEOUT.to_owned(),
    }
}

/// Whether a status means the rule is meant to run (Stop applies).
pub fn is_started(status: Option<&RuleStatus>) -> bool {
    matches!(
        status,
        Some(RuleStatus::Starting | RuleStatus::Running(_) | RuleStatus::Reconnecting { .. })
    )
}

/// Whether a rule matches the search text (case-insensitive substring of
/// the label or the summary).
pub fn matches_search(rule: &ForwardingRuleView, text: &str) -> bool {
    let text = text.trim().to_lowercase();
    text.is_empty()
        || rule
            .label
            .iter()
            .chain(std::iter::once(&rule.summary))
            .any(|field| field.to_lowercase().contains(&text))
}

/// A removal that can still be undone.
struct PendingUndo {
    id: Uuid,
    _expiry: Task<()>,
}

pub struct ForwardingScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    /// Every rule of the personal vault, in the core's order (by title).
    items: Vec<ForwardingRuleView>,
    /// Indices of `items` on display.
    shown: Vec<usize>,
    /// Statuses of the started rules, read from the session layer.
    statuses: HashMap<Uuid, RuleStatus>,
    /// Rules with a start or stop in flight.
    pending: HashSet<Uuid>,
    selected: Option<Uuid>,
    /// The rule panel: the form of the selected rule or of a new one.
    form: Option<Entity<RuleForm>>,
    /// The chosen width of the panel (UI 1.3: 320-560 px).
    panel_width: Pixels,
    /// The width of the whole screen and of the card grid, measured after
    /// layout; the grid's card columns.
    measured_width: Option<f32>,
    grid_width: Option<f32>,
    columns: usize,
    focus: FocusHandle,
    undo: Option<PendingUndo>,
    poll: Option<Task<()>>,
    /// Screenshots: the statuses on screen are set by hand, not read.
    frozen: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ForwardingScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardingScreen")
            .field("items", &self.items.len())
            .field("shown", &self.shown.len())
            .finish_non_exhaustive()
    }
}

impl Focusable for ForwardingScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ForwardingScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::SEARCH_PLACEHOLDER));
        let search_subscription = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.refilter(cx),
                InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        let state_subscription =
            cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
                AppStateEvent::RecordsChanged => this.reload(cx),
                AppStateEvent::LockChanged(_) => {
                    this.undo = None;
                    if !this.state.read(cx).is_unlocked() {
                        // Locked: the form and its values go too.
                        this.form = None;
                        this.selected = None;
                    }
                    this.reload(cx);
                }
                AppStateEvent::SelectionChanged
                | AppStateEvent::LocationChanged(_)
                | AppStateEvent::AutoLockDue => {}
            });
        let mut screen = Self {
            state,
            search,
            items: Vec::new(),
            shown: Vec::new(),
            statuses: HashMap::new(),
            pending: HashSet::new(),
            selected: None,
            form: None,
            panel_width: metrics::WIDTH_DETAILS,
            measured_width: None,
            grid_width: None,
            columns: 1,
            focus: cx.focus_handle().tab_stop(true),
            undo: None,
            poll: None,
            frozen: false,
            _subscriptions: vec![search_subscription, state_subscription],
        };
        screen.reload(cx);
        screen
    }

    // -- Reading -----------------------------------------------------------------

    pub fn items(&self) -> &[ForwardingRuleView] {
        &self.items
    }

    /// The record ids on display, in display order.
    pub fn shown_ids(&self) -> Vec<Uuid> {
        self.shown
            .iter()
            .map(|index| self.items[*index].record.record_id)
            .collect()
    }

    pub fn rule(&self, id: Uuid) -> Option<&ForwardingRuleView> {
        self.items.iter().find(|rule| rule.record.record_id == id)
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    pub fn form(&self) -> Option<&Entity<RuleForm>> {
        self.form.as_ref()
    }

    pub fn status(&self, id: Uuid) -> Option<&RuleStatus> {
        self.statuses.get(&id)
    }

    pub fn is_pending(&self, id: Uuid) -> bool {
        self.pending.contains(&id)
    }

    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// The chosen panel width while the panel is open: the shell collapses
    /// its sidebar by it (UI 1.2).
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.form.as_ref().map(|_| self.panel_width)
    }

    /// Sets the panel width (clamped to 320-560 px).
    pub fn set_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = crate::shell::details::clamp_width(width);
        if width != self.panel_width {
            self.panel_width = width;
            cx.notify();
        }
    }

    // -- Loading -------------------------------------------------------------------

    /// Reads the rules of the personal vault again (none while locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.items = match state.core().filter(|_| state.is_unlocked()) {
            Some(core) => core.forwarding_rules_personal().unwrap_or_else(|error| {
                tracing::warn!(code = error.code(), "forwarding rules could not be loaded");
                Vec::new()
            }),
            None => Vec::new(),
        };
        // The panel of a rule removed elsewhere closes.
        if let Some(id) = self.selected
            && self.rule(id).is_none()
        {
            self.selected = None;
            if self
                .form
                .as_ref()
                .is_some_and(|form| form.read(cx).mode() == FormMode::Edit(id))
            {
                self.form = None;
            }
        }
        self.read_statuses(cx);
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        self.shown = (0..self.items.len())
            .filter(|&index| matches_search(&self.items[index], &text))
            .collect();
        cx.notify();
    }

    /// Reads the statuses of the rules from the session layer; returns
    /// whether any changed.
    fn read_statuses(&mut self, cx: &App) -> bool {
        if self.frozen {
            return false;
        }
        let Some(manager) = crate::sessions::ui::manager(cx) else {
            return false;
        };
        let statuses: HashMap<Uuid, RuleStatus> = self
            .items
            .iter()
            .filter_map(|rule| {
                let id = rule.record.record_id;
                manager.forward_status(id).map(|status| (id, status))
            })
            .collect();
        let changed = statuses != self.statuses;
        self.statuses = statuses;
        changed
    }

    /// Reads the statuses once a second while any rule is started or a
    /// start or stop is in flight.
    fn poll_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.frozen {
            return;
        }
        let needed =
            !self.pending.is_empty() || self.statuses.values().any(|s| is_started(Some(s)));
        if !needed || self.poll.is_some() {
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let more = this.update(cx, |screen, cx| {
                    if screen.read_statuses(cx) {
                        cx.notify();
                    }
                    !screen.pending.is_empty()
                        || screen.statuses.values().any(|s| is_started(Some(s)))
                });
                if !matches!(more, Ok(true)) {
                    break;
                }
            }
            let _ = this.update(cx, |screen, _| screen.poll = None);
        }));
    }

    /// Shows `statuses` on the cards and stops reading the session layer's
    /// (the screenshot run: nothing connects).
    pub fn show_statuses(&mut self, statuses: Vec<(Uuid, RuleStatus)>, cx: &mut Context<Self>) {
        self.frozen = true;
        self.poll = None;
        self.statuses = statuses.into_iter().collect();
        cx.notify();
    }

    // -- Commands --------------------------------------------------------------------

    /// Selects a card and opens its rule in the panel (`None` closes it).
    pub fn select(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.filter(|id| self.rule(*id).is_some());
        self.selected = id;
        self.form = id.and_then(|id| self.rule(id).cloned()).map(|rule| {
            let state = self.state.clone();
            cx.new(|cx| RuleForm::edit(&rule, state, window, cx))
        });
        cx.notify();
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.form = None;
        cx.notify();
    }

    /// Opens the form of a new rule of `kind` (the wizard's "Skip wizard").
    pub fn new_rule(&mut self, kind: ForwardType, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        let state = self.state.clone();
        self.form = Some(cx.new(|cx| RuleForm::new(kind, state, window, cx)));
        cx.notify();
    }

    /// Duplicate: a new rule filled from `id`, labelled "<title> Copy".
    pub fn duplicate(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.rule(id).cloned() else {
            return;
        };
        self.selected = None;
        let state = self.state.clone();
        self.form = Some(cx.new(|cx| RuleForm::copy_of(&rule, state, window, cx)));
        cx.notify();
    }

    /// Double click and the menu: starts a stopped rule, stops a started one.
    pub fn toggle(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if is_started(self.statuses.get(&id)) {
            self.stop(id, cx);
        } else {
            self.start(id, window, cx);
        }
    }

    /// Starts a rule on its host's connection; a failure is shown on the
    /// card (and in a toast: the card may be out of view).
    pub fn start(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.rule(id).cloned() else {
            return;
        };
        let Some(manager) = crate::sessions::ui::manager(cx) else {
            return;
        };
        if !self.pending.insert(id) {
            return;
        }
        let started = manager.forward_start(&rule);
        self.statuses.insert(id, RuleStatus::Starting);
        self.poll_if_needed(cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let status = started.await;
            let _ = this.update_in(cx, |screen, window, cx| {
                screen.pending.remove(&id);
                tracing::info!(
                    running = matches!(status, RuleStatus::Running(_)),
                    "forwarding rule started"
                );
                if let (StatusTone::Error, text) = status_text(Some(&status)) {
                    crate::shell::show_toast(text, window, cx);
                }
                screen.statuses.insert(id, status);
                screen.read_statuses(cx);
                screen.poll_if_needed(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops a rule (its host's connection closes with its last rule).
    pub fn stop(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(manager) = crate::sessions::ui::manager(cx) else {
            return;
        };
        if !self.pending.insert(id) {
            return;
        }
        let stopped = manager.forward_stop(id);
        cx.notify();
        cx.spawn(async move |this, cx| {
            stopped.await;
            let _ = this.update(cx, |screen, cx| {
                screen.pending.remove(&id);
                screen.statuses.remove(&id);
                screen.read_statuses(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Remove (8.2): asks first, then stops and removes the rule and offers
    /// Undo.
    pub fn confirm_remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rule) = self.rule(id) else {
            return;
        };
        let body = strings::remove_body(&card_title(rule), is_started(self.statuses.get(&id)));
        let screen = cx.entity().downgrade();
        view::open_remove_confirmation(body, window, cx, move |window, cx| {
            let _ = screen.update(cx, |screen, cx| screen.remove(id, window, cx));
        });
    }

    /// Removes a rule now (after the confirmation) and shows the Undo toast.
    pub fn remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        if self.statuses.contains_key(&id) {
            self.stop(id, cx);
        }
        let removed = self
            .state
            .update(cx, |state, cx| state.delete_records(vec![id], cx));
        if let Err(error) = removed {
            tracing::warn!(code = error.code(), "forwarding rule not removed");
            crate::shell::show_toast(strings::REMOVE_FAILED, window, cx);
            return;
        }
        if self.selected == Some(id) {
            self.close_panel(cx);
        }
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::known_hosts::UNDO_WINDOW)
                .await;
            let _ = this.update(cx, |screen, _| {
                if screen.undo.as_ref().is_some_and(|undo| undo.id == id) {
                    screen.undo = None;
                }
            });
        });
        self.undo = Some(PendingUndo {
            id,
            _expiry: expiry,
        });
        self.reload(cx);
        let screen = cx.entity().downgrade();
        crate::shell::show_toast(
            Toast::new(strings::REMOVED).action_with_id(
                "toast-undo",
                crate::i18n::notifications::UNDO,
                move |window, cx| {
                    let _ = screen.update(cx, |screen, cx| screen.undo_remove(window, cx));
                },
            ),
            window,
            cx,
        );
    }

    /// Undo of the last removal: the rule comes back (stopped).
    pub fn undo_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
            return;
        };
        let restored = self
            .state
            .update(cx, |state, cx| state.restore_records(vec![undo.id], cx));
        if let Err(error) = restored {
            tracing::warn!(code = error.code(), "undo of rule removal failed");
            crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
        }
        self.reload(cx);
    }

    /// Saves the form (Create, Create and start, Save). A rule that runs
    /// starts again with the saved values.
    pub fn save(&mut self, start: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.clone() else {
            return;
        };
        let (core, vault_id) = {
            let state = self.state.read(cx);
            (state.core().cloned(), state.vault_id())
        };
        let (Some(core), Some(vault_id)) = (core, vault_id) else {
            return;
        };
        let data = match form.read(cx).data(cx) {
            Ok(data) => data,
            Err(errors) => {
                form.update(cx, |form, cx| form.set_errors(errors, cx));
                return;
            }
        };
        let record_id = match form.read(cx).mode() {
            FormMode::Edit(id) => Some(id),
            FormMode::New => None,
        };
        let input = shellpp_core::service::snippets::RecordInput {
            vault_id,
            record_id,
            data,
        };
        match core.forwarding_rule_save(input) {
            Ok(view) => {
                let id = view.record.record_id;
                tracing::info!(created = record_id.is_none(), "forwarding rule saved");
                let was_started = is_started(self.statuses.get(&id));
                self.reload(cx);
                self.select(Some(id), window, cx);
                if was_started {
                    self.restart(id, window, cx);
                } else if start {
                    self.start(id, window, cx);
                }
            }
            Err(shellpp_core::error::CoreError::Validation(fields)) => {
                form.update(cx, |form, cx| form.set_core_errors(&fields, cx));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "forwarding rule not saved");
                crate::shell::show_toast(strings::SAVE_FAILED, window, cx);
            }
        }
    }

    /// Stops and starts a rule again (its saved values changed).
    fn restart(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manager) = crate::sessions::ui::manager(cx) else {
            return;
        };
        let stopped = manager.forward_stop(id);
        self.pending.insert(id);
        cx.spawn_in(window, async move |this, cx| {
            stopped.await;
            let _ = this.update_in(cx, |screen, window, cx| {
                screen.pending.remove(&id);
                screen.statuses.remove(&id);
                screen.start(id, window, cx);
            });
        })
        .detach();
    }
}
