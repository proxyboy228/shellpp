//! The Known Hosts screen (UI 10.1; lead decision L8 of stage 2b): the
//! action bar (Import, Export; search, view toggle, sorting), the grid of
//! cards or the list, the details panel on the right, the card menu (copy
//! fingerprint, Remove), Remove with confirmation and Undo, the import
//! dialog with its summary and the export to a known_hosts file.
//!
//! - A card shows the fingerprint icon and the address in the OpenSSH form
//!   (`host`, `[host]:port`); a hashed entry shows "Hashed host" and its key
//!   type; a revoked key carries a "revoked" badge.
//! - A click selects the card and opens the details panel (address, port,
//!   key type, SHA256 fingerprint with a copy button, date added, saved
//!   hosts with this address, Remove). The panel sits on the right inside
//!   the screen, 380 px and resizable like the details panel of UI 1.3.
//! - Right click: Copy fingerprint, Remove. Delete on the selected card
//!   removes it too; arrows move the selection; Esc closes the panel.
//! - Remove asks first; the toast offers Undo for 5 s (as hosts, UI 2.4):
//!   the entry comes back under its record id.
//! - Import (`import`): a file path (default `~/.ssh/known_hosts`, see
//!   [`default_import_path`]), then the summary of four numbers. Export
//!   writes every entry to a file chosen in the save dialog.
//!
//! Connections check host keys only against this list (the core's
//! `known_host_check`); the system known_hosts file is read only by Import.
//! Nothing here logs addresses, fingerprints or paths.
//!
//! Loading (decision of stage 2b): the list is read from the core on a
//! background thread, never on the UI thread. A change of the records only
//! marks the list stale; the screen loads it when it is drawn, so a window
//! that does not show Known Hosts does no work for a large import, and the
//! changes that arrive while a load runs are covered by one more load after
//! it.

mod import;
pub mod strings;
mod view;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable, Global,
    KeyDownEvent, Pixels, SharedString, Size, Subscription, Task, WeakEntity, Window,
};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputEvent, InputState};
use shellpp_core::service::dto::{KnownHostView, RecordsRef};
use uuid::Uuid;

use crate::shell::Toast;
use crate::state::{AppState, AppStateEvent};
use crate::sync_ui::duplicates::DuplicatesHint;
use crate::theme::metrics;

pub use import::{ImportDialog, ImportStep};

/// How long a removal can be undone: the Undo toast's 5 seconds (as hosts,
/// `crate::state::undo::UNDO_WINDOW`).
pub const UNDO_WINDOW: Duration = metrics::TOAST_DURATION;

/// The file the import suggests (UI 10.1: `~/.ssh/known_hosts`). Tests, the
/// demo and screenshots set it to a temporary file (or `None`) with
/// [`set_import_source`], so they never read the real one.
#[derive(Clone, Debug, Default)]
pub struct ImportSource(pub Option<PathBuf>);

impl Global for ImportSource {}

/// Overrides the suggested import file for this app.
pub fn set_import_source(path: Option<PathBuf>, cx: &mut App) {
    cx.set_global(ImportSource(path));
}

/// Opens the known_hosts import dialog over the current window: the import
/// dialog of UI 2.5 hands its known_hosts source over to it (D39).
pub fn open_import_dialog(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let path = default_import_path(cx);
    let dialog = cx.new(|cx| ImportDialog::new(state, path, window, cx));
    import::open(dialog, window, cx);
}

/// The suggested import file: the override of [`set_import_source`], else
/// nothing in a test run (a test build, or `SHELLPP_TEST=1`), else
/// `~/.ssh/known_hosts`.
pub fn default_import_path(cx: &App) -> Option<PathBuf> {
    if let Some(source) = cx.try_global::<ImportSource>() {
        return source.0.clone();
    }
    if crate::dirs::is_test_run() {
        return None;
    }
    ::dirs::home_dir().map(|home| home.join(".ssh").join("known_hosts"))
}

/// Setting key of the view toggle (`grid` or `list`; docs/data-model.md,
/// `view.<screen>.layout`). The sort is not remembered: its modes are not
/// those of `view.<screen>.sort`.
pub const LAYOUT_KEY: &str = "view.known_hosts.layout";

