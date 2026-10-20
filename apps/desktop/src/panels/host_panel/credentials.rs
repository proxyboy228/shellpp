//! Credentials of the host panel's SSH block (UI 3.1, 7.3, 7.4): the linked
//! identity, key, certificate and FIDO2 key as removable rows, and the "+"
//! with its inline forms.
//!
//! - Identity: pick a saved identity, or create one inline (label, username,
//!   password), saved as an `identity` record and referenced by
//!   `ssh.identity_id`.
//! - Key: pick a saved key, or paste the text / choose a file; it is
//!   imported through `shellpp-core`'s key import (OpenSSH format). An
//!   encrypted key asks for its passphrase with "store in the vault" or "ask
//!   on each connection". The key record is referenced by `ssh.key_id`.
//! - Certificate: paste an OpenSSH user certificate; it is stored on the key
//!   linked to the host (own or inherited) and referenced by
//!   `ssh.certificate_key_id`.
//! - FIDO2 key: the "In development" placeholder.
//!
//! A type that is already linked disappears from the "+" list (UI 3.1). The
//! forms create their records on an explicit button (Create, Import, Attach):
//! a credential is its own object; the host itself still autosaves.

use std::path::Path;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, IntoElement, PathPromptOptions, SharedString,
    Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, IconName, Sizable as _, h_flex, v_flex,
};
use secrecy::SecretString;
use shellpp_core::CoreError;
use shellpp_core::ValidationReason;
use shellpp_core::service::dto::{KeyImportResult, KeyView, RecordRef};
use uuid::Uuid;

use super::{Dropdown, HostPanel, new_input};
use crate::panels::picker::{self, CredentialKind, PickValue, PickerRow};
use crate::panels::{PanelEvent, strings, widgets};
use crate::state::records_edit::{FieldErrors, IdentityView, PasswordChange};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, controls, metrics};

/// Largest key file read by "Choose a file" (a key is at most 32 KiB).
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

/// The state of the "+" forms and the names of linked records.
pub(super) struct Credentials {
    /// The open inline form.
    pub form: Option<CredentialKind>,
    /// The identity form shows the creation fields (not the saved list).
    pub identity_creating: bool,
    pub identities: Vec<IdentityView>,
    pub keys: Vec<KeyView>,
    /// Errors of the open form (`label`, `password`, `private_key`,
    /// `passphrase`, `certificate`).
    pub errors: FieldErrors,
    /// The chosen key file could not be read.
    pub file_error: bool,
    pub key_needs_passphrase: bool,
    pub key_store_passphrase: bool,
    pub key_importing: bool,
    pub identity_label: Entity<InputState>,
    pub identity_username: Entity<InputState>,
    pub identity_password: Entity<InputState>,
    pub key_text: Entity<TextareaState>,
    pub key_label: Entity<InputState>,
    pub key_passphrase: Entity<InputState>,
    pub certificate_text: Entity<TextareaState>,
    /// "N hosts use the same credentials" (UI 3.1 hint, stage 3), for a
    /// saved host without an identity.
    pub hint: Option<shellpp_core::service::merge::IdentityHint>,
}

impl Credentials {
    pub fn new(window: &mut Window, cx: &mut Context<HostPanel>) -> Self {
        let masked =
            |placeholder: &'static str, window: &mut Window, cx: &mut Context<HostPanel>| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(true)
                        .placeholder(placeholder)
                })
            };
        let textarea =
            |placeholder: &'static str, window: &mut Window, cx: &mut Context<HostPanel>| {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(3, 8)
                        .placeholder(placeholder)
                })
            };
        Self {
            hint: None,
            form: None,
            identity_creating: false,
            identities: Vec::new(),
            keys: Vec::new(),
            errors: FieldErrors::new(),
            file_error: false,
            key_needs_passphrase: false,
            key_store_passphrase: true,
            key_importing: false,
            identity_label: new_input(strings::IDENTITY_LABEL_PLACEHOLDER, "", window, cx),
            identity_username: new_input(strings::USERNAME, "", window, cx),
            identity_password: masked(strings::PASSWORD, window, cx),
            key_text: textarea(strings::KEY_TEXT_PLACEHOLDER, window, cx),
            key_label: new_input(strings::KEY_LABEL_PLACEHOLDER, "", window, cx),
            key_passphrase: masked(strings::PASSPHRASE, window, cx),
            certificate_text: textarea(strings::CERTIFICATE_PLACEHOLDER, window, cx),
        }
    }

    fn identity_name(&self, id: Uuid) -> String {
        self.identities
            .iter()
            .find(|identity| identity.record_id == id)
            .map(|identity| identity.label.clone())
            .unwrap_or_else(|| strings::MISSING_ITEM.to_owned())
    }

    fn key_name(&self, id: Uuid) -> String {
        self.keys
            .iter()
            .find(|key| key.record_id == id)
            .map(key_title)
            .unwrap_or_else(|| strings::MISSING_ITEM.to_owned())
    }
}

