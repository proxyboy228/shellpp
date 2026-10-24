//! The Keychain screen (UI 7.1-7.3; night plan of stage 3, items 1-4;
//! decisions D32 and D33): the action bar (the New key split button with
//! Generate key and New identity, Certificate and FIDO2; search, view
//! toggle, sorting), the Keys section with the Identities below it as a
//! card grid or a list, and the details panel on the right inside the
//! screen (380 px, resizable like the details panel of UI 1.3) with one of:
//!
//! - the key panel ([`KeyPanel`], 7.3) of the selected key: label (saved as
//!   in UI 1.3), type, SHA256 fingerprint, the private key hidden behind an
//!   explicit Show and hidden again when the panel is left, Copy private key
//!   through the core (cleared after 30 s), the public key;
//! - Generate key ([`GenerateForm`], 7.1): type, label, optional
//!   passphrase with the save / ask choice, progress while it runs;
//! - New key, the import ([`ImportForm`]): paste, choose a file or drop
//!   one; the core's `inspect` tells the format, whether a passphrase is
//!   needed, or why the text is not a usable key; a duplicate selects the
//!   key that is already there.
//!
//! Identities are read-only cards here (their panel, UI 7.4, comes later):
//! the name and the authentication method. Certificate, FIDO2 and New
//! identity are shown disabled (D30), and so are Export to host, Duplicate
//! and Export to file in the key panel. Remove in the key panel's "…" menu
//! asks first, naming the hosts, groups and identities that use the key,
//! and offers Undo (D50).
//!
//! Loading: the keys and identities are read from the core on a background
//! thread, as on the Known Hosts screen; a change of the records marks the
//! lists stale and the screen loads them when it is drawn. Nothing here logs
//! labels, key text, passphrases, fingerprints or file names.

mod certificate;
pub mod export_host;
mod forms;
mod panel;
pub mod strings;
mod view;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    KeyDownEvent, Pixels, SharedString, Size, Subscription, Task, Window,
};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputEvent, InputState};
use shellpp_core::service::dto::KeyView;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::state::records_edit::IdentityView;
use crate::state::{AppState, AppStateEvent};
use crate::theme::metrics;

pub use forms::{GenerateForm, ImportForm, Inspection, KeyFamily};
pub use panel::KeyPanel;

/// Setting keys of the view toggle and the sort (docs/data-model.md,
/// `view.<screen>.layout` and `view.<screen>.sort`).
pub const LAYOUT_KEY: &str = "view.keychain.layout";
pub const SORT_KEY: &str = "view.keychain.sort";

/// Largest file read as a key (a key is at most 32 KiB, docs/data-model.md
/// 9.1; a `.ppk` with a comment stays well below this).
pub const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;
/// Most entries of `~/.ssh` looked at by "Import from ~/.ssh".
const MAX_SSH_ENTRIES: usize = 256;

/// The folder "Import from ~/.ssh" lists. Tests, the demo and screenshots
/// set it to a temporary folder (or `None`) with [`set_ssh_folder`], so they
/// never read the real one.
#[derive(Clone, Debug, Default)]
pub struct SshFolder(pub Option<PathBuf>);

impl Global for SshFolder {}

/// Overrides the folder of "Import from ~/.ssh" for this app.
pub fn set_ssh_folder(path: Option<PathBuf>, cx: &mut App) {
    cx.set_global(SshFolder(path));
}

/// The folder of "Import from ~/.ssh": the override of [`set_ssh_folder`],
/// else nothing in a test run (a test build, or `SHELLPP_TEST=1`), else
/// `~/.ssh`.
pub fn default_ssh_folder(cx: &App) -> Option<PathBuf> {
    if let Some(folder) = cx.try_global::<SshFolder>() {
        return folder.0.clone();
    }
    if crate::dirs::is_test_run() {
        return None;
    }
    ::dirs::home_dir().map(|home| home.join(".ssh"))
}

/// Why a key file could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileProblem {
    Unreadable,
    TooLarge,
}

impl FileProblem {
    pub fn text(self) -> &'static str {
        match self {
            Self::Unreadable => strings::FILE_NOT_READ,
            Self::TooLarge => strings::FILE_TOO_LARGE,
        }
    }
}