/// The layout of the content (the view toggle), kept in [`LAYOUT_KEY`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnownHostsLayout {
    #[default]
    Tiles,
    List,
}

impl KnownHostsLayout {
    fn setting(self) -> &'static str {
        match self {
            Self::Tiles => "grid",
            Self::List => "list",
        }
    }

    /// The saved layout (tiles by default).
    fn saved(state: &AppState) -> Self {
        let settings = state.core().and_then(|core| core.settings_get().ok());
        match settings
            .as_ref()
            .and_then(|values| values.get(LAYOUT_KEY))
            .and_then(serde_json::Value::as_str)
        {
            Some("list") => Self::List,
            _ => Self::Tiles,
        }
    }
}

/// The sort menu (decision of stage 2b).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnownHostsSort {
    /// Display address A to Z, hashed entries last (the core's order).
    #[default]
    Address,
    /// Newest first.
    Added,
    /// Key type A to Z, then the address.
    KeyType,
}

impl KnownHostsSort {
    pub const ALL: [Self; 3] = [Self::Address, Self::Added, Self::KeyType];
}

/// Stable element id of a card (grid) or row (list).
pub fn card_id(id: Uuid) -> SharedString {
    format!("known-host-card-{id}").into()
}

/// Stable element id of the "Hashed host" title of a card.
pub fn hashed_title_id(id: Uuid) -> SharedString {
    format!("known-host-hashed-{id}").into()
}

/// Stable element id of the "revoked" badge of a card.
pub fn revoked_badge_id(id: Uuid) -> SharedString {
    format!("known-host-revoked-{id}").into()
}

/// Stable element id of a saved host in the details panel.
pub fn panel_host_id(id: Uuid) -> SharedString {
    format!("kh-panel-host-{id}").into()
}

/// The title of a card: the address in the OpenSSH form, or "Hashed host".
pub fn card_title(view: &KnownHostView) -> String {
    if view.hashed {
        strings::HASHED_HOST.to_owned()
    } else {
        view.display_address
            .clone()
            .unwrap_or_else(|| view.host.clone())
    }
}

/// The subtitle of a card: the key type.
pub fn card_subtitle(view: &KnownHostView) -> String {
    view.key_type.clone()
}

/// The port of the details panel: hashed entries keep it inside the hash.
pub fn port_text(view: &KnownHostView) -> String {
    if view.hashed {
        strings::HASHED_PORT.to_owned()
    } else {
        view.port.unwrap_or(22).to_string()
    }
}

/// "Date added" from the record's UTC timestamp, in local time and the
/// locale's format (D68; [`crate::dates`]); anything else as it is.
pub fn date_text(timestamp: &str) -> String {
    crate::dates::timestamp_text(timestamp)
}

/// Whether an entry matches the search text (case-insensitive substring of
/// the card title, the key type, the fingerprint or the comment).
pub fn matches_search(view: &KnownHostView, text: &str) -> bool {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return true;
    }
    [
        Some(card_title(view)),
        Some(view.key_type.clone()),
        Some(view.fingerprint.clone()),
        view.comment.clone(),
    ]
    .into_iter()
    .flatten()
    .any(|field| field.to_lowercase().contains(&text))
}

/// The indices of `items` on display: those matching `text`, in `sort`
/// order (`items` come in the core's address order).
pub fn listing(items: &[KnownHostView], text: &str, sort: KnownHostsSort) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..items.len())
        .filter(|&index| matches_search(&items[index], text))
        .collect();
    match sort {
        KnownHostsSort::Address => {}
        KnownHostsSort::Added => shown.sort_by(|a, b| {
            items[*b]
                .created_at
                .cmp(&items[*a].created_at)
                .then_with(|| items[*b].record_id.cmp(&items[*a].record_id))
        }),
        KnownHostsSort::KeyType => {
            shown.sort_by(|a, b| items[*a].key_type.cmp(&items[*b].key_type).then(a.cmp(b)))
        }
    }
    shown
}

/// A removal that can still be undone.
struct PendingUndo {
    ids: Vec<Uuid>,
    /// Ends the window after [`UNDO_WINDOW`].
    _expiry: Task<()>,
}