/// A key's title: its label, else its algorithm.
fn key_title(key: &KeyView) -> String {
    if key.label.trim().is_empty() {
        key.algorithm.clone()
    } else {
        key.label.clone()
    }
}

/// Reads a chosen key file (size-checked); `Err` when it cannot be used.
fn read_key_file(path: &Path) -> Result<String, ()> {
    let size = std::fs::metadata(path).map_err(drop)?.len();
    if size > MAX_KEY_FILE_BYTES {
        return Err(());
    }
    std::fs::read_to_string(path).map_err(drop)
}

impl HostPanel {
    // -- Reading ------------------------------------------------------------------

    /// The open "+" form, if any.
    pub fn credential_form(&self) -> Option<CredentialKind> {
        self.credentials.form
    }

    /// Errors of the open "+" form.
    pub fn credential_errors(&self) -> &FieldErrors {
        &self.credentials.errors
    }

    /// The key form asks for the passphrase of an encrypted key.
    pub fn key_needs_passphrase(&self) -> bool {
        self.credentials.key_needs_passphrase
    }

    /// Credential types the "+" still offers (a linked type disappears; a
    /// FIDO2 key is not offered before FIDO2 support, D5).
    pub fn available_credentials(&self) -> Vec<CredentialKind> {
        CredentialKind::OFFERED
            .into_iter()
            .filter(|kind| self.linked(*kind).is_none())
            .collect()
    }

    fn linked(&self, kind: CredentialKind) -> Option<Uuid> {
        match kind {
            CredentialKind::Identity => self.draft.identity_id,
            CredentialKind::Key => self.draft.key_id,
            CredentialKind::Certificate => self.draft.certificate_key_id,
            CredentialKind::Fido2 => self.draft.fido2_key_id,
        }
    }

    /// The key a certificate attaches to: the host's own key, else the
    /// inherited one.
    pub fn certificate_target(&self, cx: &App) -> Option<Uuid> {
        self.draft.key_id.or_else(|| {
            self.state
                .read(cx)
                .inherited_ssh(self.draft.group_id)
                .key_id
                .map(|key| key.value)
        })
    }

    // -- Commands ------------------------------------------------------------------

    /// Re-reads the identities and keys (names of linked rows, pickers).
    pub(super) fn refresh_keychain(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.credentials.identities = state.identities();
        self.credentials.keys = state.keys();
        self.credentials.hint = self.record_id.and_then(|id| {
            let vault_id = state.host(id)?.vault_id;
            state
                .core()?
                .identity_hint(RecordRef {
                    vault_id,
                    record_id: id,
                })
                .ok()
                .flatten()
        });
    }

