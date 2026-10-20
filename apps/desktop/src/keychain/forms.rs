//! The two forms that add a key, in the Keychain's details panel: Generate
//! key (UI 7.1, D32) and New key, the import (7.1, 7.3). Both commit with an
//! explicit button (D33): creating a key runs a key generation or a key
//! derivation and settles the passphrase choice, which typing must not do
//! by accident. Afterwards the new key's panel opens and its label saves as
//! in UI 1.3.
//!
//! - Generate key: Ed25519 (the default), ECDSA with P-256, P-384 or P-521,
//!   RSA with 3072 (the default) or 4096 bits; the label (required); an
//!   optional passphrase with "Save in the vault" or "Ask on every
//!   connection". The core generates on a background thread and the form
//!   shows its progress meanwhile (RSA takes seconds).
//! - New key: paste the text, choose a file or drop one on the form. The
//!   core's `inspect` runs on a background thread after each change and tells
//!   the format and whether a passphrase is needed, or the type "unknown"
//!   with the reason. An encrypted key asks for its passphrase with the same
//!   save / ask choice. A key that is already in the Keychain (same SHA256
//!   fingerprint) is not created again: the form opens the existing one.
//!   Opened from "Import from ~/.ssh", the form lists the private key files
//!   of that folder (an injected path in tests, never the real `~/.ssh`).
//!
//! Typed passphrases and key text are cleared from the inputs after the
//! import; the background copies are wiped (`Zeroizing`). Nothing is logged
//! but error codes.

use std::path::PathBuf;

use gpui::{AppContext as _, Context, Entity, Subscription, Task, Window};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use secrecy::SecretString;
use shellpp_core::keys::convert::KeyProblem;
use shellpp_core::service::dto::KeyImportResult;
use shellpp_core::{CoreError, ValidationReason};
use zeroize::Zeroizing;

use super::panel::check_label;
use super::{FileProblem, KeychainEvent, SshScan, read_key_file, scan_ssh_folder, strings};
use crate::state::AppState;
use crate::state::records_edit::{FieldErrors, KeyKind};

fn errors_of(fields: Vec<shellpp_core::FieldError>) -> FieldErrors {
    fields
        .into_iter()
        .map(|error| (error.field, error.reason))
        .collect()
}

/// A passphrase input: masked, with no "show" toggle of its own.
fn masked_input<T>(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(placeholder)
    })
}

// -- Generate key -------------------------------------------------------------------

/// The key type rows of the form: the family, then its curve or size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyFamily {
    #[default]
    Ed25519,
    Ecdsa,
    Rsa,
}

impl KeyFamily {
    pub const ALL: [Self; 3] = [Self::Ed25519, Self::Ecdsa, Self::Rsa];
}

/// Generate key (UI 7.1).
pub struct GenerateForm {
    state: Entity<AppState>,
    family: KeyFamily,
    /// The chosen curve (ECDSA) and size (RSA), kept while the family
    /// changes.
    curve: KeyKind,
    rsa: KeyKind,
    pub(super) label: Entity<InputState>,
    pub(super) passphrase: Entity<InputState>,
    save_passphrase: bool,
    errors: FieldErrors,
    generating: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for GenerateForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenerateForm")
            .field("kind", &self.kind())
            .field("generating", &self.generating.is_some())
            .finish_non_exhaustive()
    }
}