pub struct KnownHostsScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    layout: KnownHostsLayout,
    sort: KnownHostsSort,
    /// Every entry, in the core's order.
    items: Vec<KnownHostView>,
    /// Indices of `items` on display.
    shown: Vec<usize>,
    /// The card whose details are open.
    selected: Option<Uuid>,
    /// The chosen width of the details panel (UI 1.3: 320-560 px).
    panel_width: Pixels,
    /// The width of the whole screen, measured after layout.
    measured_width: Option<f32>,
    /// Card columns and card width of the last layout.
    columns: usize,
    card_width: f32,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    /// (columns, shown, width) the rows were built for.
    layout_key: (usize, usize, i32),
    scroll: VirtualListScrollHandle,
    focus: FocusHandle,
    undo: Option<PendingUndo>,
    import_dialog: WeakEntity<ImportDialog>,
    /// [`AppState::records_generation`] of the last load started (`None`
    /// before the first): history and connection log writes do not reload
    /// the list.
    loaded_generation: Option<u64>,
    /// The records changed since the last load: the next render loads again.
    stale: bool,
    /// The load in flight: the core's list, read on a background thread.
    loading: Option<Task<()>>,
    /// Loads started so far (tests).
    #[cfg(test)]
    reloads: usize,
    /// "2 known hosts may be duplicates" with Review (UI 13.7).
    duplicates_hint: Entity<DuplicatesHint>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for KnownHostsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnownHostsScreen")
            .field("items", &self.items.len())
            .field("shown", &self.shown.len())
            .finish_non_exhaustive()
    }
}