/// Reads a chosen, dropped or listed key file (size-checked first). The
/// text is wiped when the returned value is dropped.
pub fn read_key_file(path: &Path) -> Result<Zeroizing<String>, FileProblem> {
    let size = std::fs::metadata(path)
        .map_err(|_| FileProblem::Unreadable)?
        .len();
    if size > MAX_KEY_FILE_BYTES {
        return Err(FileProblem::TooLarge);
    }
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|_| FileProblem::Unreadable)?);
    std::str::from_utf8(&bytes)
        .map(|text| Zeroizing::new(text.to_owned()))
        .map_err(|_| FileProblem::Unreadable)
}

/// A private key file found in the `~/.ssh` folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshKeyFile {
    /// The file name (shown in the list; never logged).
    pub name: String,
    pub path: PathBuf,
}

/// What "Import from ~/.ssh" found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshScan {
    /// Still reading the folder.
    Scanning,
    /// The private key files, by name.
    Found(Vec<SshKeyFile>),
    /// No folder to read (none chosen, or it does not exist).
    Missing,
}

/// The private key files of `folder`: regular files up to
/// [`MAX_KEY_FILE_BYTES`] whose text is a private key in a supported format
/// (the core's `detect`; `.pub` files, `known_hosts`, `config` and the like
/// are not). Blocking: run it on a background thread.
pub fn scan_ssh_folder(folder: &Path) -> Option<Vec<SshKeyFile>> {
    let entries = std::fs::read_dir(folder).ok()?;
    let mut found: Vec<SshKeyFile> = entries
        .flatten()
        .take(MAX_SSH_ENTRIES)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_str()?.to_owned();
            if name.ends_with(".pub") || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let text = read_key_file(&path).ok()?;
            let normalized = shellpp_core::keys::normalize_text(&text);
            shellpp_core::keys::convert::detect(&normalized)?;
            Some(SshKeyFile { name, path })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Some(found)
}

/// The layout of the content (the view toggle), kept in [`LAYOUT_KEY`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeychainLayout {
    #[default]
    Tiles,
    List,
}

impl KeychainLayout {
    fn setting(self) -> &'static str {
        match self {
            Self::Tiles => "grid",
            Self::List => "list",
        }
    }
}

/// The sort menu (UI 7.1), kept in [`SORT_KEY`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeychainSort {
    /// Label A to Z (the core's order).
    #[default]
    Name,
    /// Newest first.
    Created,
    /// Last changed first.
    Modified,
}

impl KeychainSort {
    pub const ALL: [Self; 3] = [Self::Name, Self::Created, Self::Modified];

    fn setting(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Created => "created",
            Self::Modified => "modified",
        }
    }
}

/// The saved layout and sort (tiles and by name by default).
fn saved_view(state: &AppState) -> (KeychainLayout, KeychainSort) {
    let settings = state.core().and_then(|core| core.settings_get().ok());
    let text = |key: &str| {
        settings
            .as_ref()
            .and_then(|values| values.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let layout = match text(LAYOUT_KEY).as_deref() {
        Some("list") => KeychainLayout::List,
        _ => KeychainLayout::Tiles,
    };
    let sort = match text(SORT_KEY).as_deref() {
        Some("created") => KeychainSort::Created,
        Some("modified") => KeychainSort::Modified,
        _ => KeychainSort::Name,
    };
    (layout, sort)
}

/// Stable element id of a key card (grid) or row (list).
pub fn key_card_id(id: Uuid) -> SharedString {
    format!("key-card-{id}").into()
}

/// Stable element id of a key card's subtitle ("Type ...").
pub fn key_subtitle_id(id: Uuid) -> SharedString {
    format!("key-subtitle-{id}").into()
}

/// Stable element id of the warning icon of a key of type "unknown".
pub fn key_unknown_id(id: Uuid) -> SharedString {
    format!("key-unknown-{id}").into()
}

/// Stable element id of a key row's Type cell (list).
pub fn key_type_cell_id(id: Uuid) -> SharedString {
    format!("key-type-cell-{id}").into()
}

/// Stable element id of a key row's Fingerprint cell (list).
pub fn key_fingerprint_cell_id(id: Uuid) -> SharedString {
    format!("key-fingerprint-cell-{id}").into()
}

/// Stable element id of an identity card (grid) or row (list).
pub fn identity_card_id(id: Uuid) -> SharedString {
    format!("identity-card-{id}").into()
}

/// Stable element id of a key card's sync conflict badge (UI 13.6).
pub fn key_conflict_badge_id(id: Uuid) -> SharedString {
    format!("key-conflict-{id}").into()
}

/// Stable element id of an identity card's sync conflict badge (UI 13.6).
pub fn identity_conflict_badge_id(id: Uuid) -> SharedString {
    format!("identity-conflict-{id}").into()
}

/// Stable element id of an identity card's subtitle (the method).
pub fn identity_subtitle_id(id: Uuid) -> SharedString {
    format!("identity-subtitle-{id}").into()
}

/// Stable element id of a file of the "Keys in ~/.ssh" list.
pub fn ssh_file_id(name: &str) -> SharedString {
    format!("kc-ssh-file-{name}").into()
}

/// The type shown for a key's algorithm (UI 7.2: ED25519, RSA, ECDSA with
/// its curve); `None` for a key this client cannot read ("unknown").
pub fn key_type(algorithm: &str) -> Option<&'static str> {
    match algorithm {
        "ssh-ed25519" => Some("ED25519"),
        "ecdsa-sha2-nistp256" => Some("ECDSA P-256"),
        "ecdsa-sha2-nistp384" => Some("ECDSA P-384"),
        "ecdsa-sha2-nistp521" => Some("ECDSA P-521"),
        "ssh-rsa" => Some("RSA"),
        _ => None,
    }
}

/// The type of a key, or "unknown".
pub fn key_type_text(view: &KeyView) -> &'static str {
    key_type(&view.algorithm).unwrap_or(strings::TYPE_UNKNOWN)
}

