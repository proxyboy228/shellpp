//! The Snippets screen (UI 9; stage 5, D66): the action bar (split New
//! snippet with New snippet package in its menu; search), packages at the
//! top and snippet cards below, and the snippet panel on the right.
//!
//! - Packages are folders: a click opens one (its snippets only, with the
//!   way back above them); its menu has Open, Rename and Remove (its
//!   snippets stay, outside any package). A search looks through every
//!   package.
//! - A snippet card shows the label, else the first line of the script,
//!   and its targets. A click opens the snippet in the panel, a double click
//!   runs it on its targets; the menu has Run, Edit and Remove (with Undo).
//! - The panel (9.3): description, package, tags, the script in a
//!   monospaced multi-line field and the targets with Add targets; Run is
//!   pinned at the bottom with Save (Create for a new snippet).
//! - Run (9.4, [`run`]): the values of the script's `{{name}}` variables
//!   first, then the hosts to confirm when there are two or more, then the
//!   results table: status, exit code and the output on click, per host.
//!
//! Snippets of the open vault (the personal one in local mode). Nothing
//! here logs scripts, their output or host names.

mod form;
mod run;
pub mod strings;
mod view;

#[cfg(test)]
mod tests;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Pixels, SharedString,
    Subscription, Task, Window,
};
use gpui_kit::component::input::{InputEvent, InputState};
use shellpp_core::service::snippets::{RecordInput, SnippetPackageView, SnippetView};
use uuid::Uuid;

pub use form::{FormMode, SnippetForm};
pub use run::{RunResults, RunRow, RunState};

use crate::shell::Toast;
use crate::state::{AppState, AppStateEvent};
use crate::theme::metrics;

/// Stable element id of a snippet card.
pub fn card_id(id: Uuid) -> SharedString {
    format!("snippet-card-{id}").into()
}

/// Stable element id of a package card.
pub fn package_id(id: Uuid) -> SharedString {
    format!("snippet-package-{id}").into()
}

/// The title of a card: the label, else the first line of the script.
pub fn card_title(snippet: &SnippetView) -> String {
    if let Some(label) = &snippet.label {
        return label.clone();
    }
    snippet
        .script
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| strings::UNTITLED.to_owned(), str::to_owned)
}

/// Whether a snippet matches the search text (case-insensitive substring of
/// the label, the script or a tag).
pub fn matches_search(snippet: &SnippetView, text: &str) -> bool {
    let text = text.trim().to_lowercase();
    text.is_empty()
        || snippet
            .label
            .iter()
            .chain(std::iter::once(&snippet.script))
            .chain(snippet.tags.iter())
            .any(|field| field.to_lowercase().contains(&text))
}

/// A removal that can still be undone.
struct PendingUndo {
    id: Uuid,
    _expiry: Task<()>,
}

pub struct SnippetsScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    snippets: Vec<SnippetView>,
    packages: Vec<SnippetPackageView>,
    /// The package open (`None`: the top level).
    package: Option<Uuid>,
    /// Indices of `snippets` and `packages` on display.
    shown: Vec<usize>,
    shown_packages: Vec<usize>,
    selected: Option<Uuid>,
    form: Option<Entity<SnippetForm>>,
    panel_width: Pixels,
    measured_width: Option<f32>,
    grid_width: Option<f32>,
    columns: usize,
    focus: FocusHandle,
    undo: Option<PendingUndo>,
    /// The results of the last run, while its dialog is open.
    results: Option<Entity<RunResults>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SnippetsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnippetsScreen")
            .field("snippets", &self.snippets.len())
            .field("packages", &self.packages.len())
            .finish_non_exhaustive()
    }
}