impl Focusable for KnownHostsScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KnownHostsScreen {
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
                AppStateEvent::RecordsChanged => {
                    let generation = this.state.read(cx).records_generation();
                    if this.loaded_generation != Some(generation) {
                        this.invalidate(cx);
                    }
                }
                AppStateEvent::LockChanged(_) => {
                    // The core forgot its undo buffer with the keys.
                    this.undo = None;
                    if !this.state.read(cx).is_unlocked() {
                        // Locked: nothing stays on screen or in memory.
                        this.loading = None;
                        this.items = Vec::new();
                        this.refilter(cx);
                    }
                    this.invalidate(cx);
                }
                AppStateEvent::SelectionChanged
                | AppStateEvent::LocationChanged(_)
                | AppStateEvent::AutoLockDue => {}
            });
        let layout = KnownHostsLayout::saved(state.read(cx));
        let duplicates_hint = cx.new(|cx| {
            DuplicatesHint::new(
                state.clone(),
                "kh-duplicates",
                crate::sync_ui::duplicates::KNOWN_HOSTS,
                view::CONTENT_PADDING,
                cx,
            )
        });
        Self {
            state,
            search,
            layout,
            sort: KnownHostsSort::default(),
            items: Vec::new(),
            shown: Vec::new(),
            selected: None,
            panel_width: metrics::at_100(metrics::WIDTH_DETAILS),
            measured_width: None,
            columns: 1,
            card_width: 0.,
            row_sizes: Rc::default(),
            layout_key: (0, usize::MAX, 0),
            scroll: VirtualListScrollHandle::new(),
            // The cards are one tab stop; the arrows move inside them.
            focus: cx.focus_handle().tab_stop(true),
            undo: None,
            import_dialog: WeakEntity::new_invalid(),
            loaded_generation: None,
            // The first render loads the list.
            stale: true,
            loading: None,
            #[cfg(test)]
            reloads: 0,
            duplicates_hint,
            _subscriptions: vec![search_subscription, state_subscription],
        }
    }

    // -- Reading -----------------------------------------------------------------

    /// Every entry, in the core's order.
    pub fn items(&self) -> &[KnownHostView] {
        &self.items
    }

    /// The record ids on display, in display order.
    pub fn shown_ids(&self) -> Vec<Uuid> {
        self.shown
            .iter()
            .map(|index| self.items[*index].record_id)
            .collect()
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    /// The entry whose details are open (it still exists).
    pub fn selected_view(&self) -> Option<&KnownHostView> {
        let id = self.selected?;
        self.items.iter().find(|view| view.record_id == id)
    }

    pub fn has_panel(&self) -> bool {
        self.selected_view().is_some()
    }

    /// The chosen width of the details panel while it is open: the shell
    /// collapses its sidebar by it (UI 1.2), as for the host panel.
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.has_panel().then_some(self.panel_width)
    }

    pub fn layout(&self) -> KnownHostsLayout {
        self.layout
    }

    pub fn sort(&self) -> KnownHostsSort {
        self.sort
    }

    /// Whether the last removal can still be undone.
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    fn view(&self, id: Uuid) -> Option<&KnownHostView> {
        self.items.iter().find(|view| view.record_id == id)
    }

    // -- Loading -------------------------------------------------------------------

    /// Whether a load is in flight (tests).
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The records changed: the list loads again when the screen is drawn
    /// next (at once while it is shown, when it is shown otherwise).
    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        cx.notify();
    }

    /// Reloads the entries from the core (empty while locked) on a
    /// background thread: now, or after the load in flight.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            self.invalidate(cx);
        } else {
            self.start_load(cx);
        }
    }

    /// Called by render: a stale list loads once the load in flight ended.
    fn load_if_stale(&mut self, cx: &mut Context<Self>) {
        if self.stale && self.loading.is_none() {
            self.start_load(cx);
        }
    }

    fn start_load(&mut self, cx: &mut Context<Self>) {
        self.stale = false;
        let state = self.state.read(cx);
        self.loaded_generation = Some(state.records_generation());
        #[cfg(test)]
        {
            self.reloads += 1;
        }
        let Some(core) = state.core().filter(|_| state.is_unlocked()).cloned() else {
            self.loading = None;
            self.items = Vec::new();
            self.refilter(cx);
            return;
        };
        let listed = cx.background_spawn(async move { core.known_hosts_list() });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let listed = listed.await;
            let _ = this.update(cx, |screen, cx| {
                screen.loading = None;
                // Locked while loading: the lock emptied the list already.
                if !screen.state.read(cx).is_unlocked() {
                    return;
                }
                screen.items = listed.unwrap_or_else(|error| {
                    tracing::warn!(code = error.code(), "known hosts could not be loaded");
                    Vec::new()
                });
                screen.refilter(cx);
            });
        }));
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        self.shown = listing(&self.items, &text, self.sort);
        // A search that hides the selected card keeps its panel: the panel
        // closes only with its entry (removed elsewhere) or explicitly.
        self.layout_key.1 = usize::MAX;
        cx.notify();
    }

    // -- Commands --------------------------------------------------------------------

    pub fn set_layout(&mut self, layout: KnownHostsLayout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            self.layout_key.1 = usize::MAX;
            // Remembered across restarts, as on the Hosts screen (UI 2.2).
            if let Some(core) = self.state.read(cx).core() {
                let values = [(LAYOUT_KEY.to_owned(), serde_json::json!(layout.setting()))];
                let input = shellpp_core::service::dto::SettingsSetInput {
                    values: values.into_iter().collect(),
                    reset: Vec::new(),
                };
                if let Err(error) = core.settings_set(input) {
                    tracing::warn!(code = error.code(), "known hosts layout not saved");
                }
            }
            cx.notify();
        }
    }

    pub fn set_sort(&mut self, sort: KnownHostsSort, cx: &mut Context<Self>) {
        if self.sort != sort {
            self.sort = sort;
            self.refilter(cx);
        }
    }

    /// Selects a card and opens its details (`None` closes the panel).
    pub fn select(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        let id = id.filter(|id| self.view(*id).is_some());
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.select(None, cx);
    }

    /// Sets the details panel width (clamped to 320-560 px).
    pub fn set_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = crate::shell::details::clamp_width(width);
        if width != self.panel_width {
            self.panel_width = width;
            cx.notify();
        }
    }

    /// Copies an entry's SHA256 fingerprint and says so in a toast.
    pub fn copy_fingerprint(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(fingerprint) = self.view(id).map(|view| view.fingerprint.clone()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(fingerprint));
        crate::shell::show_toast(strings::FINGERPRINT_COPIED, window, cx);
    }

    /// Remove (10.1): asks first, then removes and offers Undo.
    pub fn confirm_remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.view(id) else {
            return;
        };
        let body = strings::remove_body(&card_title(view), view.revoked);
        let screen = cx.entity().downgrade();
        view::open_remove_confirmation(body, window, cx, move |window, cx| {
            let _ = screen.update(cx, |screen, cx| screen.remove(id, window, cx));
        });
    }

    /// Removes an entry now (after the confirmation) and shows the Undo
    /// toast.
    pub fn remove(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let (core, vault_id) = {
            let state = self.state.read(cx);
            (state.core().cloned(), state.vault_id())
        };
        let (Some(core), Some(vault_id)) = (core, vault_id) else {
            return;
        };
        let removed = core.known_host_remove(RecordsRef {
            vault_id,
            record_ids: vec![id],
        });
        if let Err(error) = removed {
            tracing::warn!(code = error.code(), "known host not removed");
            crate::shell::show_toast(strings::REMOVE_FAILED, window, cx);
            return;
        }
        if self.selected == Some(id) {
            self.selected = None;
        }
        let expiry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO_WINDOW).await;
            let _ = this.update(cx, |screen, _| {
                if screen.undo.as_ref().is_some_and(|undo| undo.ids == [id]) {
                    screen.undo = None;
                }
            });
        });
        self.undo = Some(PendingUndo {
            ids: vec![id],
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

    /// Undo of the last removal: the entry comes back under its record id.
    pub fn undo_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
            return;
        };
        let (core, vault_id) = {
            let state = self.state.read(cx);
            (state.core().cloned(), state.vault_id())
        };
        let (Some(core), Some(vault_id)) = (core, vault_id) else {
            return;
        };
        match core.known_host_restore(RecordsRef {
            vault_id,
            record_ids: undo.ids,
        }) {
            Ok(_) => tracing::info!("known host removal undone"),
            Err(error) => {
                tracing::warn!(code = error.code(), "undo of known host removal failed");
                crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
            }
        }
        self.reload(cx);
    }

    /// Opens the import dialog (2.5 with the known_hosts source).
    pub fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = default_import_path(cx);
        let state = self.state.clone();
        let dialog = cx.new(|cx| ImportDialog::new(state, path, window, cx));
        self.import_dialog = dialog.downgrade();
        import::open(dialog, window, cx);
    }

    /// The import dialog opened last, while it is open.
    pub fn import_dialog(&self) -> Option<Entity<ImportDialog>> {
        self.import_dialog.upgrade()
    }

    /// Export: asks where to save, then writes every entry there.
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = default_import_path(cx)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(::dirs::home_dir)
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(strings::EXPORT_FILE_NAME));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let _ = this.update_in(cx, |screen, window, cx| screen.export_to(path, window, cx));
        })
        .detach();
    }

    /// Writes the known_hosts file of every entry to `path` (storage-format
    /// 13.6) and says how it went in a toast.
    pub fn export_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let written = core
            .known_hosts_export()
            .map_err(|error| tracing::warn!(code = error.code(), "known hosts not exported"))
            .and_then(|text| {
                std::fs::write(&path, text)
                    .map_err(|_| tracing::warn!("known hosts file not written"))
            });
        let message = match written {
            Ok(()) => strings::EXPORTED,
            Err(()) => strings::EXPORT_FAILED,
        };
        crate::shell::show_toast(message, window, cx);
    }

    /// Arrows move the selection (the grid's columns for up and down);
    /// Delete removes the selected entry; Esc closes the panel.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let ids = self.shown_ids();
        let current = self
            .selected
            .and_then(|id| ids.iter().position(|shown| *shown == id));
        let step = match keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            "up" => -(self.row_columns() as isize),
            "down" => self.row_columns() as isize,
            "delete" | "backspace" => {
                if let Some(id) = self.selected {
                    cx.stop_propagation();
                    self.confirm_remove(id, window, cx);
                }
                return;
            }
            "escape" if self.selected.is_some() => {
                cx.stop_propagation();
                self.close_panel(cx);
                return;
            }
            _ => return,
        };
        if ids.is_empty() {
            return;
        }
        cx.stop_propagation();
        let next = match current {
            None => 0,
            Some(index) => (index as isize + step).clamp(0, ids.len() as isize - 1) as usize,
        };
        self.select(Some(ids[next]), cx);
        let row = next / self.row_columns();
        self.scroll
            .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
    }

    fn row_columns(&self) -> usize {
        match self.layout {
            KnownHostsLayout::Tiles => self.columns.max(1),
            KnownHostsLayout::List => 1,
        }
    }
}