impl GenerateForm {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let label =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::LABEL_PLACEHOLDER));
        let passphrase = masked_input(strings::PASSPHRASE_OPTIONAL, window, cx);
        let subscriptions = vec![
            cx.subscribe(&label, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.errors.remove("label");
                }
                cx.notify();
            }),
            cx.subscribe(&passphrase, |this: &mut Self, _, _: &InputEvent, cx| {
                this.errors.remove("passphrase");
                cx.notify();
            }),
        ];
        label.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            family: KeyFamily::default(),
            curve: KeyKind::EcdsaP256,
            rsa: KeyKind::Rsa3072,
            label,
            passphrase,
            save_passphrase: true,
            errors: FieldErrors::new(),
            generating: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn family(&self) -> KeyFamily {
        self.family
    }

    /// The key type that Generate makes.
    pub fn kind(&self) -> KeyKind {
        match self.family {
            KeyFamily::Ed25519 => KeyKind::Ed25519,
            KeyFamily::Ecdsa => self.curve,
            KeyFamily::Rsa => self.rsa,
        }
    }

    pub fn is_generating(&self) -> bool {
        self.generating.is_some()
    }

    pub fn save_passphrase(&self) -> bool {
        self.save_passphrase
    }

    pub fn errors(&self) -> &FieldErrors {
        &self.errors
    }

    /// Whether a passphrase is typed (the save / ask choice shows then).
    pub fn has_passphrase(&self, cx: &gpui::App) -> bool {
        !self.passphrase.read(cx).value().is_empty()
    }

    pub fn set_family(&mut self, family: KeyFamily, cx: &mut Context<Self>) {
        if !self.is_generating() {
            self.family = family;
            cx.notify();
        }
    }

    /// Picks a curve or an RSA size (and its family).
    pub fn set_kind(&mut self, kind: KeyKind, cx: &mut Context<Self>) {
        if self.is_generating() {
            return;
        }
        match kind {
            KeyKind::Ed25519 => self.family = KeyFamily::Ed25519,
            KeyKind::EcdsaP256 | KeyKind::EcdsaP384 | KeyKind::EcdsaP521 => {
                self.family = KeyFamily::Ecdsa;
                self.curve = kind;
            }
            KeyKind::Rsa3072 | KeyKind::Rsa4096 => {
                self.family = KeyFamily::Rsa;
                self.rsa = kind;
            }
        }
        cx.notify();
    }

    pub fn set_save_passphrase(&mut self, save: bool, cx: &mut Context<Self>) {
        self.save_passphrase = save;
        cx.notify();
    }

    /// Generate: checks the label, then generates and stores the key on a
    /// background thread.
    pub fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_generating() {
            return;
        }
        let label = self.label.read(cx).value().to_string();
        if let Err(reason) = check_label(&label) {
            self.errors.insert("label".into(), reason);
            cx.notify();
            return;
        }
        let typed = Zeroizing::new(self.passphrase.read(cx).value().to_string());
        let passphrase = (!typed.is_empty()).then(|| SecretString::from(typed.as_str()));
        let save = passphrase.is_some() && self.save_passphrase;
        self.errors.clear();
        let kind = self.kind();
        let task = self.state.update(cx, |state, cx| {
            state.generate_key(label.trim().to_owned(), kind, passphrase, save, cx)
        });
        self.generating = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |form, window, cx| form.generated(result, window, cx));
        }));
        cx.notify();
    }

    fn generated(
        &mut self,
        result: Result<KeyImportResult, CoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.generating = None;
        match result {
            Ok(result) => {
                self.passphrase
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.emit(KeychainEvent::Saved {
                    key_id: result.key.record_id,
                    created: result.created,
                });
            }
            Err(CoreError::Validation(fields)) => self.errors = errors_of(fields),
            Err(error) => {
                tracing::warn!(code = error.code(), "key not generated");
                crate::shell::show_toast(strings::GENERATE_FAILED, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(KeychainEvent::Close);
    }
}

// -- New key (import) -------------------------------------------------------------------

/// What `inspect` says about the text of the import form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inspection {
    /// Nothing typed.
    Empty,
    /// `inspect` runs.
    Checking,
    /// A private key in `format` (its name), encrypted or not.
    Key {
        format: &'static str,
        encrypted: bool,
    },
    /// Not a usable key: the type "unknown" with this reason (UI 7.2).
    Problem(KeyProblem),
}

/// New key: the import of an existing key (UI 7.1, 7.3).
pub struct ImportForm {
    state: Entity<AppState>,
    pub(super) text: Entity<TextareaState>,
    pub(super) label: Entity<InputState>,
    pub(super) passphrase: Entity<InputState>,
    save_passphrase: bool,
    inspection: Inspection,
    /// The import asked for a passphrase (or `inspect` found an encrypted
    /// key): the passphrase field shows.
    needs_passphrase: bool,
    /// Counts the text changes: a late `inspect` of an older text is ignored.
    revision: u64,
    inspecting: Option<Task<()>>,
    errors: FieldErrors,
    file_problem: Option<FileProblem>,
    importing: Option<Task<()>>,
    /// "Import from ~/.ssh": what the folder holds.
    ssh: Option<SshScan>,
    _scan: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ImportForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportForm")
            .field("inspection", &self.inspection)
            .field("importing", &self.importing.is_some())
            .finish_non_exhaustive()
    }
}