/// The subtitle of a key card: "Type" and the type.
pub fn key_subtitle(view: &KeyView) -> String {
    format!("{} {}", strings::TYPE, key_type_text(view))
}

/// The title of a key card: the label, else the type.
pub fn key_title(view: &KeyView) -> String {
    if view.label.trim().is_empty() {
        key_type_text(view).to_owned()
    } else {
        view.label.clone()
    }
}

/// The authentication method of an identity (UI 7.2).
pub fn identity_method(view: &IdentityView) -> &'static str {
    match (view.password_set, view.key_id.is_some()) {
        (true, true) => strings::METHOD_BOTH,
        (true, false) => strings::METHOD_PASSWORD,
        (false, true) => strings::METHOD_KEY,
        (false, false) => strings::METHOD_NONE,
    }
}

fn matches_text(fields: &[&str], text: &str) -> bool {
    let text = text.trim().to_lowercase();
    text.is_empty()
        || fields
            .iter()
            .any(|field| field.to_lowercase().contains(&text))
}

/// Whether a key matches the search text (case-insensitive substring of
/// the label, the type or the fingerprint).
pub fn key_matches(view: &KeyView, text: &str) -> bool {
    matches_text(
        &[
            &view.label,
            key_type_text(view),
            view.fingerprint.as_deref().unwrap_or(""),
        ],
        text,
    )
}

/// Whether an identity matches the search text (the name or the username).
pub fn identity_matches(view: &IdentityView, text: &str) -> bool {
    matches_text(&[&view.label, view.username.as_deref().unwrap_or("")], text)
}

/// The indices of `items` on display: those passing `matches`, in `sort`
/// order (`items` come in the core's label order).
fn listing<T>(
    items: &[T],
    matches: impl Fn(&T) -> bool,
    sort: KeychainSort,
    record: impl Fn(&T) -> (Uuid, &str),
) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..items.len())
        .filter(|&index| matches(&items[index]))
        .collect();
    match sort {
        KeychainSort::Name => {}
        // UUIDv7 ids sort by creation time.
        KeychainSort::Created => {
            shown.sort_by(|a, b| record(&items[*b]).0.cmp(&record(&items[*a]).0))
        }
        KeychainSort::Modified => shown.sort_by(|a, b| {
            let (a_id, a_modified) = record(&items[*a]);
            let (b_id, b_modified) = record(&items[*b]);
            b_modified.cmp(a_modified).then_with(|| b_id.cmp(&a_id))
        }),
    }
    shown
}

/// The keys on display, in display order.
pub fn key_listing(keys: &[KeyView], text: &str, sort: KeychainSort) -> Vec<usize> {
    listing(
        keys,
        |view| key_matches(view, text),
        sort,
        |view| (view.record_id, view.modified_at.as_str()),
    )
}