    /// "Create an identity" of the hint (UI 3.1): saves a pending edit, lets
    /// the core create the identity and move every host of the hint onto it,
    /// then shows this host as the core left it (identity linked; username,
    /// password and key cleared), so a later autosave writes the same.
    pub fn accept_identity_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(record_id) = self.record_id else {
            return;
        };
        self.flush(cx);
        let state = self.state.read(cx);
        let (Some(core), Some(vault_id)) = (
            state.core().cloned(),
            state.host(record_id).map(|host| host.vault_id),
        ) else {
            return;
        };
        let hosts = self
            .credentials
            .hint
            .as_ref()
            .map_or(0, |hint| hint.hosts.len());
        match core.identity_from_hint(RecordRef {
            vault_id,
            record_id,
        }) {
            Ok(identity_id) => {
                self.draft.identity_id = Some(identity_id);
                self.draft.key_id = None;
                self.draft.username.clear();
                self.username
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.password
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.password_dirty = false;
                self.password_stored = false;
                self.autosave.cancel();
                self.credentials.hint = None;
                self.refresh_keychain(cx);
                self.refresh_placeholders(window, cx);
                cx.emit(PanelEvent::Toast(strings::identity_created(hosts).into()));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "identity not created from the hint");
                cx.emit(PanelEvent::Toast(strings::IDENTITY_HINT_FAILED.into()));
            }
        }
        cx.notify();
    }

    /// The hint above the credentials (UI 3.1), when there is one.
    pub(super) fn render_identity_hint(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hint = self.credentials.hint.as_ref()?;
        let others = hint.hosts.len().saturating_sub(1);
        let text = match hint.kind {
            shellpp_core::service::merge::IdentityHintKind::Password => {
                strings::identity_hint_password(others)
            }
            shellpp_core::service::merge::IdentityHintKind::Key => {
                strings::identity_hint_key(others)
            }
        };
        Some(
            v_flex()
                .id("host-identity-hint")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_1)
                .p(metrics::SPACE_2)
                .rounded(metrics::RADIUS_CONTROL)
                .bg(cx.theme().secondary)
                .child(widgets::caption(text, cx))
                .child(
                    widgets::text_button(
                        "host-identity-hint-create",
                        strings::IDENTITY_HINT_CREATE,
                        true,
                        cx,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.accept_identity_hint(window, cx),
                    )),
                )
                .into_any_element(),
        )
    }

    /// Opens the "+" form of `kind` (FIDO2, which the "+" does not offer
    /// yet, shows the placeholder).
    pub fn open_credential(
        &mut self,
        kind: CredentialKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dropdown = None;
        if kind == CredentialKind::Fido2 {
            self.credentials.form = None;
            cx.emit(PanelEvent::Toast(strings::IN_DEVELOPMENT.into()));
            cx.notify();
            return;
        }
        self.refresh_keychain(cx);
        self.reset_forms(window, cx);
        self.credentials.form = Some(kind);
        match kind {
            CredentialKind::Identity => {
                self.credentials.identity_creating = self.credentials.identities.is_empty();
                if self.credentials.identity_creating {
                    let input = self.credentials.identity_label.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
            }
            CredentialKind::Key => {
                let input = self.credentials.key_text.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            CredentialKind::Certificate => {
                let input = self.credentials.certificate_text.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
            }
            CredentialKind::Fido2 => {}
        }
        cx.notify();
    }

    /// Closes the open form and clears what was typed in it.
    pub fn cancel_credential(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_forms(window, cx);
        self.credentials.form = None;
        cx.notify();
    }

    fn reset_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let credentials = &mut self.credentials;
        credentials.errors.clear();
        credentials.file_error = false;
        credentials.identity_creating = false;
        credentials.key_needs_passphrase = false;
        credentials.key_store_passphrase = true;
        for input in [
            &credentials.identity_label,
            &credentials.identity_username,
            &credentials.identity_password,
            &credentials.key_label,
            &credentials.key_passphrase,
        ] {
            picker::clear_search(input, window, cx);
        }
        for text in [&credentials.key_text, &credentials.certificate_text] {
            text.update(cx, |text, cx| text.set_value("", window, cx));
        }
    }

    /// Unlinks a credential (the row's remove button).
    pub fn unlink_credential(&mut self, kind: CredentialKind, cx: &mut Context<Self>) {
        match kind {
            CredentialKind::Identity => self.draft.identity_id = None,
            CredentialKind::Key => self.draft.key_id = None,
            CredentialKind::Certificate => self.draft.certificate_key_id = None,
            CredentialKind::Fido2 => self.draft.fido2_key_id = None,
        }
        self.changed(cx);
    }

    pub(super) fn pick_identity(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.identity_id = Some(id);
        self.cancel_credential(window, cx);
        self.changed(cx);
    }

    pub(super) fn pick_key(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.key_id = Some(id);
        self.cancel_credential(window, cx);
        self.changed(cx);
    }

    pub(super) fn start_new_identity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.credentials.identity_creating = true;
        let input = self.credentials.identity_label.clone();
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Creates the identity typed in the form and links it.
    pub fn create_identity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let label = text(&self.credentials.identity_label);
        let username = text(&self.credentials.identity_username);
        let password = text(&self.credentials.identity_password);
        let password = if password.is_empty() {
            PasswordChange::Keep
        } else {
            PasswordChange::Set(SecretString::from(password))
        };
        let saved = self.state.update(cx, |state, cx| {
            state.save_identity(None, &label, &username, password, cx)
        });
        match saved {
            Ok(identity) => {
                self.refresh_keychain(cx);
                self.pick_identity(identity.record_id, window, cx);
            }
            Err(CoreError::Validation(fields)) => {
                self.credentials.errors = fields
                    .into_iter()
                    .map(|error| (error.field, error.reason))
                    .collect();
                cx.notify();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "identity not created");
                cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
            }
        }
    }

    /// Puts key text into the key form (a chosen file, or a paste).
    pub fn set_key_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_owned();
        self.credentials
            .key_text
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.credentials.errors.remove("private_key");
        self.credentials.file_error = false;
        cx.notify();
    }

    fn choose_key_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let read = cx
                .background_executor()
                .spawn(async move { read_key_file(&path) })
                .await;
            let _ = this.update_in(cx, |panel, window, cx| match read {
                Ok(text) => panel.set_key_text(&text, window, cx),
                Err(()) => {
                    tracing::info!("key file not read");
                    panel.credentials.file_error = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Imports the key of the form through the core and links it.
    pub fn import_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.credentials.key_importing {
            return;
        }
        let credentials = &self.credentials;
        let text = credentials.key_text.read(cx).value().to_string();
        let label = credentials.key_label.read(cx).value().trim().to_owned();
        let label = (!label.is_empty()).then_some(label);
        let passphrase = credentials
            .key_needs_passphrase
            .then(|| credentials.key_passphrase.read(cx).value().to_string())
            .filter(|value| !value.is_empty())
            .map(SecretString::from);
        let save = credentials.key_needs_passphrase && credentials.key_store_passphrase;
        self.credentials.errors.clear();
        self.credentials.key_importing = true;
        let task = self.state.update(cx, |state, cx| {
            state.import_key(SecretString::from(text), label, passphrase, save, cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |panel, window, cx| {
                panel.key_imported(result, window, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn key_imported(
        &mut self,
        result: Result<KeyImportResult, CoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.credentials.key_importing = false;
        match result {
            Ok(imported) => {
                if !imported.created {
                    cx.emit(PanelEvent::Toast(strings::KEY_ALREADY_SAVED.into()));
                }
                self.refresh_keychain(cx);
                self.pick_key(imported.key.record_id, window, cx);
            }
            Err(CoreError::Validation(fields)) => {
                let errors: FieldErrors = fields
                    .into_iter()
                    .map(|error| (error.field, error.reason))
                    .collect();
                let asks_passphrase = errors.get("passphrase") == Some(&ValidationReason::Required);
                if asks_passphrase && !self.credentials.key_needs_passphrase {
                    // An encrypted key: ask for its passphrase, no error yet.
                    self.credentials.key_needs_passphrase = true;
                    let input = self.credentials.key_passphrase.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                } else {
                    self.credentials.errors = errors;
                }
                cx.notify();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "key not imported");
                cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
                cx.notify();
            }
        }
    }

    /// Stores the pasted certificate on the target key and links it.
    pub fn attach_certificate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key_id) = self.certificate_target(cx) else {
            return;
        };
        let text = self
            .credentials
            .certificate_text
            .read(cx)
            .value()
            .to_string();
        if text.trim().is_empty() {
            self.credentials
                .errors
                .insert("certificate".into(), ValidationReason::Required);
            cx.notify();
            return;
        }
        let saved = self
            .state
            .update(cx, |state, cx| state.set_key_certificate(key_id, &text, cx));
        match saved {
            Ok(_) => {
                self.draft.certificate_key_id = Some(key_id);
                self.refresh_keychain(cx);
                self.cancel_credential(window, cx);
                self.changed(cx);
            }
            Err(CoreError::Validation(fields)) => {
                self.credentials.errors = fields
                    .into_iter()
                    .map(|error| (error.field, error.reason))
                    .collect();
                cx.notify();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "certificate not saved");
                cx.emit(PanelEvent::Toast(strings::NOT_SAVED.into()));
            }
        }
    }

    fn form_error(&self, field: &str) -> Option<&'static str> {
        self.credentials
            .errors
            .get(field)
            .map(|reason| strings::reason(field, *reason))
    }

    // -- Rendering -------------------------------------------------------------------

    /// Linked rows, inherited hints, the "+" and the open form.
    pub(super) fn render_credentials(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        for kind in CredentialKind::ALL {
            let Some(id) = self.linked(kind) else {
                continue;
            };
            let (icon, name): (gpui_kit::component::Icon, String) = match kind {
                CredentialKind::Identity => {
                    (IconName::User.into(), self.credentials.identity_name(id))
                }
                CredentialKind::Key => (Lucide::Key.into(), self.credentials.key_name(id)),
                CredentialKind::Certificate => (
                    IconName::FileText.into(),
                    strings::certificate_of(&self.credentials.key_name(id)),
                ),
                CredentialKind::Fido2 => (Lucide::Usb.into(), self.credentials.key_name(id)),
            };
            let remove = widgets::remove_button(
                SharedString::from(format!("host-cred-remove-{}", kind.as_str())),
                cx,
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _, cx| this.unlink_credential(kind, cx)),
            );
            rows.push(
                widgets::linked_row(
                    SharedString::from(format!("host-cred-{}", kind.as_str())),
                    icon,
                    kind.label(),
                    name,
                    remove,
                    cx,
                )
                .into_any_element(),
            );
        }
        let state = self.state.read(cx);
        let inherited = state.inherited_ssh(self.draft.group_id);
        for (label, from) in [
            (
                strings::CREDENTIAL_IDENTITY,
                inherited
                    .identity_id
                    .as_ref()
                    .filter(|_| self.draft.identity_id.is_none())
                    .map(|value| value.group_id),
            ),
            (
                strings::CREDENTIAL_KEY,
                inherited
                    .key_id
                    .as_ref()
                    .filter(|_| self.draft.key_id.is_none())
                    .map(|value| value.group_id),
            ),
        ] {
            if let Some(group_id) = from {
                let group = state
                    .group(group_id)
                    .map(|group| group.name.clone())
                    .unwrap_or_default();
                rows.push(
                    widgets::caption(
                        format!("{label} \u{b7} {}", strings::from_group(&group)),
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }

        let available = self.available_credentials();
        let open = self.dropdown == Some(Dropdown::Credential);
        let surface = cx.theme().secondary;
        let add = (!available.is_empty()).then(|| {
            Button::new("host-add-credential")
                .custom(controls::ghost(cx))
                .small()
                .rounded(metrics::RADIUS_CONTROL)
                .focus_ring(false)
                .focus_ring_visible(surface, cx)
                .icon(IconName::Plus)
                .tooltip(strings::ADD_CREDENTIAL)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_dropdown(Dropdown::Credential, window, cx)
                }))
        });
        let list = (open && !available.is_empty()).then(|| {
            let rows = available
                .iter()
                .map(|kind| {
                    PickerRow::new(kind.as_str(), PickValue::Credential(*kind), kind.label())
                })
                .collect();
            picker::render_list(
                "host-add-credential",
                None,
                rows,
                window,
                cx,
                self.on_pick(),
            )
        });
        let form = self.credentials.form.map(|kind| match kind {
            CredentialKind::Identity => self.render_identity_form(window, cx).into_any_element(),
            CredentialKind::Key => self.render_key_form(window, cx).into_any_element(),
            CredentialKind::Certificate => {
                self.render_certificate_form(window, cx).into_any_element()
            }
            CredentialKind::Fido2 => div().into_any_element(),
        });
        v_flex()
            .id("host-credentials")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_2)
            .children(rows)
            .child(
                v_flex()
                    .w_full()
                    .gap(metrics::SPACE_1)
                    .child(h_flex().w_full().justify_end().children(add))
                    .children(list)
                    .when(open, |this| {
                        this.on_mouse_down_out(cx.listener(Self::close_dropdown))
                    }),
            )
            .children(form)
    }

    fn form_buttons(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        on_submit: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let submit = widgets::form_button(id, label, enabled, cx).when(enabled, |this| {
            this.on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| on_submit(this, window, cx)),
            )
        });
        let cancel = widgets::text_button("host-credential-cancel", strings::CANCEL, true, cx)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.cancel_credential(window, cx)),
            );
        h_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .items_center()
            .justify_end()
            .child(cancel)
            .child(submit)
    }

    fn textarea(
        &self,
        state: &Entity<TextareaState>,
        id: &'static str,
        invalid: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let danger = cx.theme().danger;
        let mono = cx.theme().mono_font_family.clone();
        let textarea = Textarea::new(state)
            .px(metrics::SPACE_3 - gpui_kit::component::Size::Medium.input_px())
            .text_size(metrics::FONT_SIZE_DENSE)
            .font_family(mono)
            .bg(theme::tokens(cx).bg_input.hsla())
            .rounded(metrics::RADIUS_CONTROL)
            .when(invalid, |this| this.border_color(danger));
        div()
            .id(id)
            .test_support()
            .relative()
            .w_full()
            .child(theme::controls::muted_placeholder(state, cx))
            .child(textarea)
    }

    fn render_identity_form(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let credentials = &self.credentials;
        let body = if credentials.identity_creating || credentials.identities.is_empty() {
            let label_error = self.form_error("label");
            let password_error = self.form_error("password");
            v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(widgets::field(
                    strings::LABEL,
                    widgets::text_input(
                        &credentials.identity_label,
                        "host-identity-label",
                        label_error.is_some(),
                        window,
                        cx,
                    ),
                    label_error,
                    cx,
                ))
                .child(widgets::field(
                    strings::USERNAME,
                    widgets::text_input(
                        &credentials.identity_username,
                        "host-identity-username",
                        self.form_error("username").is_some(),
                        window,
                        cx,
                    ),
                    self.form_error("username"),
                    cx,
                ))
                .child(widgets::field(
                    strings::PASSWORD,
                    widgets::text_input_with(
                        Input::new(&credentials.identity_password),
                        &credentials.identity_password,
                        "host-identity-password",
                        password_error.is_some(),
                        window,
                        cx,
                    ),
                    password_error,
                    cx,
                ))
                .child(self.form_buttons(
                    "host-identity-create",
                    strings::CREATE,
                    true,
                    |this, window, cx| this.create_identity(window, cx),
                    cx,
                ))
                .into_any_element()
        } else {
            let mut rows: Vec<PickerRow> = credentials
                .identities
                .iter()
                .map(|identity| {
                    let row = PickerRow::new(
                        identity.record_id.to_string(),
                        PickValue::Identity(identity.record_id),
                        identity.label.clone(),
                    );
                    match &identity.username {
                        Some(username) => row.secondary(username.clone()),
                        None => row,
                    }
                })
                .collect();
            rows.push(
                PickerRow::new("new", PickValue::NewIdentity, strings::NEW_IDENTITY).leading(
                    gpui_kit::component::Icon::new(IconName::Plus)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                ),
            );
            v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(strings::CHOOSE_IDENTITY, cx))
                .child(picker::render_list(
                    "host-identity",
                    None,
                    rows,
                    window,
                    cx,
                    self.on_pick(),
                ))
                .child(
                    h_flex().w_full().justify_end().child(
                        widgets::text_button("host-credential-cancel", strings::CANCEL, true, cx)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_credential(window, cx)
                            })),
                    ),
                )
                .into_any_element()
        };
        widgets::inset("host-identity-form", cx).child(body)
    }

    fn render_key_form(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let credentials = &self.credentials;
        let saved = (!credentials.keys.is_empty()).then(|| {
            let rows = credentials
                .keys
                .iter()
                .map(|key| {
                    PickerRow::new(
                        key.record_id.to_string(),
                        PickValue::Key(key.record_id),
                        key_title(key),
                    )
                    .secondary(key.algorithm.clone())
                })
                .collect();
            v_flex()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(strings::SAVED_KEYS, cx))
                .child(picker::render_list(
                    "host-key",
                    None,
                    rows,
                    window,
                    cx,
                    self.on_pick(),
                ))
        });
        let text_error = self.form_error("private_key");
        let passphrase = credentials.key_needs_passphrase.then(|| {
            let error = self.form_error("passphrase");
            let choice = |id: &'static str, label: &'static str, store: bool| {
                widgets::segment(id, label, credentials.key_store_passphrase == store, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.credentials.key_store_passphrase = store;
                        cx.notify();
                    }))
                    .into_any_element()
            };
            v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(strings::KEY_PASSPHRASE_NOTE, cx))
                .child(widgets::field(
                    strings::PASSPHRASE,
                    widgets::text_input(
                        &credentials.key_passphrase,
                        "host-key-passphrase",
                        error.is_some(),
                        window,
                        cx,
                    ),
                    error,
                    cx,
                ))
                .child(widgets::segments([
                    choice("host-key-passphrase-store", strings::PASSPHRASE_STORE, true),
                    choice("host-key-passphrase-ask", strings::PASSPHRASE_ASK, false),
                ]))
        });
        let label_error = self.form_error("label");
        let choose = widgets::text_button("host-key-file", strings::CHOOSE_FILE, true, cx)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.choose_key_file(window, cx)),
            );
        let importing = credentials.key_importing;
        widgets::inset("host-key-form", cx)
            .children(saved)
            .child(widgets::caption(strings::ADD_A_KEY, cx))
            .child(self.textarea(
                &credentials.key_text,
                "host-key-text",
                text_error.is_some(),
                cx,
            ))
            .when_some(text_error, |this, error| {
                this.child(widgets::error_text(error, cx))
            })
            .when(credentials.file_error, |this| {
                this.child(widgets::error_text(strings::FILE_NOT_READ, cx))
            })
            .child(choose)
            .child(widgets::field(
                strings::LABEL,
                widgets::text_input(
                    &credentials.key_label,
                    "host-key-label",
                    label_error.is_some(),
                    window,
                    cx,
                ),
                label_error,
                cx,
            ))
            .children(passphrase)
            .child(self.form_buttons(
                "host-key-import",
                if importing {
                    strings::IMPORTING
                } else {
                    strings::IMPORT
                },
                !importing,
                |this, window, cx| this.import_key(window, cx),
                cx,
            ))
    }

    fn render_certificate_form(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let target = self.certificate_target(cx);
        let inset = widgets::inset("host-cert-form", cx);
        match target {
            None => inset
                .child(widgets::caption(strings::CERTIFICATE_NEEDS_KEY, cx))
                .child(
                    h_flex().w_full().justify_end().child(
                        widgets::text_button("host-credential-cancel", strings::CANCEL, true, cx)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.cancel_credential(window, cx)
                            })),
                    ),
                ),
            Some(key_id) => {
                let error = self.form_error("certificate");
                inset
                    .child(widgets::caption(
                        strings::certificate_for(&self.credentials.key_name(key_id)),
                        cx,
                    ))
                    .child(self.textarea(
                        &self.credentials.certificate_text,
                        "host-cert-text",
                        error.is_some(),
                        cx,
                    ))
                    .when_some(error, |this, error| {
                        this.child(widgets::error_text(error, cx))
                    })
                    .child(self.form_buttons(
                        "host-cert-attach",
                        strings::ATTACH,
                        true,
                        |this, window, cx| this.attach_certificate(window, cx),
                        cx,
                    ))
            }
        }
    }
}