impl ImportForm {
    /// `folder`: `Some` when opened from "Import from ~/.ssh" (with the
    /// folder to list, if there is one).
    pub fn new(
        state: Entity<AppState>,
        folder: Option<Option<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 10)
                .placeholder(strings::IMPORT_TEXT_PLACEHOLDER)
        });
        let label =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::IMPORT_LABEL_PLACEHOLDER));
        let passphrase = masked_input(strings::FIELD_PASSPHRASE, window, cx);
        let subscriptions = vec![
            cx.subscribe(&text, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.text_changed(cx);
                }
            }),
            cx.subscribe(&label, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.errors.remove("label");
                    cx.notify();
                }
            }),
            cx.subscribe(&passphrase, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.errors.remove("passphrase");
                    cx.notify();
                }
            }),
        ];
        let (ssh, scan) = match folder {
            None => (None, None),
            Some(None) => (Some(SshScan::Missing), None),
            Some(Some(folder)) => {
                let listed = cx.background_spawn(async move { scan_ssh_folder(&folder) });
                let task = cx.spawn(async move |this, cx| {
                    let found = listed.await;
                    let _ = this.update(cx, |form, cx| {
                        form.ssh = Some(found.map_or(SshScan::Missing, SshScan::Found));
                        cx.notify();
                    });
                });
                (Some(SshScan::Scanning), Some(task))
            }
        };
        text.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            text,
            label,
            passphrase,
            save_passphrase: true,
            inspection: Inspection::Empty,
            needs_passphrase: false,
            revision: 0,
            inspecting: None,
            errors: FieldErrors::new(),
            file_problem: None,
            importing: None,
            ssh,
            _scan: scan,
            _subscriptions: subscriptions,
        }
    }

    pub fn inspection(&self) -> &Inspection {
        &self.inspection
    }

    pub fn needs_passphrase(&self) -> bool {
        self.needs_passphrase
    }

    pub fn is_importing(&self) -> bool {
        self.importing.is_some()
    }

    pub fn errors(&self) -> &FieldErrors {
        &self.errors
    }

    pub fn file_problem(&self) -> Option<FileProblem> {
        self.file_problem
    }

    pub fn ssh_scan(&self) -> Option<&SshScan> {
        self.ssh.as_ref()
    }

    pub fn save_passphrase(&self) -> bool {
        self.save_passphrase
    }

    pub fn set_save_passphrase(&mut self, save: bool, cx: &mut Context<Self>) {
        self.save_passphrase = save;
        cx.notify();
    }

    /// Whether Import can run: some text that is not a refused key.
    pub fn can_import(&self, cx: &gpui::App) -> bool {
        !self.is_importing()
            && !self.text.read(cx).value().trim().is_empty()
            && !matches!(
                self.inspection,
                Inspection::Problem(_) | Inspection::Checking
            )
    }

    /// Puts key text into the form (a chosen, dropped or listed file).
    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_owned();
        self.text
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.file_problem = None;
        self.text_changed(cx);
    }

    /// Reads a key file on a background thread and puts its text in the form.
    pub fn load_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let read = cx.background_spawn(async move { read_key_file(&path) });
        cx.spawn_in(window, async move |this, cx| {
            let read = read.await;
            let _ = this.update_in(cx, |form, window, cx| match read {
                Ok(text) => form.set_text(&text, window, cx),
                Err(problem) => {
                    tracing::info!("key file not read");
                    form.file_problem = Some(problem);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Choose file: the system file dialog, then [`ImportForm::load_file`].
    pub(super) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
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
            let _ = this.update_in(cx, |form, window, cx| form.load_file(path, window, cx));
        })
        .detach();
    }

    /// The text changed: `inspect` it again on a background thread.
    fn text_changed(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.errors.remove("private_key");
        self.errors.remove("passphrase");
        let text = Zeroizing::new(self.text.read(cx).value().to_string());
        if text.trim().is_empty() {
            self.inspection = Inspection::Empty;
            self.needs_passphrase = false;
            self.inspecting = None;
            cx.notify();
            return;
        }
        self.inspection = Inspection::Checking;
        let revision = self.revision;
        let inspected = cx.background_spawn(async move { shellpp_core::keys::inspect(&text) });
        self.inspecting = Some(cx.spawn(async move |this, cx| {
            let result = inspected.await;
            let _ = this.update(cx, |form, cx| {
                if form.revision != revision {
                    return;
                }
                form.inspecting = None;
                form.inspection = match result {
                    Ok((format, encrypted)) => {
                        form.needs_passphrase = encrypted;
                        Inspection::Key {
                            format: format.label(),
                            encrypted,
                        }
                    }
                    Err(problem) => {
                        form.needs_passphrase = false;
                        Inspection::Problem(problem)
                    }
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Import: stores the key through the core (decryption on a background
    /// thread); a key that is already saved is opened instead.
    pub fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_import(cx) {
            if self.text.read(cx).value().trim().is_empty() {
                self.errors
                    .insert("private_key".into(), ValidationReason::Required);
                cx.notify();
            }
            return;
        }
        let label = self.label.read(cx).value().to_string();
        if let Err(ValidationReason::TooLong) = check_label(&label) {
            self.errors
                .insert("label".into(), ValidationReason::TooLong);
            cx.notify();
            return;
        }
        let label = (!label.trim().is_empty()).then(|| label.trim().to_owned());
        let text = SecretString::from(self.text.read(cx).value().to_string());
        let typed = Zeroizing::new(self.passphrase.read(cx).value().to_string());
        let passphrase = (self.needs_passphrase && !typed.is_empty())
            .then(|| SecretString::from(typed.as_str()));
        let save = passphrase.is_some() && self.save_passphrase;
        self.errors.clear();
        let task = self.state.update(cx, |state, cx| {
            state.import_key(text, label, passphrase, save, cx)
        });
        self.importing = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |form, window, cx| form.imported(result, window, cx));
        }));
        cx.notify();
    }

    fn imported(
        &mut self,
        result: Result<KeyImportResult, CoreError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.importing = None;
        match result {
            Ok(result) => {
                // Nothing of the key stays in the inputs.
                for input in [&self.label, &self.passphrase] {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                }
                self.text
                    .update(cx, |input, cx| input.set_value("", window, cx));
                cx.emit(KeychainEvent::Saved {
                    key_id: result.key.record_id,
                    created: result.created,
                });
            }
            Err(CoreError::Validation(fields)) => {
                let errors = errors_of(fields);
                if errors.get("passphrase") == Some(&ValidationReason::Required)
                    && !self.needs_passphrase
                {
                    // An encrypted key the inspection did not flag: ask now.
                    self.needs_passphrase = true;
                    let input = self.passphrase.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                } else {
                    if errors.contains_key("passphrase") {
                        let input = self.passphrase.clone();
                        input.update(cx, |input, cx| input.focus(window, cx));
                    }
                    self.errors = errors;
                }
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "key not imported");
                crate::shell::show_toast(strings::IMPORT_FAILED, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(KeychainEvent::Close);
    }
}