/// The identities on display, in display order.
pub fn identity_listing(identities: &[IdentityView], text: &str, sort: KeychainSort) -> Vec<usize> {
    listing(
        identities,
        |view| identity_matches(view, text),
        sort,
        |view| (view.record_id, view.modified_at.as_str()),
    )
}

/// One row of the virtual list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    KeysHeading,
    /// The list layout's column names of a section.
    KeyColumns,
    /// "No keys yet" (identities exist, keys do not).
    NoKeys,
    /// Keys `start..start + columns` of the listing.
    Keys(usize),
    IdentitiesHeading,
    IdentityColumns,
    Identities(usize),
}

/// What the details panel shows.
#[derive(Clone)]
pub enum PanelContent {
    Key(Entity<KeyPanel>),
    Generate(Entity<GenerateForm>),
    Import(Entity<ImportForm>),
}

impl std::fmt::Debug for PanelContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Key(_) => "PanelContent::Key",
            Self::Generate(_) => "PanelContent::Generate",
            Self::Import(_) => "PanelContent::Import",
        })
    }
}

/// What a panel or form asks the screen to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeychainEvent {
    /// Close the panel (Cancel, the collapse button).
    Close,
    /// A key was created, or the key that was already there (`created`
    /// false) was found: show it.
    Saved { key_id: Uuid, created: bool },
    /// Remove was confirmed in the key panel (D50).
    Remove { key_id: Uuid },
}

pub struct KeychainScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    layout: KeychainLayout,
    sort: KeychainSort,
    /// Every key and identity, in the core's order.
    keys: Vec<KeyView>,
    identities: Vec<IdentityView>,
    /// Indices on display.
    shown_keys: Vec<usize>,
    shown_identities: Vec<usize>,
    /// The key whose panel is open.
    selected: Option<Uuid>,
    panel: Option<PanelContent>,
    /// The chosen width of the details panel (UI 1.3: 320-560 px).
    panel_width: Pixels,
    /// The width of the whole screen, measured after layout.
    measured_width: Option<f32>,
    columns: usize,
    card_width: f32,
    rows: Vec<Row>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    /// (columns, rows, width) the rows were built for.
    layout_key: (usize, usize, i32),
    scroll: VirtualListScrollHandle,
    focus: FocusHandle,
    /// [`AppState::records_generation`] of the last load started.
    loaded_generation: Option<u64>,
    stale: bool,
    loading: Option<Task<()>>,
    /// Loads started so far (tests).
    #[cfg(test)]
    reloads: usize,
    _subscriptions: Vec<Subscription>,
    panel_subscriptions: Vec<Subscription>,
    /// The duplicates-from-sync hint (UI 13.7).
    duplicates_hint: Entity<crate::sync_ui::duplicates::DuplicatesHint>,
}

impl std::fmt::Debug for KeychainScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeychainScreen")
            .field("keys", &self.keys.len())
            .field("identities", &self.identities.len())
            .finish_non_exhaustive()
    }
}

