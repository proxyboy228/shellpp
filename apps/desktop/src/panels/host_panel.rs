//! The host panel (UI 3): view, create and edit a host in the details panel.
//!
//! Blocks from top to bottom, strictly in the order of UI 3.1:
//! 1. Vault (`host-block-vault`): a dropdown, only the personal vault for now.
//! 2. Address (`host-block-address`): OS icon tile + "IP or hostname"; focused
//!    when the panel opens for a new host and on the E key.
//! 3. General (`host-block-general`): Label, Parent Group (search, create on
//!    the fly), Tags (chips, autocomplete, create), Color (8 `--tag-*`,
//!    graphite by default), Backspace (Default, `^?`, `^H`), Notes.
//! 4. Share this host (`host-block-share`): an `--accent-text` text button,
//!    disabled until the host exists; opens the "In development" placeholder.
//! 5. SSH (`host-block-ssh`): port on the protocol line (placeholder 22),
//!    username, password (masked, show/hide), the linked identity, key,
//!    certificate and FIDO2 key as removable rows with "+" to link more
//!    ([`credentials`]), inherited values as "from group X", and "Show more"
//!    with one field per line: Agent Forwarding, Startup snippet, Host
//!    Chaining, Proxy, Environment Variables, Charset, Keepalive, Record
//!    history, Record output, Mosh (lead decision 2: shown, only the value is
//!    saved) and the terminal theme.
//! 6. Telnet (`host-block-telnet`): "Add Telnet" adds a second protocol block
//!    to the same host (port, placeholder 23, and username; removable). The
//!    "…" menu's Add Telnet does the same. Sessions come later (stage 9).
//! 7. Connect (`host-connect`), pinned to the bottom, disabled until the
//!    address is valid. It is the screen's one accent button unless the
//!    search bar holds a recognized address ([`HostPanel::set_search_has_address`]).
//!    It saves a pending edit first, then asks the shell for an SSH session
//!    tab of this host (`PanelRequest::Connect`; the tab connects with the
//!    saved settings, UI 4.8).
//!
//! Saving (UI 1.3, 3.2): no save button. A new host is a draft until its
//! address is valid; then it is created (and selected) and every later valid
//! change is saved after [`crate::theme::metrics::AUTOSAVE_DEBOUNCE`]. An
//! invalid value is outlined in `--danger` with the reason under it and is
//! not saved. Closing an empty draft discards it without asking. The rules
//! themselves live in [`crate::state::records_edit`].

mod credentials;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, IntoElement, MouseDownEvent,
    Render, SharedString, Subscription, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, IconName, Sizable as _, h_flex, v_flex,
};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use self::credentials::Credentials;
use super::autosave::Autosave;
use super::menus::{self, HOST_PANEL_MENU, HostPanelAction};
pub use super::picker::CredentialKind;
use super::picker::{self, OnPick, PickValue, PickerRow};
use super::{PanelEvent, PanelRequest, strings, widgets};
use crate::shell::details::DetailsHeader;
use crate::state::records_edit::{
    BackspaceMode, CHARSETS, EnvVar, FieldErrors, HostDraft, HostSecrets, OverrideMode, ProxyKind,
    SecretChange, TERMINAL_THEMES, TelnetDraft, check_address, has_tag, normalize_tag,
};
use crate::state::{AppState, AppStateEvent, SelectMode};
use crate::theme::controls::{ButtonColors as _, Filled};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::terminal::TerminalTheme;
use crate::theme::{self, metrics, palette};

/// Which inline dropdown of the panel is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dropdown {
    Vault,
    ParentGroup,
    JumpHost,
    /// The credential types of "+".
    Credential,
    StartupSnippet,
    Charset,
    TerminalTheme,
}

/// One environment variable row.
struct EnvRow {
    name: Entity<InputState>,
    value: Entity<InputState>,
}

pub struct HostPanel {
    state: Entity<AppState>,
    /// `None` while the new host is a draft.
    record_id: Option<Uuid>,
    draft: HostDraft,
    errors: FieldErrors,
    /// The stored host has a password (`ssh.password`).
    password_stored: bool,
    password_dirty: bool,
    proxy_password_dirty: bool,
    password_masked: bool,
    /// The search bar holds a recognized address: Connect here is neutral.
    search_has_address: bool,
    show_more: bool,
    dropdown: Option<Dropdown>,
    address: Entity<InputState>,
    label: Entity<InputState>,
    notes: Entity<TextareaState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    keepalive: Entity<InputState>,
    proxy_address: Entity<InputState>,
    proxy_port: Entity<InputState>,
    proxy_username: Entity<InputState>,
    proxy_password: Entity<InputState>,
    telnet_port: Entity<InputState>,
    telnet_username: Entity<InputState>,
    tag_input: Entity<InputState>,
    group_search: Entity<InputState>,
    jump_search: Entity<InputState>,
    env_rows: Vec<EnvRow>,
    /// The built-in terminal themes with their slugs, for the previews.
    terminal_themes: Vec<(String, TerminalTheme)>,
    /// The "+" forms and the names of linked identities and keys.
    credentials: Credentials,
    autosave: Autosave,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HostPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostPanel")
            .field("record_id", &self.record_id)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<PanelEvent> for HostPanel {}
impl EventEmitter<PanelRequest> for HostPanel {}

fn new_input(
    placeholder: impl Into<SharedString>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<HostPanel>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let value = value.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(value)
    })
}