impl Focusable for SnippetsScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SnippetsScreen {
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
                        this.form = None;
                        this.selected = None;
                        this.results = None;
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
            snippets: Vec::new(),
            packages: Vec::new(),
            package: None,
            shown: Vec::new(),
            shown_packages: Vec::new(),
            selected: None,
            form: None,
            panel_width: metrics::WIDTH_DETAILS,
            measured_width: None,
            grid_width: None,
            columns: 1,
            focus: cx.focus_handle().tab_stop(true),
            undo: None,
            results: None,
            _subscriptions: vec![search_subscription, state_subscription],
        };
        screen.reload(cx);
        screen
    }

    // -- Reading -----------------------------------------------------------------

    pub fn snippets(&self) -> &[SnippetView] {
        &self.snippets
    }

    pub fn packages(&self) -> &[SnippetPackageView] {
        &self.packages
    }

    pub fn snippet(&self, id: Uuid) -> Option<&SnippetView> {
        self.snippets
            .iter()
            .find(|snippet| snippet.record.record_id == id)
    }

    pub fn package_view(&self, id: Uuid) -> Option<&SnippetPackageView> {
        self.packages
            .iter()
            .find(|package| package.record.record_id == id)
    }

    /// The snippet ids on display, in display order.
    pub fn shown_ids(&self) -> Vec<Uuid> {
        self.shown
            .iter()
            .map(|index| self.snippets[*index].record.record_id)
            .collect()
    }

    /// The package ids on display.
    pub fn shown_package_ids(&self) -> Vec<Uuid> {
        self.shown_packages
            .iter()
            .map(|index| self.packages[*index].record.record_id)
            .collect()
    }

    pub fn open_package(&self) -> Option<Uuid> {
        self.package
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    pub fn form(&self) -> Option<&Entity<SnippetForm>> {
        self.form.as_ref()
    }

    pub fn results(&self) -> Option<&Entity<RunResults>> {
        self.results.as_ref()
    }

    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// The snippets of a package.
    pub fn package_size(&self, id: Uuid) -> usize {
        self.snippets
            .iter()
            .filter(|snippet| snippet.package_id == Some(id))
            .count()
    }

    /// The chosen panel width while the panel is open (UI 1.2).
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.form.as_ref().map(|_| self.panel_width)
    }

    pub fn set_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = crate::shell::details::clamp_width(width);
        if width != self.panel_width {
            self.panel_width = width;
            cx.notify();
        }
    }

    // -- Loading -------------------------------------------------------------------

    /// Reads the snippets and packages of the vault again (none while
    /// locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let loaded = state
            .core()
            .filter(|_| state.is_unlocked())
            .zip(state.vault_id())
            .map(|(core, vault)| {
                let snippets = core.snippets_list(vault).unwrap_or_else(|error| {
                    tracing::warn!(code = error.code(), "snippets could not be loaded");
                    Vec::new()
                });
                let packages = core.snippet_packages_list(vault).unwrap_or_else(|error| {
                    tracing::warn!(code = error.code(), "packages could not be loaded");
                    Vec::new()
                });
                (snippets, packages)
            });
        (self.snippets, self.packages) = loaded.unwrap_or_default();
        if self
            .package
            .is_some_and(|id| self.package_view(id).is_none())
        {
            self.package = None;
        }
        if let Some(id) = self.selected
            && self.snippet(id).is_none()
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
        if let Some(form) = &self.form {
            form.update(cx, |_, cx| cx.notify());
        }
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().trim().to_lowercase();
        let searching = !text.is_empty();
        self.shown = (0..self.snippets.len())
            .filter(|&index| {
                let snippet = &self.snippets[index];
                // A search looks through every package.
                (searching || snippet.package_id == self.package) && matches_search(snippet, &text)
            })
            .collect();
        self.shown_packages = if self.package.is_some() && !searching {
            Vec::new()
        } else {
            (0..self.packages.len())
                .filter(|&index| self.packages[index].name.to_lowercase().contains(&text))
                .collect()
        };
        cx.notify();
    }

    // -- Commands --------------------------------------------------------------------

    /// Opens a package (`None`: back to the top level).
    pub fn enter_package(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        self.package = id.filter(|id| self.package_view(*id).is_some());
        self.refilter(cx);
    }

    /// Selects a card and opens its snippet in the panel (`None` closes it).
    pub fn select(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.filter(|id| self.snippet(*id).is_some());
        self.selected = id;
        self.form = id.and_then(|id| self.snippet(id).cloned()).map(|snippet| {
            let state = self.state.clone();
            cx.new(|cx| SnippetForm::edit(&snippet, state, window, cx))
        });
        cx.notify();
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.form = None;
        cx.notify();
    }

    /// New snippet: in the package open, if any.
    pub fn new_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        let state = self.state.clone();
        let package = self.package;
        self.form = Some(cx.new(|cx| SnippetForm::new(package, state, window, cx)));
        cx.notify();
    }

    /// Saves the form; `then_run` runs the saved snippet after.
    pub fn save(&mut self, then_run: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        match core.snippet_save(RecordInput {
            vault_id,
            record_id,
            data,
        }) {
            Ok(view) => {
                let id = view.record.record_id;
                tracing::info!(created = record_id.is_none(), "snippet saved");
                self.reload(cx);
                self.select(Some(id), window, cx);
                if then_run {
                    self.run(id, window, cx);
                }
            }
            Err(shellpp_core::error::CoreError::Validation(fields)) => {
                form.update(cx, |form, cx| form.set_core_errors(&fields, cx));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "snippet not saved");
                crate::shell::show_toast(strings::SAVE_FAILED, window, cx);
            }
        }
    }

    /// Run (9.4): the variables' values, the confirmation for two or more
    /// hosts, then the results. A snippet without targets opens in the
    /// panel, where they are added.
    pub fn run(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snippet) = self.snippet(id).cloned() else {
            return;
        };
        if snippet.target_host_ids.is_empty() {
            if self.selected != Some(id) {
                self.select(Some(id), window, cx);
            }
            return;
        }
        // Each step that needs no dialog goes on here, inside this update.
        if snippet.variables.is_empty() {
            let script = snippet.script.clone();
            self.confirm_run(snippet, script, window, cx);
            return;
        }
        let screen = cx.entity().downgrade();
        let asked = snippet.clone();
        run::ask_values(&asked, window, cx, move |script, window, cx| {
            let _ = screen.update(cx, |screen, cx| {
                screen.confirm_run(snippet.clone(), script, window, cx)
            });
        });
    }

    fn confirm_run(
        &mut self,
        snippet: SnippetView,
        script: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hosts = self.target_titles(&snippet, cx);
        if hosts.len() < 2 {
            self.start_run(&snippet, script, window, cx);
            return;
        }
        let title = card_title(&snippet);
        let screen = cx.entity().downgrade();
        run::confirm(title, hosts, window, cx, move |window, cx| {
            let _ = screen.update(cx, |screen, cx| {
                screen.start_run(&snippet, script.clone(), window, cx)
            });
        });
    }

    /// The (host id, title) of the snippet's targets, in its order; a host
    /// removed since is left out.
    fn target_titles(&self, snippet: &SnippetView, cx: &App) -> Vec<(Uuid, String)> {
        let state = self.state.read(cx);
        snippet
            .target_host_ids
            .iter()
            .filter_map(|id| state.host(*id).map(|host| (*id, host.title().to_owned())))
            .collect()
    }

    fn start_run(
        &mut self,
        snippet: &SnippetView,
        script: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = self.target_titles(snippet, cx);
        let vault_id = snippet.record.vault_id;
        let results =
            cx.new(|cx| RunResults::start(card_title(snippet), script, vault_id, targets, cx));
        self.results = Some(results.clone());
        run::open_results(results, window, cx);
    }

    /// Shows `results` in the results dialog (the screenshot run).
    pub fn show_results(
        &mut self,
        results: Entity<RunResults>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.results = Some(results.clone());
        run::open_results(results, window, cx);
    }

    /// Remove (with Undo for 5 s).
    pub fn confirm_remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snippet) = self.snippet(id) else {
            return;
        };
        let body = strings::remove_body(&card_title(snippet));
        let screen = cx.entity().downgrade();
        view::open_confirmation(
            "snippet-remove",
            strings::REMOVE_SNIPPET_TITLE,
            body,
            window,
            cx,
            move |window, cx| {
                let _ = screen.update(cx, |screen, cx| screen.remove(id, window, cx));
            },
        );
    }

    pub fn remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let removed = self
            .state
            .update(cx, |state, cx| state.delete_records(vec![id], cx));
        if let Err(error) = removed {
            tracing::warn!(code = error.code(), "snippet not removed");
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

    pub fn undo_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
            return;
        };
        let restored = self
            .state
            .update(cx, |state, cx| state.restore_records(vec![undo.id], cx));
        if let Err(error) = restored {
            tracing::warn!(code = error.code(), "undo of snippet removal failed");
            crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
        }
        self.reload(cx);
    }

    /// New snippet package / Rename: a dialog asks for the name.
    pub fn edit_package(&mut self, id: Option<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let name = id
            .and_then(|id| self.package_view(id))
            .map(|package| package.name.clone())
            .unwrap_or_default();
        let screen = cx.entity().downgrade();
        view::open_package_dialog(id, name, window, cx, move |name, window, cx| {
            screen
                .update(cx, |screen, cx| screen.save_package(id, name, window, cx))
                .unwrap_or(false)
        });
    }

    /// Saves a package's name; false (the dialog stays) when it is refused.
    pub fn save_package(
        &mut self,
        id: Option<Uuid>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (core, vault_id) = {
            let state = self.state.read(cx);
            (state.core().cloned(), state.vault_id())
        };
        let (Some(core), Some(vault_id)) = (core, vault_id) else {
            return false;
        };
        let name = name.trim().to_owned();
        if name.is_empty() {
            return false;
        }
        let mut data = serde_json::Map::new();
        data.insert("name".into(), serde_json::Value::String(name));
        match core.snippet_package_save(RecordInput {
            vault_id,
            record_id: id,
            data,
        }) {
            Ok(_) => {
                tracing::info!(created = id.is_none(), "snippet package saved");
                self.reload(cx);
                true
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "snippet package not saved");
                crate::shell::show_toast(strings::SAVE_FAILED, window, cx);
                false
            }
        }
    }

    /// Remove of a package: its snippets stay, outside any package.
    pub fn confirm_remove_package(
        &mut self,
        id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(package) = self.package_view(id) else {
            return;
        };
        let body = strings::remove_package_body(&package.name, self.package_size(id));
        let screen = cx.entity().downgrade();
        view::open_confirmation(
            "package-remove",
            strings::REMOVE_PACKAGE_TITLE,
            body,
            window,
            cx,
            move |window, cx| {
                let _ = screen.update(cx, |screen, cx| {
                    let removed = screen
                        .state
                        .update(cx, |state, cx| state.delete_records(vec![id], cx));
                    match removed {
                        Ok(_) => {
                            if screen.package == Some(id) {
                                screen.package = None;
                            }
                            screen.reload(cx);
                            crate::shell::show_toast(strings::PACKAGE_REMOVED, window, cx);
                        }
                        Err(error) => {
                            tracing::warn!(code = error.code(), "snippet package not removed");
                            crate::shell::show_toast(strings::REMOVE_FAILED, window, cx);
                        }
                    }
                });
            },
        );
    }
}