impl Focusable for KeychainScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl KeychainScreen {
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
                    if !this.state.read(cx).is_unlocked() {
                        // Locked: no key text, typed passphrase or list
                        // stays on screen or in memory. A label edit still
                        // waiting cannot be saved any more (the keys are
                        // gone); the auto-lock saved it first.
                        if let Some(PanelContent::Key(panel)) = this.panel.take() {
                            panel.update(cx, |panel, cx| panel.discard(cx));
                        }
                        this.panel = None;
                        this.panel_subscriptions.clear();
                        this.selected = None;
                        this.loading = None;
                        this.keys = Vec::new();
                        this.identities = Vec::new();
                        this.refilter(cx);
                    }
                    this.invalidate(cx);
                }
                // The windows save their pending edits before the auto-lock.
                AppStateEvent::AutoLockDue => {
                    if let Some(PanelContent::Key(panel)) = &this.panel {
                        panel.update(cx, |panel, cx| panel.flush(cx));
                    }
                }
                AppStateEvent::SelectionChanged | AppStateEvent::LocationChanged(_) => {}
            });
        let (layout, sort) = saved_view(state.read(cx));
        let duplicates_hint = cx.new(|cx| {
            crate::sync_ui::duplicates::DuplicatesHint::new(
                state.clone(),
                "keychain-duplicates",
                crate::sync_ui::duplicates::KEYCHAIN,
                view::CONTENT_PADDING,
                cx,
            )
        });
        Self {
            state,
            search,
            layout,
            sort,
            keys: Vec::new(),
            identities: Vec::new(),
            shown_keys: Vec::new(),
            shown_identities: Vec::new(),
            selected: None,
            panel: None,
            panel_width: metrics::at_100(metrics::WIDTH_DETAILS),
            measured_width: None,
            columns: 1,
            card_width: 0.,
            rows: Vec::new(),
            row_sizes: Rc::default(),
            layout_key: (0, usize::MAX, 0),
            scroll: VirtualListScrollHandle::new(),
            // The cards are one tab stop; the arrows move inside them.
            focus: cx.focus_handle().tab_stop(true),
            loaded_generation: None,
            stale: true,
            loading: None,
            #[cfg(test)]
            reloads: 0,
            duplicates_hint,
            _subscriptions: vec![search_subscription, state_subscription],
            panel_subscriptions: Vec::new(),
        }
    }

    // -- Reading -----------------------------------------------------------------

    pub fn keys(&self) -> &[KeyView] {
        &self.keys
    }

    pub fn identities(&self) -> &[IdentityView] {
        &self.identities
    }

    /// The key record ids on display, in display order.
    pub fn shown_key_ids(&self) -> Vec<Uuid> {
        self.shown_keys
            .iter()
            .map(|index| self.keys[*index].record_id)
            .collect()
    }

    /// The identity record ids on display, in display order.
    pub fn shown_identity_ids(&self) -> Vec<Uuid> {
        self.shown_identities
            .iter()
            .map(|index| self.identities[*index].record_id)
            .collect()
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    /// The key the arrows start from: the selected one, else the first on
    /// display. The keyboard focus ring marks it, also while nothing is
    /// selected (as on Hosts and Snippets; run 5, plan 11.3).
    pub fn cursor_key(&self) -> Option<Uuid> {
        let mut shown = self
            .shown_keys
            .iter()
            .map(|index| self.keys[*index].record_id);
        let first = shown.next();
        // A selected key the search hides: the arrows start at the first.
        match self.selected {
            Some(id) if first == Some(id) || shown.any(|shown| shown == id) => Some(id),
            _ => first,
        }
    }

    pub fn panel(&self) -> Option<&PanelContent> {
        self.panel.as_ref()
    }

    /// The key panel, while it is open.
    pub fn key_panel(&self) -> Option<&Entity<KeyPanel>> {
        match &self.panel {
            Some(PanelContent::Key(panel)) => Some(panel),
            _ => None,
        }
    }

    pub fn generate_form(&self) -> Option<&Entity<GenerateForm>> {
        match &self.panel {
            Some(PanelContent::Generate(form)) => Some(form),
            _ => None,
        }
    }

    pub fn import_form(&self) -> Option<&Entity<ImportForm>> {
        match &self.panel {
            Some(PanelContent::Import(form)) => Some(form),
            _ => None,
        }
    }

    pub fn has_panel(&self) -> bool {
        self.panel.is_some()
    }

    /// The chosen width of the details panel while it is open: the shell
    /// collapses its sidebar by it (UI 1.2), as for the host panel.
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.has_panel().then_some(self.panel_width)
    }

    pub fn layout(&self) -> KeychainLayout {
        self.layout
    }

    pub fn sort(&self) -> KeychainSort {
        self.sort
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    fn key(&self, id: Uuid) -> Option<&KeyView> {
        self.keys.iter().find(|view| view.record_id == id)
    }

    // -- Loading -------------------------------------------------------------------

    fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        cx.notify();
    }

    /// Reloads the keys and identities (now, or after the load in flight).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            self.invalidate(cx);
        } else {
            self.start_load(cx);
        }
    }

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
            self.keys = Vec::new();
            self.identities = Vec::new();
            self.refilter(cx);
            return;
        };
        let listed = cx.background_spawn(async move { (core.keys_list(), core.identities_list()) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let (keys, identities) = listed.await;
            let _ = this.update(cx, |screen, cx| {
                screen.loading = None;
                if !screen.state.read(cx).is_unlocked() {
                    return;
                }
                screen.keys = keys.unwrap_or_else(|error| {
                    tracing::warn!(code = error.code(), "keys could not be loaded");
                    Vec::new()
                });
                screen.identities = identities.unwrap_or_else(|error| {
                    tracing::warn!(code = error.code(), "identities could not be loaded");
                    Vec::new()
                });
                screen.loaded();
                screen.refilter(cx);
            });
        }));
    }

    /// After a load: the key panel follows its key (a new label elsewhere),
    /// and closes when the key is gone.
    fn loaded(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        if self.key(id).is_none() {
            self.selected = None;
            if matches!(self.panel, Some(PanelContent::Key(_))) {
                self.panel = None;
                self.panel_subscriptions.clear();
            }
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        self.shown_keys = key_listing(&self.keys, &text, self.sort);
        self.shown_identities = identity_listing(&self.identities, &text, self.sort);
        if let (Some(PanelContent::Key(panel)), Some(id)) = (&self.panel, self.selected)
            && let Some(view) = self.keys.iter().find(|view| view.record_id == id).cloned()
        {
            panel.update(cx, |panel, cx| panel.set_view(view, cx));
        }
        self.layout_key.1 = usize::MAX;
        cx.notify();
    }

    // -- Commands --------------------------------------------------------------------

    fn save_settings(&self, key: &str, value: &str, cx: &App) {
        if let Some(core) = self.state.read(cx).core() {
            let input = shellpp_core::service::dto::SettingsSetInput {
                values: [(key.to_owned(), serde_json::json!(value))]
                    .into_iter()
                    .collect(),
                reset: Vec::new(),
            };
            if let Err(error) = core.settings_set(input) {
                tracing::warn!(code = error.code(), "keychain view setting not saved");
            }
        }
    }

    pub fn set_layout(&mut self, layout: KeychainLayout, cx: &mut Context<Self>) {
        if self.layout != layout {
            self.layout = layout;
            self.layout_key.1 = usize::MAX;
            // Remembered across restarts, as on the Hosts screen (UI 2.2).
            self.save_settings(LAYOUT_KEY, layout.setting(), cx);
            cx.notify();
        }
    }

    pub fn set_sort(&mut self, sort: KeychainSort, cx: &mut Context<Self>) {
        if self.sort != sort {
            self.sort = sort;
            self.save_settings(SORT_KEY, sort.setting(), cx);
            self.refilter(cx);
        }
    }

    /// The window shows this screen or not: leaving it hides a shown
    /// private key (UI 7.3). The shell calls it on every frame.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if !shown && let Some(PanelContent::Key(panel)) = &self.panel {
            panel.update(cx, |panel, cx| panel.hide(cx));
        }
    }

    /// Replaces the panel: the previous one saves its pending label and
    /// drops what it held (a shown private key, typed text).
    fn set_panel(&mut self, panel: PanelContent, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_panel(cx);
        self.panel_subscriptions = match &panel {
            PanelContent::Key(panel) => vec![cx.subscribe_in(panel, window, Self::on_key_panel)],
            PanelContent::Generate(form) => {
                vec![cx.subscribe_in(form, window, Self::on_generate_form)]
            }
            PanelContent::Import(form) => vec![cx.subscribe_in(form, window, Self::on_import_form)],
        };
        self.panel = Some(panel);
        cx.notify();
    }

    fn drop_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(PanelContent::Key(panel)) = self.panel.take() {
            panel.update(cx, |panel, cx| {
                panel.flush(cx);
                panel.hide(cx);
            });
        }
        self.panel_subscriptions.clear();
    }

    /// Selects a key and opens its panel.
    pub fn open_key(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.key(id).cloned() else {
            return;
        };
        if self.selected == Some(id) && self.key_panel().is_some() {
            return;
        }
        self.selected = Some(id);
        let state = self.state.clone();
        let panel = cx.new(|cx| KeyPanel::new(state, view, window, cx));
        self.set_panel(PanelContent::Key(panel), window, cx);
    }

    /// Generate key (the arrow menu of New key, the empty state).
    pub fn open_generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = None;
        let state = self.state.clone();
        let form = cx.new(|cx| GenerateForm::new(state, window, cx));
        self.set_panel(PanelContent::Generate(form), window, cx);
    }

    /// New key: the import form (paste, file, drop).
    pub fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_import_with(None, window, cx);
    }

    /// Import from ~/.ssh (the empty state): the import form with the
    /// private keys found in the `~/.ssh` folder ([`default_ssh_folder`]).
    pub fn open_import_from_ssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folder = default_ssh_folder(cx);
        self.open_import_with(Some(folder), window, cx);
    }

    /// FIDO2: the import form for a key that `ssh-keygen -t ed25519-sk`
    /// made on a security key (its file holds a handle; D37).
    pub fn open_import_security_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_import_with(None, window, cx);
        if let Some(PanelContent::Import(form)) = &self.panel {
            form.update(cx, |form, cx| form.set_security_key(cx));
        }
    }

    fn open_import_with(
        &mut self,
        folder: Option<Option<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = None;
        let state = self.state.clone();
        let form = cx.new(|cx| ImportForm::new(state, folder, window, cx));
        self.set_panel(PanelContent::Import(form), window, cx);
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.drop_panel(cx);
        self.selected = None;
        cx.notify();
    }

    /// Sets the details panel width (clamped to 320-560 px).
    pub fn set_panel_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = crate::shell::details::clamp_width(width);
        if width != self.panel_width {
            self.panel_width = width;
            cx.notify();
        }
    }

    fn on_key_panel(
        &mut self,
        _: &Entity<KeyPanel>,
        event: &KeychainEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_panel_event(event, window, cx);
    }

    fn on_generate_form(
        &mut self,
        _: &Entity<GenerateForm>,
        event: &KeychainEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_panel_event(event, window, cx);
    }

    fn on_import_form(
        &mut self,
        _: &Entity<ImportForm>,
        event: &KeychainEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_panel_event(event, window, cx);
    }

    fn on_panel_event(
        &mut self,
        event: &KeychainEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            KeychainEvent::Close => self.close_panel(cx),
            KeychainEvent::Saved { key_id, created } => {
                // The new (or existing) key is shown at once: its panel needs
                // its view now, not after the background load (a few keys,
                // read from the unlocked index).
                self.keys = self.state.read(cx).keys();
                self.loaded_generation = Some(self.state.read(cx).records_generation());
                self.refilter(cx);
                self.open_key(*key_id, window, cx);
                if !created {
                    crate::shell::show_toast(strings::ALREADY_SAVED, window, cx);
                }
            }
            KeychainEvent::Remove { key_id } => self.remove_key(*key_id, window, cx),
        }
    }

    /// Removes a key once its Remove was confirmed (D50) and offers Undo for
    /// the undo window, as Remove of hosts does (UI 2.4). Its users keep a
    /// missing reference (data-model 3.4).
    pub fn remove_key(&mut self, key_id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let removed = self
            .state
            .update(cx, |state, cx| state.delete_records(vec![key_id], cx));
        if let Err(error) = removed {
            tracing::warn!(code = error.code(), "key not removed");
            crate::shell::show_toast(strings::REMOVE_FAILED, window, cx);
            return;
        }
        if self.selected == Some(key_id) {
            self.close_panel(cx);
        }
        let state = self.state.downgrade();
        crate::shell::show_toast(
            crate::shell::Toast::new(strings::KEY_REMOVED).action_with_id(
                "toast-undo",
                crate::i18n::notifications::UNDO,
                move |window, cx| {
                    let restored =
                        state.update(cx, |state, cx| state.restore_records(vec![key_id], cx));
                    if !matches!(restored, Ok(Ok(()))) {
                        crate::shell::show_toast(strings::UNDO_FAILED, window, cx);
                    }
                },
            ),
            window,
            cx,
        );
    }

    /// Arrows move the selection over the keys (the grid's columns for up
    /// and down); Esc closes the panel.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let ids = self.shown_key_ids();
        let current = self
            .selected
            .and_then(|id| ids.iter().position(|shown| *shown == id));
        let columns = self.row_columns() as isize;
        let step = match keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            "up" => -columns,
            "down" => columns,
            "escape" if self.panel.is_some() => {
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
        self.open_key(ids[next], window, cx);
        if let Some(row) = self.rows.iter().position(|row| match row {
            Row::Keys(start) => (*start..*start + self.row_columns()).contains(&next),
            _ => false,
        }) {
            self.scroll
                .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
        }
    }

    fn row_columns(&self) -> usize {
        match self.layout {
            KeychainLayout::Tiles => self.columns.max(1),
            KeychainLayout::List => 1,
        }
    }
}

impl EventEmitter<KeychainEvent> for KeyPanel {}
impl EventEmitter<KeychainEvent> for GenerateForm {}
impl EventEmitter<KeychainEvent> for ImportForm {}