impl HostPanel {
    /// A panel for an existing host, or an empty one (`None`) for a new host
    /// inside the group the screen shows, with the focus in Address.
    pub fn new(
        state: Entity<AppState>,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = record_id.and_then(|id| state.read(cx).host(id).cloned());
        let record_id = host.as_ref().map(|host| host.record_id);
        let draft = match &host {
            Some(host) => HostDraft::from_host(host),
            None => HostDraft {
                group_id: state.read(cx).location(),
                ..HostDraft::default()
            },
        };
        let password_stored = host
            .as_ref()
            .is_some_and(|host| host.secrets_set.iter().any(|path| path == "ssh.password"));
        let proxy_password_stored = host.as_ref().is_some_and(|host| {
            host.secrets_set
                .iter()
                .any(|path| path == "ssh.proxy.password")
        });

        let address = new_input(strings::ADDRESS_PLACEHOLDER, &draft.address, window, cx);
        let label = new_input(strings::LABEL_PLACEHOLDER, &draft.label, window, cx);
        let notes_value = draft.notes.clone();
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 10)
                .placeholder(strings::NOTES_PLACEHOLDER)
                .default_value(notes_value)
        });
        let port = new_input(strings::DEFAULT_PORT, &draft.port, window, cx);
        let username = new_input(strings::USERNAME, &draft.username, window, cx);
        let password_placeholder = if password_stored {
            strings::PASSWORD_SAVED
        } else {
            strings::PASSWORD
        };
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(password_placeholder)
        });
        let keepalive = new_input(strings::KEEPALIVE_PLACEHOLDER, &draft.keepalive, window, cx);
        let telnet = draft.telnet.clone().unwrap_or_default();
        let telnet_port = new_input(strings::TELNET_DEFAULT_PORT, &telnet.port, window, cx);
        let telnet_username = new_input(strings::USERNAME, &telnet.username, window, cx);
        let proxy_address = new_input(
            strings::ADDRESS_PLACEHOLDER,
            &draft.proxy_address,
            window,
            cx,
        );
        let proxy_port = new_input(strings::PROXY_PORT, &draft.proxy_port, window, cx);
        let proxy_username = new_input(strings::USERNAME, &draft.proxy_username, window, cx);
        let proxy_password_placeholder = if proxy_password_stored {
            strings::PASSWORD_SAVED
        } else {
            strings::PASSWORD
        };
        let proxy_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(proxy_password_placeholder)
        });
        let tag_input = new_input(strings::TAGS_PLACEHOLDER, "", window, cx);
        let group_search = new_input(strings::SEARCH_GROUPS, "", window, cx);
        let jump_search = new_input(strings::SEARCH_HOSTS, "", window, cx);

        let mut this = Self {
            state: state.clone(),
            record_id,
            draft,
            errors: FieldErrors::new(),
            password_stored,
            password_dirty: false,
            proxy_password_dirty: false,
            password_masked: true,
            search_has_address: false,
            show_more: false,
            dropdown: None,
            address,
            label,
            notes,
            port,
            username,
            password,
            keepalive,
            proxy_address,
            proxy_port,
            proxy_username,
            proxy_password,
            telnet_port,
            telnet_username,
            tag_input,
            group_search,
            jump_search,
            env_rows: Vec::new(),
            terminal_themes: crate::theme::terminal::registry()
                .iter()
                .map(|entry| (entry.id().to_owned(), entry.theme.clone()))
                .filter(|(slug, _)| TERMINAL_THEMES.contains(&slug.as_str()))
                .collect(),
            credentials: Credentials::new(window, cx),
            autosave: Autosave::default(),
            _subscriptions: Vec::new(),
        };
        let env = this.draft.env.clone();
        for var in env {
            this.push_env_row(&var, window, cx);
        }
        this.refresh_keychain(cx);

        let mut subscriptions = Vec::new();
        for input in [
            &this.address,
            &this.label,
            &this.port,
            &this.username,
            &this.keepalive,
            &this.proxy_address,
            &this.proxy_port,
            &this.proxy_username,
            &this.telnet_port,
            &this.telnet_username,
        ] {
            subscriptions.push(cx.subscribe_in(input, window, Self::on_field_event));
        }
        subscriptions.push(cx.subscribe_in(
            &this.notes,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.edited(cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &this.password,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.password_dirty = true;
                    this.edited(cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &this.proxy_password,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.proxy_password_dirty = true;
                    this.edited(cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &this.tag_input,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    let raw = this.tag_input.read(cx).value().to_string();
                    this.add_tag(&raw, window, cx);
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        ));
        for search in [&this.group_search, &this.jump_search] {
            subscriptions.push(cx.subscribe_in(
                search,
                window,
                |_: &mut Self, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &state,
            window,
            |this: &mut Self, _, event: &AppStateEvent, window, cx| {
                if let AppStateEvent::RecordsChanged = event {
                    this.on_records_changed(window, cx);
                }
            },
        ));
        this._subscriptions.extend(subscriptions);
        this.refresh_placeholders(window, cx);
        if this.record_id.is_none() {
            this.focus_address(window, cx);
        }
        this
    }

    // -- Reading ------------------------------------------------------------------

    /// The host shown, once it exists.
    pub fn record_id(&self) -> Option<Uuid> {
        self.record_id
    }

    /// The form as typed.
    pub fn draft(&self) -> &HostDraft {
        &self.draft
    }

    /// Refused members and their reasons.
    pub fn errors(&self) -> &FieldErrors {
        &self.errors
    }

    /// The panel title (UI 1.3).
    pub fn title(&self) -> &'static str {
        if self.record_id.is_some() {
            strings::HOST_DETAILS
        } else {
            strings::NEW_HOST
        }
    }

    /// Whether an edit is waiting for the autosave delay.
    pub fn has_pending_save(&self) -> bool {
        self.autosave.is_pending()
    }

    /// Whether Connect can be used: the address is valid.
    pub fn can_connect(&self) -> bool {
        matches!(check_address(&self.draft.address), Ok(Some(_)))
    }

    /// Whether Connect is the accent-filled button (lead decision 3).
    pub fn connect_is_accent(&self) -> bool {
        !self.search_has_address
    }

    pub fn is_showing_more(&self) -> bool {
        self.show_more
    }

    /// The placeholders of Port and Username: "22" / "Username", or the
    /// inherited value with "from group X".
    pub fn ssh_placeholders(&self, cx: &App) -> (SharedString, SharedString) {
        (
            self.port.read(cx).presentation().placeholder().clone(),
            self.username.read(cx).presentation().placeholder().clone(),
        )
    }

    // -- Commands from the shell ------------------------------------------------------

    /// Puts the focus in Address (New host, key E).
    pub fn focus_address(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.address.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Tells the panel whether the search bar holds a recognized address:
    /// then the search's Connect is the screen's accent button and this
    /// panel's Connect turns neutral.
    pub fn set_search_has_address(&mut self, has_address: bool, cx: &mut Context<Self>) {
        if self.search_has_address != has_address {
            self.search_has_address = has_address;
            cx.notify();
        }
    }

    /// Saves a pending edit now (the panel closes or another record opens).
    /// A draft without a valid address is discarded silently.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.autosave.is_pending() {
            self.save_now(cx);
        }
        self.autosave.cancel();
    }

    /// Reads the fields and saves by the saving rule.
    pub fn save_now(&mut self, cx: &mut Context<Self>) {
        self.autosave.take_pending();
        self.read_inputs(cx);
        let secrets = self.secrets(cx);
        let record_id = self.record_id;
        let draft = self.draft.clone();
        let report = self.state.update(cx, |state, cx| {
            state.save_host_draft(record_id, &draft, &secrets, cx)
        });
        if let Some(host) = report.saved {
            if self.record_id.is_none() {
                tracing::info!("host created from the panel");
            }
            self.record_id = Some(host.record_id);
            self.password_stored = host.secrets_set.iter().any(|path| path == "ssh.password");
            if !report.errors.keys().any(|path| path == "ssh.password") {
                self.password_dirty = false;
            }
            if !report
                .errors
                .keys()
                .any(|path| path.starts_with("ssh.proxy"))
            {
                self.proxy_password_dirty = false;
            }
        }
        self.errors = report.errors;
        cx.notify();
    }

    // -- Editing ------------------------------------------------------------------------

    fn on_field_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let InputEvent::Change = event {
            self.edited(cx);
        }
    }

    /// A field changed: take the values and save after the debounce.
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.read_inputs(cx);
        // Client-side checks show at once; the save follows the debounce.
        let mut errors = self.draft.validate();
        if self.record_id.is_none() && self.draft.address.trim().is_empty() {
            errors.remove("address");
        }
        self.errors.retain(|path, _| !is_client_checked(path));
        for (path, reason) in errors {
            self.errors.insert(path, reason);
        }
        self.autosave.schedule(cx, |this, cx| this.save_now(cx));
        cx.notify();
    }

    /// A control other than a text field changed (picker, chips, swatches).
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.autosave.schedule(cx, |this, cx| this.save_now(cx));
        cx.notify();
    }

    fn read_inputs(&mut self, cx: &App) {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        self.draft.address = text(&self.address);
        self.draft.label = text(&self.label);
        self.draft.notes = self.notes.read(cx).value().to_string();
        self.draft.port = text(&self.port);
        self.draft.username = text(&self.username);
        self.draft.keepalive = text(&self.keepalive);
        if let Some(telnet) = self.draft.telnet.as_mut() {
            telnet.port = text(&self.telnet_port);
            telnet.username = text(&self.telnet_username);
        }
        self.draft.proxy_address = text(&self.proxy_address);
        self.draft.proxy_port = text(&self.proxy_port);
        self.draft.proxy_username = text(&self.proxy_username);
        self.draft.env = self
            .env_rows
            .iter()
            .map(|row| EnvVar {
                name: row.name.read(cx).value().to_string(),
                value: row.value.read(cx).value().to_string(),
            })
            .collect();
    }

    fn secrets(&self, cx: &App) -> HostSecrets {
        let change = |dirty: bool, input: &Entity<InputState>| {
            if !dirty {
                return SecretChange::Keep;
            }
            let value = input.read(cx).value().to_string();
            if value.is_empty() {
                SecretChange::Clear
            } else {
                SecretChange::Set(SecretString::from(value))
            }
        };
        HostSecrets {
            password: change(self.password_dirty, &self.password),
            proxy_password: change(self.proxy_password_dirty, &self.proxy_password),
        }
    }

    fn on_records_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_keychain(cx);
        let Some(id) = self.record_id else {
            self.refresh_placeholders(window, cx);
            return;
        };
        let Some(host) = self.state.read(cx).host(id).cloned() else {
            // Removed elsewhere: nothing left to show.
            self.autosave.cancel();
            cx.emit(PanelEvent::Close);
            return;
        };
        let stored = HostDraft::from_host(&host);
        // The group of a stored host changes only through `move_records`
        // (Parent Group here, at once; drags and Move to elsewhere), never
        // through the autosave: always take it from the stored record, or a
        // pending autosave would move the host back.
        self.draft.group_id = stored.group_id;
        if !self.autosave.is_pending() {
            // Tag and color edits made elsewhere.
            self.draft.tags = stored.tags;
            self.draft.color = stored.color;
        }
        self.password_stored = host.secrets_set.iter().any(|path| path == "ssh.password");
        self.refresh_placeholders(window, cx);
        cx.notify();
    }

    /// Placeholders that show inherited values ("from group X").
    fn refresh_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let inherited = state.inherited_ssh(self.draft.group_id);
        let group_name = |id: Uuid| {
            state
                .group(id)
                .map(|group| group.name.clone())
                .unwrap_or_default()
        };
        let port = inherited
            .port
            .as_ref()
            .map(|port| strings::inherited(&port.value.to_string(), &group_name(port.group_id)))
            .unwrap_or_else(|| strings::DEFAULT_PORT.to_owned());
        let username = inherited
            .username
            .as_ref()
            .map(|user| strings::inherited(&user.value, &group_name(user.group_id)))
            .unwrap_or_else(|| strings::USERNAME.to_owned());
        self.port
            .update(cx, |input, cx| input.set_placeholder(port, window, cx));
        self.username
            .update(cx, |input, cx| input.set_placeholder(username, window, cx));
        let password = if self.password_stored {
            strings::PASSWORD_SAVED
        } else {
            strings::PASSWORD
        };
        self.password
            .update(cx, |input, cx| input.set_placeholder(password, window, cx));
    }

    fn toggle_dropdown(&mut self, dropdown: Dropdown, window: &mut Window, cx: &mut Context<Self>) {
        if self.dropdown == Some(dropdown) {
            self.dropdown = None;
        } else {
            self.dropdown = Some(dropdown);
            let search = match dropdown {
                Dropdown::ParentGroup => Some(self.group_search.clone()),
                Dropdown::JumpHost => Some(self.jump_search.clone()),
                Dropdown::Vault
                | Dropdown::Credential
                | Dropdown::StartupSnippet
                | Dropdown::Charset
                | Dropdown::TerminalTheme => None,
            };
            if let Some(search) = search {
                picker::clear_search(&search, window, cx);
                search.update(cx, |input, cx| input.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// Esc closes the open dropdown, then the panel (the shell saves a
    /// pending edit first).
    fn on_escape(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key != "escape" || keystroke.modifiers.modified() {
            return;
        }
        cx.stop_propagation();
        if self.dropdown.take().is_some() {
            cx.notify();
        } else {
            cx.emit(PanelEvent::Close);
        }
    }

    /// A click outside an open dropdown closes it.
    fn close_dropdown(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dropdown.take().is_some() {
            cx.notify();
        }
    }

    fn pick(&mut self, value: PickValue, window: &mut Window, cx: &mut Context<Self>) {
        match value {
            PickValue::PersonalVault => {}
            PickValue::TopLevel => self.set_group(None, cx),
            PickValue::Group(id) => self.set_group(Some(id), cx),
            PickValue::Create(name) if self.dropdown == Some(Dropdown::ParentGroup) => {
                let parent = self.state.read(cx).location();
                let created = self
                    .state
                    .update(cx, |state, cx| state.create_group_named(&name, parent, cx));
                match created {
                    Ok(group) => {
                        cx.emit(PanelEvent::Toast(strings::GROUP_CREATED.into()));
                        self.set_group(Some(group.record_id), cx);
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "group not created");
                        cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
                    }
                }
            }
            PickValue::Host(id) => {
                let mut chain = self.draft.jump_chain.clone().unwrap_or_default();
                if !chain.contains(&id) && chain.len() < crate::state::records_edit::MAX_JUMP_HOSTS
                {
                    chain.push(id);
                }
                self.draft.jump_chain = Some(chain);
                self.changed(cx);
            }
            PickValue::Tag(tag) | PickValue::Create(tag) => {
                self.add_tag(&tag, window, cx);
                return;
            }
            PickValue::Credential(kind) => {
                self.open_credential(kind, window, cx);
                return;
            }
            PickValue::Identity(id) => self.pick_identity(id, window, cx),
            PickValue::NewIdentity => {
                self.start_new_identity(window, cx);
                return;
            }
            PickValue::Key(id) => self.pick_key(id, window, cx),
            PickValue::Charset(label) => {
                // UTF-8 is the default: stored as no value (data-model 8.5).
                let label = if label == CHARSETS[0] {
                    String::new()
                } else {
                    label
                };
                if self.draft.charset.to_lowercase() != label {
                    self.draft.charset = label;
                    self.errors.remove("charset");
                    self.changed(cx);
                }
            }
            PickValue::TerminalTheme(slug) => {
                if self.draft.terminal_theme.as_deref() != Some(slug.as_str()) {
                    self.draft.terminal_theme = Some(slug);
                    self.changed(cx);
                }
            }
            PickValue::Snippet(id) => {
                if self.draft.startup_snippet_id != id {
                    self.draft.startup_snippet_id = id;
                    self.changed(cx);
                }
            }
        }
        self.dropdown = None;
        self.refresh_placeholders(window, cx);
        cx.notify();
    }

    /// Parent Group: a stored host moves there at once (appended, like Move
    /// to); a draft just remembers it.
    fn set_group(&mut self, group: Option<Uuid>, cx: &mut Context<Self>) {
        if self.draft.group_id == group {
            return;
        }
        let previous = std::mem::replace(&mut self.draft.group_id, group);
        let Some(id) = self.record_id else {
            self.changed(cx);
            return;
        };
        let moved = self
            .state
            .update(cx, |state, cx| state.move_records(vec![id], group, cx));
        if let Err(error) = moved {
            tracing::warn!(code = error.code(), "host not moved");
            // The host stays where it is; the autosave must not move it.
            self.draft.group_id = previous;
            cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
        }
        cx.notify();
    }

    fn add_tag(&mut self, raw: &str, window: &mut Window, cx: &mut Context<Self>) {
        if raw.trim().is_empty() {
            return;
        }
        match normalize_tag(raw) {
            Ok(tag) => {
                self.errors.remove("tags");
                if !has_tag(&self.draft.tags, &tag) {
                    self.draft.tags.push(tag);
                    self.changed(cx);
                }
                picker::clear_search(&self.tag_input, window, cx);
            }
            Err(reason) => {
                self.errors.insert("tags".into(), reason);
            }
        }
        cx.notify();
    }

    fn remove_tag(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.draft.tags.len() {
            self.draft.tags.remove(index);
            self.errors.retain(|path, _| !path.starts_with("tags"));
            self.changed(cx);
        }
    }

    fn push_env_row(&mut self, var: &EnvVar, window: &mut Window, cx: &mut Context<Self>) {
        let name = new_input(strings::ENV_NAME, &var.name, window, cx);
        let value = new_input(strings::ENV_VALUE, &var.value, window, cx);
        for input in [&name, &value] {
            let subscription = cx.subscribe_in(input, window, Self::on_field_event);
            self._subscriptions.push(subscription);
        }
        self.env_rows.push(EnvRow { name, value });
    }

    fn toggle_password_mask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reveal = self.password_masked;
        if reveal
            && self.password_stored
            && !self.password_dirty
            && self.password.read(cx).value().is_empty()
            && let Some(id) = self.record_id
        {
            match self.state.read(cx).reveal_secret(id, "ssh.password") {
                Ok(secret) => {
                    let value = secret.expose_secret().to_owned();
                    self.password
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                }
                Err(error) => tracing::warn!(code = error.code(), "password not revealed"),
            }
        }
        self.password_masked = !reveal;
        let masked = self.password_masked;
        self.password
            .update(cx, |input, cx| input.set_masked(masked, window, cx));
        cx.notify();
    }

    fn placeholder(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::Toast(strings::IN_DEVELOPMENT.into()));
    }

    /// Add Telnet (UI 3.1, 3.3): a Telnet block on the same host, port 23
    /// unless changed. The focus goes to its port.
    pub fn add_telnet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.telnet.is_some() {
            return;
        }
        for input in [&self.telnet_port, &self.telnet_username] {
            picker::clear_search(input, window, cx);
        }
        self.draft.telnet = Some(TelnetDraft::default());
        self.telnet_port
            .update(cx, |input, cx| input.focus(window, cx));
        self.changed(cx);
    }

    /// Removes the Telnet block (data-model 7.4: the member is removed).
    pub fn remove_telnet(&mut self, cx: &mut Context<Self>) {
        if self.draft.telnet.take().is_some() {
            self.errors.retain(|path, _| !path.starts_with("telnet"));
            self.changed(cx);
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if !self.can_connect() {
            return;
        }
        self.flush(cx);
        if let Some(id) = self.record_id {
            cx.emit(PanelRequest::Connect(id));
        }
    }

    /// Runs an item of the "…" menu (UI 3.3).
    pub fn run_menu_action(
        &mut self,
        action: HostPanelAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            HostPanelAction::Connect => self.connect(cx),
            HostPanelAction::AddTelnet => self.add_telnet(window, cx),
            HostPanelAction::Duplicate => {
                self.flush(cx);
                let Some(id) = self.record_id else {
                    return;
                };
                let result = self
                    .state
                    .update(cx, |state, cx| state.duplicate_record(id, cx));
                match result {
                    Ok(copy) => {
                        // The copy is selected and shown (UI 2.4 Duplicate).
                        let copy_id = copy.record_id();
                        self.state.update(cx, |state, cx| {
                            state.select(copy_id, SelectMode::Replace, &[], cx)
                        });
                        cx.emit(PanelEvent::Toast(strings::DUPLICATED.into()));
                        cx.emit(PanelRequest::OpenHost(copy_id));
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "host not duplicated");
                        cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
                    }
                }
            }
            HostPanelAction::Remove => {
                self.flush(cx);
                let Some(id) = self.record_id else {
                    return;
                };
                let panel = cx.entity().downgrade();
                menus::confirm_remove_hosts(
                    self.state.clone(),
                    vec![id],
                    window,
                    cx,
                    move |removed, _, cx| {
                        let _ = panel.update(cx, |panel, cx| {
                            panel.autosave.cancel();
                            cx.emit(PanelRequest::Removed(removed));
                            cx.emit(PanelEvent::Close);
                        });
                    },
                );
            }
        }
    }

    /// Whether a menu item can be used now.
    pub fn menu_action_enabled(&self, action: HostPanelAction) -> bool {
        match action {
            HostPanelAction::Connect => self.can_connect(),
            HostPanelAction::AddTelnet => self.draft.telnet.is_none(),
            HostPanelAction::Duplicate | HostPanelAction::Remove => self.record_id.is_some(),
        }
    }

    fn error(&self, prefix: &str) -> Option<&'static str> {
        self.errors
            .iter()
            .find(|(path, _)| {
                path.as_str() == prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('.'))
            })
            .map(|(path, reason)| strings::reason(path, *reason))
    }

    // -- Rendering ------------------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = cx.entity().downgrade();
        let enabled: Vec<bool> = HOST_PANEL_MENU
            .iter()
            .map(|action| self.menu_action_enabled(*action))
            .collect();
        let collapse_panel = panel.clone();
        DetailsHeader::new(self.title())
            .menu(move |mut menu, _, _| {
                for (action, enabled) in HOST_PANEL_MENU.iter().zip(enabled.iter()) {
                    let panel = panel.clone();
                    let action = *action;
                    menu = menu.item(menus::menu_item(
                        action.label(),
                        action.is_danger(),
                        !enabled,
                        move |window, cx| {
                            let _ = panel.update(cx, |panel, cx| {
                                panel.run_menu_action(action, window, cx);
                            });
                        },
                    ));
                }
                menu
            })
            .on_collapse(move |_, _, cx| {
                let _ = collapse_panel.update(cx, |panel, cx| {
                    panel.flush(cx);
                    cx.emit(PanelEvent::Close);
                });
            })
    }

    fn on_pick(&self) -> OnPick<Self> {
        Rc::new(
            |this: &mut Self, value: PickValue, window: &mut Window, cx: &mut Context<Self>| {
                this.pick(value, window, cx)
            },
        )
    }

    fn render_vault(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.dropdown == Some(Dropdown::Vault);
        let trigger = widgets::select_trigger(
            "host-vault",
            Some(crate::app::assets::vault_icon().small().into_any_element()),
            strings::PERSONAL_VAULT,
            open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::Vault, window, cx)
        }));
        let list = open.then(|| {
            picker::render_list(
                "host-vault",
                None,
                vec![
                    PickerRow::new(
                        "personal",
                        PickValue::PersonalVault,
                        strings::PERSONAL_VAULT,
                    )
                    .checked(true),
                ],
                window,
                cx,
                Rc::new(
                    |this: &mut Self, _: PickValue, _: &mut Window, cx: &mut Context<Self>| {
                        this.dropdown = None;
                        cx.notify();
                    },
                ),
            )
        });
        widgets::block("host-block-vault", None, cx).child(widgets::field(
            strings::VAULT,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list)
                .when(open, |this| {
                    this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                }),
            None,
            cx,
        ))
    }

    fn render_address(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let os_family = self
            .record_id
            .and_then(|id| self.state.read(cx).host(id))
            .and_then(|host| host.os_family.clone());
        let error = self.error("address");
        let tile = div().id("host-os-icon").test_support().child(widgets::tile(
            crate::hosts::layout::os_icon(os_family.as_deref()),
            self.draft.color,
            metrics::HEIGHT_CONTROL,
        ));
        widgets::block("host-block-address", None, cx).child(widgets::field(
            strings::ADDRESS,
            h_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .items_center()
                .child(tile)
                .child(div().flex_1().min_w_0().child(widgets::text_input(
                    &self.address,
                    "host-address",
                    error.is_some(),
                    window,
                    cx,
                ))),
            error,
            cx,
        ))
    }

    fn render_parent_group(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.dropdown == Some(Dropdown::ParentGroup);
        let state = self.state.read(cx);
        let current = self
            .draft
            .group_id
            .map(|id| state.group_path_label(id))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| strings::TOP_LEVEL.to_owned());
        let current_color = self
            .draft
            .group_id
            .and_then(|id| state.group(id))
            .map(|group| group.color)
            .unwrap_or_default();
        let trigger = widgets::select_trigger(
            "host-parent-group",
            Some(widgets::tile(Lucide::Folder, current_color, px(20.)).into_any_element()),
            current,
            open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::ParentGroup, window, cx)
        }));
        let list = open.then(|| {
            let query = self.group_search.read(cx).value().to_string();
            let state = self.state.read(cx);
            let mut rows = Vec::new();
            if picker::matches(&query, strings::TOP_LEVEL) {
                rows.push(
                    PickerRow::new("top", PickValue::TopLevel, strings::TOP_LEVEL)
                        .checked(self.draft.group_id.is_none()),
                );
            }
            let mut exact = false;
            for group in state.groups().iter() {
                let path = state.group_path_label(group.record_id);
                if !picker::matches(&query, &path) {
                    continue;
                }
                exact |= shellpp_core::model::text::fold(&group.name)
                    == shellpp_core::model::text::fold(query.trim());
                rows.push(
                    PickerRow::new(
                        group.record_id.to_string(),
                        PickValue::Group(group.record_id),
                        path,
                    )
                    .checked(self.draft.group_id == Some(group.record_id))
                    .leading(widgets::tile(
                        Lucide::Folder,
                        group.color,
                        px(20.),
                    )),
                );
            }
            let name = query.trim();
            if !name.is_empty() && !exact {
                rows.push(PickerRow::new(
                    "create",
                    PickValue::Create(name.to_owned()),
                    strings::create_group(name),
                ));
            }
            picker::render_list(
                "host-parent-group",
                Some((&self.group_search, "host-parent-group-search")),
                rows,
                window,
                cx,
                self.on_pick(),
            )
        });
        widgets::field(
            strings::PARENT_GROUP,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(trigger)
                .children(list)
                .when(open, |this| {
                    this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                }),
            self.error("group_id"),
            cx,
        )
    }

    fn render_tags(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chips: Vec<AnyElement> = self
            .draft
            .tags
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                let remove = widgets::remove_button(
                    SharedString::from(format!("host-tag-remove-{index}")),
                    cx,
                )
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.remove_tag(index, cx)),
                );
                widgets::chip(
                    SharedString::from(format!("host-tag-{index}")),
                    tag.clone(),
                    remove,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let query = self.tag_input.read(cx).value().trim().to_owned();
        let suggestions = (!query.is_empty()).then(|| {
            let all = self.state.read(cx).all_tags();
            let mut rows: Vec<PickerRow> = all
                .iter()
                .filter(|tag| picker::matches(&query, tag) && !has_tag(&self.draft.tags, tag))
                .take(8)
                .enumerate()
                .map(|(index, tag)| {
                    PickerRow::new(
                        format!("suggest-{index}"),
                        PickValue::Tag(tag.clone()),
                        tag.clone(),
                    )
                })
                .collect();
            if !has_tag(&all, &query) && !has_tag(&self.draft.tags, &query) {
                rows.push(PickerRow::new(
                    "create",
                    PickValue::Create(query.clone()),
                    strings::create_tag(&query),
                ));
            }
            picker::render_list("host-tag", None, rows, window, cx, self.on_pick())
        });
        let error = self.error("tags");
        widgets::field(
            strings::TAGS,
            v_flex()
                .id("host-tags")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .when(!chips.is_empty(), |this| {
                    this.child(h_flex().flex_wrap().gap(metrics::SPACE_1).children(chips))
                })
                .child(widgets::text_input(
                    &self.tag_input,
                    "host-tag-input",
                    error.is_some(),
                    window,
                    cx,
                ))
                .children(suggestions),
            error,
            cx,
        )
    }

    fn render_color(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let swatches: Vec<AnyElement> = palette::TAG_COLORS
            .iter()
            .map(|color| {
                let color = *color;
                widgets::swatch("host", color, self.draft.color == color, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.draft.color != color {
                            this.draft.color = color;
                            this.changed(cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        widgets::field(
            strings::COLOR,
            h_flex()
                .id("host-color")
                .test_support()
                .w_full()
                .flex_wrap()
                .gap(metrics::SPACE_2)
                .children(swatches),
            self.error("color"),
            cx,
        )
    }

    fn render_backspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let options = BackspaceMode::ALL.map(|mode| {
            let label = match mode {
                BackspaceMode::Default => strings::BACKSPACE_DEFAULT,
                BackspaceMode::Del => strings::BACKSPACE_DEL,
                BackspaceMode::CtrlH => strings::BACKSPACE_CTRL_H,
            };
            widgets::segment(
                SharedString::from(format!("host-backspace-{}", mode.as_str())),
                label,
                self.draft.backspace == mode,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.draft.backspace != mode {
                    this.draft.backspace = mode;
                    this.changed(cx);
                }
            }))
            .into_any_element()
        });
        widgets::field(
            strings::BACKSPACE,
            widgets::segments(options),
            self.error("backspace"),
            cx,
        )
    }

    fn render_notes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.error("notes");
        let danger = cx.theme().danger;
        // gpui-kit's Textarea cannot turn its own focus ring off, so it keeps
        // that one ring (no second ring is drawn around it).
        // The kit pads a multi-line editor inside its frame by the size's
        // input padding (medium: 10 px); the frame adds the rest, so the text
        // starts 12 px in like every other field of the panel.
        let notes = Textarea::new(&self.notes)
            .px(metrics::SPACE_3 - gpui_kit::component::Size::Medium.input_px())
            .text_size(metrics::FONT_SIZE_BODY)
            .bg(theme::tokens(cx).bg_input.hsla())
            .rounded(metrics::RADIUS_CONTROL)
            .when(error.is_some(), |this| this.border_color(danger));
        widgets::field(
            strings::NOTES,
            div()
                .id("host-notes")
                .test_support()
                .relative()
                .w_full()
                .child(theme::controls::muted_placeholder(&self.notes, cx))
                .child(notes),
            error,
            cx,
        )
    }

    fn render_general(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label_error = self.error("label");
        widgets::block("host-block-general", Some(strings::GENERAL), cx)
            // UI 3.1 General, in this order (lead decision 4).
            .child(widgets::field(
                strings::LABEL,
                widgets::text_input(&self.label, "host-label", label_error.is_some(), window, cx),
                label_error,
                cx,
            ))
            .child(self.render_parent_group(window, cx))
            .child(self.render_tags(window, cx))
            .child(self.render_color(cx))
            .child(self.render_backspace(cx))
            .child(self.render_notes(cx))
    }

    fn render_share(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.record_id.is_some();
        let button = widgets::text_button("host-share", strings::SHARE_HOST, enabled, cx)
            .when(enabled, |this| {
                this.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.placeholder(cx)))
            });
        widgets::block("host-block-share", None, cx).child(button)
    }

    fn render_password(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = widgets::icon_button(
            "host-password-toggle",
            if self.password_masked {
                IconName::Eye
            } else {
                IconName::EyeOff
            },
            cx,
        )
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_password_mask(window, cx)),
        );
        let error = self.error("ssh.password");
        widgets::field(
            strings::PASSWORD,
            widgets::text_input_with(
                Input::new(&self.password).suffix(toggle),
                &self.password,
                "host-password",
                error.is_some(),
                window,
                cx,
            ),
            error,
            cx,
        )
    }

    fn render_ssh(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let port_error = self.error("ssh.port");
        let username_error = self.error("ssh.username");
        let protocol_line = h_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .items_center()
            .child(
                div()
                    .flex_none()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(strings::SSH),
            )
            .child(widgets::caption(strings::PORT, cx))
            .child(div().flex_1().min_w(px(96.)).child(widgets::text_input(
                &self.port,
                "host-port",
                port_error.is_some(),
                window,
                cx,
            )));
        let more_label = if self.show_more {
            strings::SHOW_LESS
        } else {
            strings::SHOW_MORE
        };
        let show_more = widgets::text_button("host-show-more", more_label, true, cx).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_more = !this.show_more;
                cx.notify();
            }),
        );
        widgets::block("host-block-ssh", None, cx)
            .child(protocol_line)
            .when_some(port_error, |this, error| {
                this.child(widgets::error_text(error, cx))
            })
            .children(self.render_identity_hint(cx))
            .child(widgets::field(
                strings::USERNAME,
                widgets::text_input(
                    &self.username,
                    "host-username",
                    username_error.is_some(),
                    window,
                    cx,
                ),
                username_error,
                cx,
            ))
            .child(self.render_password(window, cx))
            .child(self.render_credentials(window, cx))
            .child(show_more)
            .when(self.show_more, |this| {
                this.child(self.render_more(window, cx))
            })
    }

    fn render_jump_hosts(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let inherited = state.inherited_ssh(self.draft.group_id);
        let title_of = |id: Uuid| {
            state
                .host(id)
                .map(|host| host.title().to_owned())
                .unwrap_or_else(|| id.to_string())
        };
        let chain = self.draft.jump_chain.clone().unwrap_or_default();
        let chips: Vec<AnyElement> = chain
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let remove = widgets::remove_button(
                    SharedString::from(format!("host-jump-remove-{index}")),
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(chain) = this.draft.jump_chain.as_mut()
                        && index < chain.len()
                    {
                        chain.remove(index);
                        if chain.is_empty() {
                            this.draft.jump_chain = None;
                        }
                        this.changed(cx);
                    }
                }));
                widgets::chip(
                    SharedString::from(format!("host-jump-{index}")),
                    title_of(*id),
                    remove,
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let inherited_text = inherited
            .jump_chain
            .as_ref()
            .filter(|_| self.draft.jump_chain.is_none())
            .map(|chain| {
                let group = state
                    .group(chain.group_id)
                    .map(|group| group.name.clone())
                    .unwrap_or_default();
                let hops = if chain.value.is_empty() {
                    strings::DIRECT.to_owned()
                } else {
                    chain
                        .value
                        .iter()
                        .map(|id| title_of(*id))
                        .collect::<Vec<_>>()
                        .join(" \u{2192} ")
                };
                strings::inherited(&hops, &group)
            });
        let open = self.dropdown == Some(Dropdown::JumpHost);
        let add = widgets::text_button(
            "host-jump-add",
            strings::ADD_JUMP_HOST,
            chain.len() < crate::state::records_edit::MAX_JUMP_HOSTS,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_dropdown(Dropdown::JumpHost, window, cx)
        }));
        let query = self.jump_search.read(cx).value().to_string();
        let rows: Option<Vec<PickerRow>> = open.then(|| {
            state
                .hosts()
                .iter()
                .filter(|host| {
                    Some(host.record_id) != self.record_id && !chain.contains(&host.record_id)
                })
                .filter(|host| {
                    picker::matches(&query, host.title()) || picker::matches(&query, &host.address)
                })
                .take(50)
                .map(|host| {
                    PickerRow::new(
                        host.record_id.to_string(),
                        PickValue::Host(host.record_id),
                        host.title().to_owned(),
                    )
                    .secondary(host.address.clone())
                    .leading(widgets::tile(
                        crate::hosts::layout::os_icon(host.os_family.as_deref()),
                        host.color,
                        px(20.),
                    ))
                })
                .collect()
        });
        let list = rows.map(|rows| {
            picker::render_list(
                "host-jump",
                Some((&self.jump_search, "host-jump-search")),
                rows,
                window,
                cx,
                self.on_pick(),
            )
        });
        widgets::field(
            strings::HOST_CHAINING,
            v_flex()
                .id("host-jump-hosts")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_1)
                .when(!chips.is_empty(), |this| {
                    this.child(h_flex().flex_wrap().gap(metrics::SPACE_1).children(chips))
                })
                .when_some(inherited_text, |this, text| {
                    this.child(widgets::caption(text, cx))
                })
                .child(add)
                .children(list)
                .when(open, |this| {
                    this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                }),
            self.error("ssh.jump_chain"),
            cx,
        )
    }

    fn render_proxy(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = ProxyKind::ALL.map(|kind| {
            let (key, label) = match kind {
                ProxyKind::None => ("none", strings::PROXY_NONE),
                ProxyKind::Http => ("http", strings::PROXY_HTTP),
                ProxyKind::Socks5 => ("socks5", strings::PROXY_SOCKS5),
            };
            widgets::segment(
                SharedString::from(format!("host-proxy-{key}")),
                label,
                self.draft.proxy_kind == kind,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if this.draft.proxy_kind != kind {
                    this.draft.proxy_kind = kind;
                    this.errors.retain(|path, _| !path.starts_with("ssh.proxy"));
                    this.changed(cx);
                }
            }))
            .into_any_element()
        });
        let address_error = self.error("ssh.proxy.address");
        let port_error = self.error("ssh.proxy.port");
        let details = (self.draft.proxy_kind != ProxyKind::None).then(|| {
            v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(
                    h_flex()
                        .w_full()
                        .gap(metrics::SPACE_2)
                        .child(div().flex_1().min_w_0().child(widgets::text_input(
                            &self.proxy_address,
                            "host-proxy-address",
                            address_error.is_some(),
                            window,
                            cx,
                        )))
                        .child(div().w(px(96.)).child(widgets::text_input(
                            &self.proxy_port,
                            "host-proxy-port",
                            port_error.is_some(),
                            window,
                            cx,
                        ))),
                )
                .children(
                    address_error
                        .or(port_error)
                        .map(|e| widgets::error_text(e, cx)),
                )
                .child(widgets::text_input(
                    &self.proxy_username,
                    "host-proxy-username",
                    false,
                    window,
                    cx,
                ))
                .child(widgets::text_input(
                    &self.proxy_password,
                    "host-proxy-password",
                    false,
                    window,
                    cx,
                ))
        });
        widgets::field(
            strings::PROXY,
            v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(widgets::segments(options))
                .children(details),
            None,
            cx,
        )
    }

    fn render_env(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .env_rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let name_error = self.errors.contains_key(&format!("ssh.env.{index}.name"));
                let remove = widgets::remove_button(
                    SharedString::from(format!("host-env-remove-{index}")),
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if index < this.env_rows.len() {
                        this.env_rows.remove(index);
                        this.errors.retain(|path, _| !path.starts_with("ssh.env"));
                        this.edited(cx);
                    }
                }));
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(div().w(px(120.)).child(widgets::text_input(
                        &row.name,
                        SharedString::from(format!("host-env-name-{index}")),
                        name_error,
                        window,
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().child(widgets::text_input(
                        &row.value,
                        SharedString::from(format!("host-env-value-{index}")),
                        false,
                        window,
                        cx,
                    )))
                    .child(remove)
                    .into_any_element()
            })
            .collect();
        let add = widgets::text_button("host-env-add", strings::ADD_VARIABLE, true, cx).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.push_env_row(&EnvVar::default(), window, cx);
                if let Some(row) = this.env_rows.last() {
                    row.name.update(cx, |input, cx| input.focus(window, cx));
                }
                cx.notify();
            }),
        );
        widgets::field(
            strings::ENVIRONMENT,
            v_flex()
                .id("host-env")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .children(rows)
                .child(add),
            self.error("ssh.env"),
            cx,
        )
    }

    fn render_override(
        &self,
        prefix: &'static str,
        label: &'static str,
        current: OverrideMode,
        set: fn(&mut HostDraft, OverrideMode),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let options = OverrideMode::ALL.map(|mode| {
            let text = match mode {
                OverrideMode::Default => strings::AS_IN_SETTINGS,
                OverrideMode::On => strings::ON,
                OverrideMode::Off => strings::OFF,
            };
            widgets::segment(
                SharedString::from(format!("{prefix}-{}", mode.as_str())),
                text,
                current == mode,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                set(&mut this.draft, mode);
                this.changed(cx);
            }))
            .into_any_element()
        });
        widgets::field(label, widgets::segments(options), None, cx)
    }

    /// A small preview of a terminal theme: its background, foreground,
    /// cursor and two ANSI colors.
    fn theme_preview(theme: &TerminalTheme, cx: &App) -> gpui::Div {
        let bar = |token: theme::palette::Token, width: f32| {
            div()
                .w(px(width))
                .h(px(3.))
                .rounded(px(1.))
                .bg(token.hsla())
        };
        div()
            .flex_none()
            .w(px(40.))
            .h(px(24.))
            .rounded(metrics::RADIUS_CONTROL)
            .border_1()
            .border_color(cx.theme().input)
            .bg(theme.background.hsla())
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(bar(theme.foreground, 22.))
            .child(
                h_flex()
                    .gap(px(2.))
                    .child(bar(theme.ansi.0[2], 8.))
                    .child(bar(theme.ansi.0[4], 12.)),
            )
            .child(bar(theme.cursor, 6.))
    }

    /// A select of "Show more": the trigger and, while open, its list.
    fn select_field(
        &self,
        dropdown: Dropdown,
        id: &'static str,
        leading: Option<AnyElement>,
        value: SharedString,
        list: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let open = self.dropdown == Some(dropdown);
        let trigger = widgets::select_trigger(id, leading, value, open, cx).on_click(cx.listener(
            move |this, _: &ClickEvent, window, cx| this.toggle_dropdown(dropdown, window, cx),
        ));
        v_flex()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(trigger)
            .when(open, |this| {
                this.children(list)
                    .on_mouse_down_out(cx.listener(Self::close_dropdown))
            })
    }

    fn render_startup_snippet(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value: SharedString = match self.draft.startup_snippet_id {
            None => strings::NONE.into(),
            // Snippets arrive with their screen; a stored reference is kept.
            Some(_) => strings::MISSING_ITEM.into(),
        };
        let list = (self.dropdown == Some(Dropdown::StartupSnippet)).then(|| {
            let mut rows = vec![
                PickerRow::new("none", PickValue::Snippet(None), strings::NONE)
                    .checked(self.draft.startup_snippet_id.is_none()),
            ];
            if let Some(id) = self.draft.startup_snippet_id {
                rows.push(
                    PickerRow::new(
                        "current",
                        PickValue::Snippet(Some(id)),
                        strings::MISSING_ITEM,
                    )
                    .checked(true),
                );
            }
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(picker::render_list(
                    "host-startup-snippet",
                    None,
                    rows,
                    window,
                    cx,
                    self.on_pick(),
                ))
                .child(widgets::caption(strings::NO_SNIPPETS, cx))
                .into_any_element()
        });
        let error = self.error("ssh.startup_snippet_id");
        widgets::field(
            strings::STARTUP_SNIPPET,
            self.select_field(
                Dropdown::StartupSnippet,
                "host-startup-snippet",
                None,
                value,
                list,
                cx,
            ),
            error,
            cx,
        )
    }

    fn render_charset(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.draft.charset.trim().to_lowercase();
        let value: SharedString = if current.is_empty() || current == CHARSETS[0] {
            strings::CHARSET_DEFAULT.into()
        } else {
            current.to_uppercase().into()
        };
        let list = (self.dropdown == Some(Dropdown::Charset)).then(|| {
            let mut labels: Vec<String> =
                CHARSETS.iter().map(|label| (*label).to_owned()).collect();
            if !current.is_empty() && !labels.contains(&current) {
                // A label set elsewhere (an import) stays selectable.
                labels.push(current.clone());
            }
            let rows = labels
                .into_iter()
                .map(|label| {
                    let selected = if current.is_empty() {
                        label == CHARSETS[0]
                    } else {
                        label == current
                    };
                    let title = if label == CHARSETS[0] {
                        strings::CHARSET_DEFAULT.to_owned()
                    } else {
                        label.to_uppercase()
                    };
                    PickerRow::new(label.clone(), PickValue::Charset(label), title)
                        .checked(selected)
                })
                .collect();
            picker::render_list("host-charset", None, rows, window, cx, self.on_pick())
                .into_any_element()
        });
        let error = self.error("charset");
        widgets::field(
            strings::CHARSET,
            self.select_field(Dropdown::Charset, "host-charset", None, value, list, cx),
            error,
            cx,
        )
    }

    fn render_terminal_theme(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Graphite is the default (data-model 8.5).
        let current = self
            .draft
            .terminal_theme
            .clone()
            .unwrap_or_else(|| TERMINAL_THEMES[0].to_owned());
        let chosen = self
            .terminal_themes
            .iter()
            .find(|(slug, _)| *slug == current)
            .or_else(|| self.terminal_themes.first());
        let value: SharedString = chosen
            .map(|(_, theme)| SharedString::from(theme.name.clone()))
            .unwrap_or_else(|| strings::DEFAULT_THEME_NAME.into());
        let leading = chosen.map(|(_, theme)| Self::theme_preview(theme, cx).into_any_element());
        let list = (self.dropdown == Some(Dropdown::TerminalTheme)).then(|| {
            let rows = self
                .terminal_themes
                .iter()
                .map(|(slug, theme)| {
                    PickerRow::new(
                        slug.clone(),
                        PickValue::TerminalTheme(slug.clone()),
                        theme.name.clone(),
                    )
                    .checked(*slug == current)
                    .leading(Self::theme_preview(theme, cx))
                })
                .collect();
            picker::render_list(
                "host-terminal-theme",
                None,
                rows,
                window,
                cx,
                self.on_pick(),
            )
            .into_any_element()
        });
        let error = self.error("terminal_theme");
        widgets::field(
            strings::TERMINAL_THEME,
            self.select_field(
                Dropdown::TerminalTheme,
                "host-terminal-theme",
                leading,
                value,
                list,
                cx,
            ),
            error,
            cx,
        )
    }

    fn render_more(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let agent = Switch::new("host-agent-forwarding")
            .checked(self.draft.agent_forwarding)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.agent_forwarding = *checked;
                this.changed(cx);
            }));
        // Agent Forwarding (stage 5, D60): the warning and the two options
        // show while it is on.
        let agent_options = self.draft.agent_forwarding.then(|| {
            let theme = cx.theme();
            v_flex()
                .id("host-agent-options")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(
                    div()
                        .id("host-agent-warning")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.warning)
                        .child(strings::AGENT_FORWARDING_WARNING),
                )
                .child(
                    Checkbox::new("host-agent-system")
                        .checked(self.draft.agent_forward_system)
                        .label(strings::AGENT_FORWARD_SYSTEM)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.agent_forward_system = *checked;
                            this.changed(cx);
                        })),
                )
                .child(
                    Checkbox::new("host-agent-confirm")
                        .checked(self.draft.agent_confirm)
                        .label(strings::AGENT_CONFIRM)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.draft.agent_confirm = *checked;
                            this.changed(cx);
                        })),
                )
                .when(self.draft.agent_confirm, |this| {
                    this.child(widgets::caption(strings::AGENT_CONFIRM_NOTE, cx))
                })
        });
        let mosh = Switch::new("host-mosh")
            .checked(self.draft.mosh)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.mosh = *checked;
                this.changed(cx);
            }));
        let keepalive_error = self.error("ssh.keepalive_s");
        v_flex()
            .id("host-more")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::divider(cx))
            // UI 3.1 "Show more", one field per line, in this order.
            .child(widgets::inline_row(strings::AGENT_FORWARDING, agent, cx))
            .children(agent_options)
            .child(self.render_startup_snippet(window, cx))
            .child(self.render_jump_hosts(window, cx))
            .child(self.render_proxy(window, cx))
            .child(self.render_env(window, cx))
            .child(self.render_charset(window, cx))
            .child(widgets::field(
                strings::KEEPALIVE,
                widgets::text_input(
                    &self.keepalive,
                    "host-keepalive",
                    keepalive_error.is_some(),
                    window,
                    cx,
                ),
                keepalive_error,
                cx,
            ))
            .child(self.render_override(
                "host-history",
                strings::RECORD_HISTORY,
                self.draft.record_history,
                |draft, mode| draft.record_history = mode,
                cx,
            ))
            .child(self.render_override(
                "host-output",
                strings::RECORD_OUTPUT,
                self.draft.record_output,
                |draft, mode| draft.record_output = mode,
                cx,
            ))
            .child(widgets::inline_row(strings::MOSH, mosh, cx))
            .child(self.render_terminal_theme(window, cx))
    }

    fn render_telnet(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let block = widgets::block("host-block-telnet", None, cx);
        if self.draft.telnet.is_none() {
            return block.child(
                widgets::outline_button(
                    "host-add-telnet",
                    Some(IconName::Plus),
                    strings::ADD_TELNET,
                    cx,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.add_telnet(window, cx)),
                ),
            );
        }
        let port_error = self.error("telnet.port");
        let username_error = self.error("telnet.username");
        let remove = widgets::icon_button("host-telnet-remove", IconName::Close, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.remove_telnet(cx)));
        let protocol_line = h_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .items_center()
            .child(
                div()
                    .flex_none()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(strings::TELNET),
            )
            .child(widgets::caption(strings::PORT, cx))
            .child(div().flex_1().min_w(px(72.)).child(widgets::text_input(
                &self.telnet_port,
                "host-telnet-port",
                port_error.is_some(),
                window,
                cx,
            )))
            .child(remove);
        block
            .child(protocol_line)
            .when_some(port_error, |this, error| {
                this.child(widgets::error_text(error, cx))
            })
            .child(widgets::field(
                strings::USERNAME,
                widgets::text_input(
                    &self.telnet_username,
                    "host-telnet-username",
                    username_error.is_some(),
                    window,
                    cx,
                ),
                username_error,
                cx,
            ))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.can_connect();
        // The screen's one accent button, unless the search's Connect is
        // (lead decision 3); disabled: the whole button at 0.45, no hover.
        let accent = self.connect_is_accent();
        let surface = cx.theme().colors.list;
        let button = Button::new("host-connect")
            .map(|button| {
                if accent {
                    button.accent_colors()
                } else {
                    button.neutral_colors()
                }
            })
            // Large: the label at body size (`text_base` = 14 px with our
            // rem; the kit's default is `text_sm` = 12.25 px).
            .large()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .rounded(metrics::RADIUS_CONTROL)
            .label(strings::CONNECT)
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .disabled_as_designed(
                !enabled,
                if accent {
                    Filled::Accent
                } else {
                    Filled::Neutral
                },
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.connect(cx)));
        div()
            .flex_none()
            .w_full()
            .p(metrics::SPACE_4)
            .border_t_1()
            .border_color(cx.theme().border)
            .child(button)
    }
}

/// Members the panel checks itself (their errors are recomputed on edit).
fn is_client_checked(path: &str) -> bool {
    path == "address"
        || path == "charset"
        || path.starts_with("telnet")
        || path == "ssh.port"
        || path == "ssh.keepalive_s"
        || path == "tags"
        || path.starts_with("ssh.env")
        || path.starts_with("ssh.proxy")
}

impl Render for HostPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window: &Window = window;
        v_flex()
            .id("host-panel")
            .test_support()
            .on_key_down(cx.listener(Self::on_escape))
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(self.render_header(cx))
            .child(
                v_flex()
                    .id("host-panel-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(metrics::SPACE_4)
                    .gap(metrics::SPACE_3)
                    // UI 3.1, blocks from top to bottom.
                    .child(self.render_vault(window, cx))
                    .child(self.render_address(window, cx))
                    .child(self.render_general(window, cx))
                    .child(self.render_share(cx))
                    .child(self.render_ssh(window, cx))
                    .child(self.render_telnet(window, cx)),
            )
            .child(self.render_footer(cx))
    }
}
