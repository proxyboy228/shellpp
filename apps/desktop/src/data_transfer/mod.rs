//! Export Data and Import Data (SPEC principle 4; app menu, UI 1.4; night
//! decisions D34 and D38): the native JSON export of `shellpp_core`, plain
//! or encrypted, and its import.
//!
//! - Export, format "Shell++ data": "Encrypt with a password" is on by
//!   default (password and repeat, at least 8 characters); turning it off
//!   shows the warning that the file holds every secret in clear text and
//!   asks for "I understand".
//! - Export, format "OpenSSH files" (night plan, stage 3 item 11; D43):
//!   the core writes `shellpp-ssh-config` and, when "Include the key files"
//!   is on, the key files into a folder the user chooses; "Protect the key
//!   files with a passphrase" (on) encrypts the keys that have none, and
//!   off asks for "I understand". Files that are there already are
//!   replaced only after "Replace".
//!   The core builds the file off the UI thread (Argon2id for the encrypted
//!   one), then the save dialog suggests the core's file name; the file is
//!   written for the user only (mode 0600 on Unix, a protected DACL on
//!   Windows, D42) and a toast says how many records it holds.
//!   "Include command history" and "Include the connection log" are off by
//!   default (D50).
//! - Import: a file (typed or chosen), "Apply the settings in the file"
//!   (on) and "Full restore from my backup" (off; D50: every setting, with
//!   the warning that the file may hold a script that then runs on the
//!   servers); an encrypted file asks for its password; a file that holds
//!   command history or a connection log asks whether to import them (both
//!   on, D51); the import runs off the UI thread and ends with the summary
//!   (added, already here, imported as copies, skipped, errors, settings;
//!   the settings an import never applies, D41, when the file has other
//!   values for them; keys kept as unknown and history left out by the
//!   secret detector, when there are any).
//!
//! Nothing here logs paths, passwords or the file's content; the text of a
//! file is held as a secret until the import has run.

pub mod strings;

#[cfg(test)]
mod tests;

use std::io::Read as _;

use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::radio::Radio;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use secrecy::{ExposeSecret, SecretString};
use shellpp_core::service::dto::{
    DataExportInput, DataImportContents, DataImportContentsInput, DataImportInput,
    DataImportSummary,
};
use shellpp_core::service::ssh_export::SshFilesExportInput;
use shellpp_core::{CoreError, ValidationReason};

use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// Width of the dialogs (as the import dialog of Known Hosts).
const DIALOG_WIDTH: f32 = 480.;
/// The largest file Import Data reads (the core's limit).
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// The shortest export password, as the master password (D29).
const MIN_PASSWORD_CHARS: usize = 8;

/// Characters of a typed password (the core counts them again after NFC).
fn chars(text: &str) -> usize {
    text.chars().count()
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Where the export dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportStep {
    Form,
    Running,
    /// OpenSSH files: files of the export exist in the chosen folder.
    Replace,
}

/// What Export Data writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The native JSON of every record and the settings.
    Data,
    /// `shellpp-ssh-config` and the key files (storage-format 13.4–13.5).
    OpenSsh,
}

pub struct ExportDialog {
    state: Entity<AppState>,
    format: ExportFormat,
    /// OpenSSH files: the key files too.
    include_keys: bool,
    /// OpenSSH files: the folder waiting for "Replace".
    folder: Option<PathBuf>,
    /// Shell++ data: the command history and the connection log too (D50).
    include_history: bool,
    include_connection_log: bool,
    /// The JSON's encryption, or the key files' passphrase.
    encrypt: bool,
    understood: bool,
    password: Entity<InputState>,
    repeat: Entity<InputState>,
    step: ExportStep,
    password_error: Option<&'static str>,
    repeat_error: Option<&'static str>,
    error: Option<&'static str>,
    task: Option<Task<()>>,
    /// Stage 7 (UI 14): entries of Recently deleted to export instead of
    /// the vault; empty for an ordinary export.
    pub recently_deleted: Vec<uuid::Uuid>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ExportDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportDialog")
            .field("step", &self.step)
            .field("format", &self.format)
            .field("encrypt", &self.encrypt)
            .finish_non_exhaustive()
    }
}

impl ExportDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let repeat = cx.new(|cx| InputState::new(window, cx).masked(true));
        let mut subscriptions = Vec::new();
        for input in [&password, &repeat] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.run(window, cx),
                    InputEvent::Change => {
                        this.password_error = None;
                        this.repeat_error = None;
                        cx.notify();
                    }
                    InputEvent::Focus | InputEvent::Blur => cx.notify(),
                },
            ));
        }
        password.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            format: ExportFormat::Data,
            include_keys: true,
            folder: None,
            include_history: false,
            include_connection_log: false,
            encrypt: true,
            understood: false,
            password,
            repeat,
            step: ExportStep::Form,
            password_error: None,
            repeat_error: None,
            error: None,
            task: None,
            recently_deleted: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn step(&self) -> ExportStep {
        self.step
    }

    pub fn format(&self) -> ExportFormat {
        self.format
    }

    pub fn set_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        if self.step != ExportStep::Form {
            return;
        }
        self.format = format;
        self.understood = false;
        self.error = None;
        self.password_error = None;
        self.repeat_error = None;
        cx.notify();
    }

    pub fn set_include_keys(&mut self, include: bool, cx: &mut Context<Self>) {
        self.include_keys = include;
        self.understood = false;
        cx.notify();
    }

    pub fn set_include_history(&mut self, include: bool, cx: &mut Context<Self>) {
        self.include_history = include;
        cx.notify();
    }

    pub fn set_include_connection_log(&mut self, include: bool, cx: &mut Context<Self>) {
        self.include_connection_log = include;
        cx.notify();
    }

    /// Whether the form asks for a password or passphrase at all.
    fn secret_asked(&self) -> bool {
        self.format == ExportFormat::Data || self.include_keys
    }

    /// "Encrypt with a password" (on when the dialog opens).
    pub fn encrypt(&self) -> bool {
        self.encrypt
    }

    pub fn set_encrypt(&mut self, encrypt: bool, cx: &mut Context<Self>) {
        self.encrypt = encrypt;
        self.understood = false;
        self.error = None;
        cx.notify();
    }

    pub fn set_understood(&mut self, understood: bool, cx: &mut Context<Self>) {
        self.understood = understood;
        cx.notify();
    }

    /// Fills both password fields (tests).
    pub fn set_passwords(
        &mut self,
        password: &str,
        repeat: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (password, repeat) = (password.to_owned(), repeat.to_owned());
        self.password
            .update(cx, |input, cx| input.set_value(password, window, cx));
        self.repeat
            .update(cx, |input, cx| input.set_value(repeat, window, cx));
    }

    /// Export is enabled: not running, and "I understand" for a plain file
    /// (or plain key files).
    pub fn can_run(&self) -> bool {
        match self.step {
            ExportStep::Form => !self.secret_asked() || self.encrypt || self.understood,
            ExportStep::Replace => true,
            ExportStep::Running => false,
        }
    }

    /// The typed password (or passphrase) when the form asks for one and
    /// both fields agree; `Err` after showing what is wrong.
    fn checked_secret(&mut self, cx: &mut Context<Self>) -> Result<Option<SecretString>, ()> {
        if !(self.secret_asked() && self.encrypt) {
            return Ok(None);
        }
        let first = Zeroizing::new(self.password.read(cx).value().to_string());
        let second = Zeroizing::new(self.repeat.read(cx).value().to_string());
        let mismatch = match self.format {
            ExportFormat::Data => strings::PASSWORD_MISMATCH,
            ExportFormat::OpenSsh => strings::PASSPHRASE_MISMATCH,
        };
        self.password_error =
            (chars(&first) < MIN_PASSWORD_CHARS).then_some(strings::PASSWORD_TOO_SHORT);
        self.repeat_error = (self.password_error.is_none() && first != second).then_some(mismatch);
        if self.password_error.is_some() || self.repeat_error.is_some() {
            cx.notify();
            return Err(());
        }
        Ok(Some(SecretString::from(first.to_string())))
    }

    /// Checks the form, builds the file off the UI thread, then asks where
    /// to save it.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_run() {
            return;
        }
        if self.format == ExportFormat::OpenSsh {
            self.run_openssh(window, cx);
            return;
        }
        let Ok(password) = self.checked_secret(cx) else {
            return;
        };
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.step = ExportStep::Running;
        self.error = None;
        cx.notify();
        let input = DataExportInput {
            password,
            include_history: self.include_history,
            include_connection_log: self.include_connection_log,
            recently_deleted: self.recently_deleted.clone(),
        };
        let built = cx.background_spawn(async move { core.data_export(input) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let export = match built.await {
                Ok(export) => export,
                Err(error) => {
                    tracing::warn!(code = error.code(), "data not exported");
                    let _ = this.update(cx, |dialog, cx| dialog.fail(strings::EXPORT_FAILED, cx));
                    return;
                }
            };
            let directory = ::dirs::document_dir()
                .or_else(::dirs::home_dir)
                .unwrap_or_default();
            let Ok(chosen) = this.update(cx, |_, cx| {
                cx.prompt_for_new_path(&directory, Some(&export.file_name))
            }) else {
                return;
            };
            let Ok(Ok(Some(path))) = chosen.await else {
                // Cancelled: back to the form, nothing written.
                let _ = this.update(cx, |dialog, cx| dialog.back(cx));
                return;
            };
            let records: usize = export.counts.values().sum();
            let written = cx
                .background_spawn(async move { write_private(&path, export.text.expose_secret()) })
                .await;
            let _ = this.update_in(cx, |dialog, window, cx| match written {
                Ok(()) => {
                    window.close_dialog(cx);
                    crate::shell::show_toast(strings::exported(records), window, cx);
                }
                Err(()) => dialog.fail(strings::EXPORT_WRITE_FAILED, cx),
            });
        }));
    }

    /// OpenSSH files: the folder (asked for first, or the one waiting for
    /// "Replace"), then the core writes the files off the UI thread.
    fn run_openssh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(passphrase) = self.checked_secret(cx) else {
            return;
        };
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let keys = self.include_keys;
        let replace = self.folder.take();
        self.step = ExportStep::Running;
        self.error = None;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let overwrite = replace.is_some();
            let dir = match replace {
                Some(dir) => dir,
                None => {
                    let Ok(chosen) = this.update(cx, |_, cx| {
                        cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: Some(strings::OPENSSH_CHOOSE.into()),
                        })
                    }) else {
                        return;
                    };
                    match chosen.await {
                        Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                        // Cancelled: back to the form, nothing written.
                        _ => {
                            let _ = this.update(cx, |dialog, cx| dialog.back(cx));
                            return;
                        }
                    }
                }
            };
            let input = SshFilesExportInput {
                dir: dir.clone(),
                keys,
                passphrase,
                overwrite,
            };
            let written = cx
                .background_spawn(async move { core.ssh_files_export(input) })
                .await;
            let _ = this.update_in(cx, |dialog, window, cx| match written {
                Ok(summary) => {
                    window.close_dialog(cx);
                    let text = strings::exported_openssh(summary.hosts, summary.keys);
                    crate::shell::show_toast(text, window, cx);
                }
                Err(CoreError::Validation(errors))
                    if errors.iter().any(|error| {
                        error.field == "files" && error.reason == ValidationReason::Duplicate
                    }) =>
                {
                    dialog.folder = Some(dir);
                    dialog.step = ExportStep::Replace;
                    dialog.task = None;
                    cx.notify();
                }
                Err(error) => {
                    tracing::warn!(code = error.code(), "openssh files not exported");
                    dialog.fail(strings::EXPORT_WRITE_FAILED, cx);
                }
            });
        }));
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.step = ExportStep::Form;
        self.folder = None;
        self.task = None;
        cx.notify();
    }

    fn fail(&mut self, error: &'static str, cx: &mut Context<Self>) {
        self.step = ExportStep::Form;
        self.error = Some(error);
        self.task = None;
        cx.notify();
    }
}

/// Writes `text` to `path`, readable by the user only before a byte of it
/// is written (Unix mode 0600, Windows a protected DACL for the user, also
/// when the save dialog replaces an existing file; D42).
fn write_private(path: &Path, text: &str) -> Result<(), ()> {
    shellpp_private_file::write(
        path,
        text.as_bytes(),
        shellpp_private_file::Existing::Replace,
    )
    .map_err(|_| ())
}

impl Render for ExportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let running = self.step == ExportStep::Running;
        if self.step == ExportStep::Replace {
            return div()
                .id("data-export-dialog")
                .test_support()
                .w_full()
                .child(
                    div()
                        .id("data-export-replace-text")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .child(strings::OPENSSH_REPLACE),
                );
        }
        let openssh = self.format == ExportFormat::OpenSsh;
        let format =
            v_flex()
                .gap(metrics::SPACE_1)
                .child(
                    Radio::new("data-export-format-data")
                        .label(strings::FORMAT_DATA)
                        .checked(!openssh)
                        .disabled(running)
                        .on_click(cx.listener(|this, _: &bool, _, cx| {
                            this.set_format(ExportFormat::Data, cx)
                        })),
                )
                .child(
                    Radio::new("data-export-format-openssh")
                        .label(strings::FORMAT_OPENSSH)
                        .checked(openssh)
                        .disabled(running)
                        .on_click(cx.listener(|this, _: &bool, _, cx| {
                            this.set_format(ExportFormat::OpenSsh, cx)
                        })),
                );
        let mut body = v_flex().w_full().gap(metrics::SPACE_3).child(format);
        if openssh {
            body = body
                .child(widgets::caption(strings::OPENSSH_HINT, cx))
                .child(
                    Checkbox::new("data-export-keys")
                        .label(strings::OPENSSH_KEYS)
                        .checked(self.include_keys)
                        .disabled(running)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_include_keys(*checked, cx)
                        })),
                );
        } else {
            body = body.child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(
                        Checkbox::new("data-export-history")
                            .label(strings::EXPORT_HISTORY)
                            .checked(self.include_history)
                            .disabled(running)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_include_history(*checked, cx)
                            })),
                    )
                    .child(
                        Checkbox::new("data-export-connection-log")
                            .label(strings::EXPORT_CONNECTION_LOG)
                            .checked(self.include_connection_log)
                            .disabled(running)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_include_connection_log(*checked, cx)
                            })),
                    ),
            );
        }
        if self.secret_asked() {
            let (label, hint) = if openssh {
                (strings::OPENSSH_PROTECT, None)
            } else {
                (strings::EXPORT_ENCRYPT, Some(strings::EXPORT_ENCRYPT_HINT))
            };
            body = body.child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(
                        Switch::new("data-export-encrypt")
                            .checked(self.encrypt)
                            .label(label)
                            .disabled(running)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_encrypt(*checked, cx);
                            })),
                    )
                    .when_some(hint, |this, hint| this.child(widgets::caption(hint, cx))),
            );
        }
        let (password_label, repeat_label, plain_warning) = if openssh {
            (
                strings::OPENSSH_PASSPHRASE,
                strings::OPENSSH_REPEAT,
                strings::OPENSSH_PLAIN_WARNING,
            )
        } else {
            (
                strings::EXPORT_PASSWORD,
                strings::EXPORT_REPEAT,
                strings::EXPORT_PLAIN_WARNING,
            )
        };
        // No key files: nothing secret is written, nothing is asked.
        if self.secret_asked() && self.encrypt {
            body = body
                .child(crate::lock::password_field(
                    password_label,
                    &self.password,
                    "data-export-password",
                    self.password_error.is_some(),
                    running,
                    window,
                    cx,
                ))
                .when_some(self.password_error, |this, error| {
                    this.child(widgets::error_text(error, cx))
                })
                .child(crate::lock::password_field(
                    repeat_label,
                    &self.repeat,
                    "data-export-repeat",
                    self.repeat_error.is_some(),
                    running,
                    window,
                    cx,
                ))
                .when_some(self.repeat_error, |this, error| {
                    this.child(widgets::error_text(error, cx))
                });
        } else if self.secret_asked() {
            body = body
                .child(
                    div()
                        .id("data-export-warning")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.danger)
                        .child(plain_warning),
                )
                .child(
                    Checkbox::new("data-export-understand")
                        .label(strings::EXPORT_UNDERSTAND)
                        .checked(self.understood)
                        .disabled(running)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_understood(*checked, cx)
                        })),
                );
        }
        if let Some(error) = self.error {
            body = body.child(widgets::error_text(error, cx));
        }
        div()
            .id("data-export-dialog")
            .test_support()
            .w_full()
            .child(body)
    }
}

fn export_footer(dialog: &Entity<ExportDialog>, cx: &App) -> impl IntoElement {
    let step = dialog.read(cx).step;
    let running = step == ExportStep::Running;
    let enabled = dialog.read(cx).can_run();
    let run = dialog.downgrade();
    let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2).child(
        ControlButton::new("data-export-cancel", ControlKind::Neutral)
            .label(strings::CANCEL)
            .on_click(|_, window, cx| window.close_dialog(cx)),
    );
    if step == ExportStep::Replace {
        let back = dialog.downgrade();
        return row
            .child(
                ControlButton::new("data-export-back", ControlKind::Neutral)
                    .label(strings::BACK)
                    .on_click(move |_, _, cx| {
                        let _ = back.update(cx, |dialog, cx| dialog.back(cx));
                    }),
            )
            .child(
                ControlButton::new("data-export-replace", ControlKind::Danger)
                    .label(strings::OPENSSH_REPLACE_RUN)
                    .on_click(move |_, window, cx| {
                        let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                    }),
            );
    }
    row.child(
        ControlButton::new("data-export-run", ControlKind::Accent)
            .label(if running {
                strings::EXPORT_RUNNING
            } else {
                strings::EXPORT_RUN
            })
            .disabled(!enabled)
            .on_click(move |_, window, cx| {
                let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
            }),
    )
}

/// Opens Export Data over the current window. Returns its entity.
pub fn open_export(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ExportDialog> {
    let dialog = cx.new(|cx| ExportDialog::new(state, window, cx));
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::EXPORT_TITLE))
            .child(shown.clone())
            .footer(export_footer(&shown, cx))
    });
    dialog
}

fn title(text: &'static str) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Where the import dialog is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportStep {
    File {
        error: Option<&'static str>,
    },
    Password {
        error: Option<&'static str>,
    },
    /// The file holds command history or a connection log: import them?
    Contents(DataImportContents),
    Running,
    Summary(DataImportSummary),
}

pub struct ImportDialog {
    state: Entity<AppState>,
    path: Entity<InputState>,
    password: Entity<InputState>,
    apply_settings: bool,
    /// Every setting of the file, D41 aside (D50).
    full_restore: bool,
    /// The file's history and connection log, when it holds them (D51).
    import_history: bool,
    import_connection_log: bool,
    /// The content of the file while its password or its options are
    /// asked.
    text: Option<SecretString>,
    /// The password the file opened with.
    opened_with: Option<SecretString>,
    step: ImportStep,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ImportDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportDialog")
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// Reads an export file within the limit; `Err` is the message. The bytes
/// land in one buffer of the file's size (no copies left behind by a
/// growing buffer) and become the secret without another copy; a file that
/// is not UTF-8 is wiped before it is dropped.
fn read_file(path: &Path) -> Result<SecretString, &'static str> {
    let file = std::fs::File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => strings::FILE_NOT_FOUND,
        _ => strings::FILE_UNREADABLE,
    })?;
    let metadata = file.metadata().map_err(|_| strings::FILE_UNREADABLE)?;
    if !metadata.is_file() {
        return Err(strings::FILE_UNREADABLE);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(strings::FILE_TOO_LARGE);
    }
    // Exactly the size: the text then fills its buffer, so neither the
    // read (which probes for the end on the stack) nor the secret (which
    // shrinks a buffer to its length) moves it and leaves a copy behind.
    let size = usize::try_from(metadata.len()).map_err(|_| strings::FILE_TOO_LARGE)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(size));
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| strings::FILE_NOT_EXPORT)?;
    match String::from_utf8(std::mem::take(&mut *bytes)) {
        Ok(text) => Ok(SecretString::from(text)),
        Err(error) => {
            drop(Zeroizing::new(error.into_bytes()));
            Err(strings::FILE_NOT_EXPORT)
        }
    }
}

/// The `format` member of a file, read without keeping the rest: the other
/// members are skipped, so no copy of a plain export's secrets is made.
#[derive(serde::Deserialize)]
struct Head<'a> {
    #[serde(borrow)]
    format: Option<std::borrow::Cow<'a, str>>,
}

/// Whether a file's JSON names the encrypted format; `None` when it is not
/// an export at all.
fn encrypted(text: &str) -> Option<bool> {
    let head: Head<'_> = serde_json::from_str(text).ok()?;
    match head.format?.as_ref() {
        shellpp_core::ids::EXPORT_FORMAT_V1 => Some(false),
        shellpp_core::ids::EXPORT_FORMAT_ENCRYPTED_V1 => Some(true),
        _ => None,
    }
}

impl ImportDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = cx.new(|cx| InputState::new(window, cx));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let mut subscriptions = Vec::new();
        for input in [&path, &password] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.run(window, cx),
                    InputEvent::Change => {
                        match &mut this.step {
                            ImportStep::File { error } | ImportStep::Password { error } => {
                                *error = None;
                            }
                            _ => {}
                        }
                        cx.notify();
                    }
                    InputEvent::Focus | InputEvent::Blur => cx.notify(),
                },
            ));
        }
        path.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            path,
            password,
            apply_settings: true,
            full_restore: false,
            import_history: true,
            import_connection_log: true,
            text: None,
            opened_with: None,
            step: ImportStep::File { error: None },
            task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn step(&self) -> &ImportStep {
        &self.step
    }

    pub fn set_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = path.display().to_string();
        self.path
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.step = ImportStep::File { error: None };
        cx.notify();
    }

    /// Fills the password field (tests).
    pub fn set_password(&mut self, password: &str, window: &mut Window, cx: &mut Context<Self>) {
        let password = password.to_owned();
        self.password
            .update(cx, |input, cx| input.set_value(password, window, cx));
    }

    pub fn set_apply_settings(&mut self, apply: bool, cx: &mut Context<Self>) {
        self.apply_settings = apply;
        cx.notify();
    }

    pub fn set_full_restore(&mut self, full: bool, cx: &mut Context<Self>) {
        self.full_restore = full;
        cx.notify();
    }

    pub fn set_import_history(&mut self, import: bool, cx: &mut Context<Self>) {
        self.import_history = import;
        cx.notify();
    }

    pub fn set_import_connection_log(&mut self, import: bool, cx: &mut Context<Self>) {
        self.import_connection_log = import;
        cx.notify();
    }

    /// "Choose file…": the system open dialog.
    pub fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let _ = this.update_in(cx, |dialog, window, cx| dialog.set_path(&path, window, cx));
        })
        .detach();
    }

    /// The primary button: reads the file (File step), opens it with the
    /// typed password (Password step) or imports it (Contents step).
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            ImportStep::File { .. } => self.read(window, cx),
            ImportStep::Password { .. } => {
                let password = self.password.read(cx).value().to_string();
                if password.is_empty() {
                    return;
                }
                if let Some(text) = self.text.clone() {
                    self.open(text, Some(SecretString::from(password)), window, cx);
                }
            }
            ImportStep::Contents(_) => {
                if let Some(text) = self.text.clone() {
                    let password = self.opened_with.clone();
                    self.import(text, password, window, cx);
                }
            }
            ImportStep::Running | ImportStep::Summary(_) => {}
        }
    }

    fn read(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.path.read(cx).value().trim().to_owned();
        if typed.is_empty() {
            self.step = ImportStep::File {
                error: Some(strings::FILE_MISSING),
            };
            cx.notify();
            return;
        }
        let path = PathBuf::from(typed);
        self.step = ImportStep::Running;
        cx.notify();
        let work = cx.background_spawn(async move {
            let text = read_file(&path)?;
            let encrypted = encrypted(text.expose_secret()).ok_or(strings::FILE_NOT_EXPORT)?;
            Ok::<_, &'static str>((text, encrypted))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |dialog, window, cx| match result {
                Ok((text, true)) => {
                    dialog.text = Some(text);
                    dialog.step = ImportStep::Password { error: None };
                    dialog.task = None;
                    dialog
                        .password
                        .update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                }
                Ok((text, false)) => dialog.open(text, None, window, cx),
                Err(error) => {
                    dialog.step = ImportStep::File { error: Some(error) };
                    dialog.task = None;
                    cx.notify();
                }
            });
        }));
    }

    /// Opens the file (with its password when encrypted) to see whether it
    /// holds command history or a connection log: then the Contents step
    /// asks about them, else the import runs at once.
    fn open(
        &mut self,
        text: SecretString,
        password: Option<SecretString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let encrypted_file = password.is_some();
        self.step = ImportStep::Running;
        cx.notify();
        let input = DataImportContentsInput {
            text: text.clone(),
            password: password.clone(),
        };
        let work = cx.background_spawn(async move { core.data_import_contents(input) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |dialog, window, cx| match result {
                Ok(contents) if contents.history > 0 || contents.connection_log > 0 => {
                    dialog.task = None;
                    dialog.text = Some(text);
                    dialog.opened_with = password;
                    dialog.step = ImportStep::Contents(contents);
                    cx.notify();
                }
                Ok(_) => dialog.import(text, password, window, cx),
                Err(error) => {
                    dialog.task = None;
                    dialog.step = dialog.failed(&error, encrypted_file);
                    cx.notify();
                }
            });
        }));
    }

    /// The step after an error of the core: a wrong password asks again;
    /// otherwise back to the file.
    fn failed(&mut self, error: &CoreError, encrypted_file: bool) -> ImportStep {
        match error {
            CoreError::Validation(errors)
                if encrypted_file
                    && errors.iter().any(|error| {
                        error.field == "password" && error.reason == ValidationReason::Invalid
                    }) =>
            {
                ImportStep::Password {
                    error: Some(strings::PASSWORD_WRONG),
                }
            }
            CoreError::Validation(_) => {
                self.text = None;
                self.opened_with = None;
                ImportStep::File {
                    error: Some(strings::FILE_NOT_EXPORT),
                }
            }
            error => {
                tracing::warn!(code = error.code(), "data not imported");
                self.text = None;
                self.opened_with = None;
                ImportStep::File {
                    error: Some(strings::IMPORT_FAILED),
                }
            }
        }
    }

    fn import(
        &mut self,
        text: SecretString,
        password: Option<SecretString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let encrypted_file = password.is_some();
        let input = DataImportInput {
            apply_settings: self.apply_settings || self.full_restore,
            import_history: self.import_history,
            import_connection_log: self.import_connection_log,
            full_restore: self.full_restore,
            ..DataImportInput::new(text, password)
        };
        self.step = ImportStep::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.data_import(input) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                dialog.step = match result {
                    Ok(summary) => {
                        dialog.text = None;
                        dialog.opened_with = None;
                        ImportStep::Summary(summary)
                    }
                    Err(error) => dialog.failed(&error, encrypted_file),
                };
                cx.notify();
            });
        }));
    }

    fn render_summary(&self, summary: &DataImportSummary, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let row = |id: &'static str, label: &'static str, value: usize| {
            h_flex()
                .id(id)
                .test_support()
                .w_full()
                .h(metrics::rpx(28.))
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(value.to_string()),
                )
        };
        v_flex()
            .id("data-import-summary")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(row(
                "data-import-added",
                strings::SUMMARY_ADDED,
                summary.added,
            ))
            .child(row(
                "data-import-existing",
                strings::SUMMARY_EXISTING,
                summary.existing,
            ))
            .child(row(
                "data-import-conflicts",
                strings::SUMMARY_CONFLICTS,
                summary.conflicts,
            ))
            .child(row(
                "data-import-skipped",
                strings::SUMMARY_SKIPPED,
                summary.skipped,
            ))
            .child(row(
                "data-import-errors",
                strings::SUMMARY_ERRORS,
                summary.refused.len(),
            ))
            .child(row(
                "data-import-settings-applied",
                strings::SUMMARY_SETTINGS,
                summary.settings,
            ))
            // Settings an import never applies (D41), only when the file
            // has other values for them.
            .when(summary.settings_left_out > 0, |this| {
                this.child(row(
                    "data-import-settings-left-out",
                    strings::SUMMARY_SETTINGS_LEFT_OUT,
                    summary.settings_left_out,
                ))
            })
            .when(summary.unknown_keys > 0, |this| {
                this.child(row(
                    "data-import-unknown-keys",
                    strings::SUMMARY_UNKNOWN_KEYS,
                    summary.unknown_keys,
                ))
            })
            .when(summary.history_left_out > 0, |this| {
                this.child(row(
                    "data-import-history-left-out",
                    strings::SUMMARY_HISTORY_LEFT_OUT,
                    summary.history_left_out,
                ))
            })
            // Rules whose autostart the import turned off (D63).
            .when(summary.rules_autostart_off > 0, |this| {
                this.child(row(
                    "data-import-rules-autostart-off",
                    strings::SUMMARY_RULES_AUTOSTART_OFF,
                    summary.rules_autostart_off,
                ))
            })
    }

    fn render_contents(
        &self,
        contents: DataImportContents,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id("data-import-contents")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_2)
            .child(widgets::caption(strings::IMPORT_CONTENTS_HINT, cx))
            .when(contents.history > 0, |this| {
                this.child(
                    Checkbox::new("data-import-history")
                        .label(strings::import_history(contents.history))
                        .checked(self.import_history)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_import_history(*checked, cx)
                        })),
                )
            })
            .when(contents.connection_log > 0, |this| {
                this.child(
                    Checkbox::new("data-import-connection-log")
                        .label(strings::import_connection_log(contents.connection_log))
                        .checked(self.import_connection_log)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_import_connection_log(*checked, cx)
                        })),
                )
            })
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = match &self.step {
            ImportStep::Summary(summary) => self.render_summary(summary, cx).into_any_element(),
            ImportStep::Contents(contents) => {
                let contents = *contents;
                self.render_contents(contents, cx).into_any_element()
            }
            ImportStep::File { error } => v_flex()
                .w_full()
                .gap(metrics::SPACE_3)
                .child(widgets::field(
                    strings::IMPORT_FILE,
                    h_flex()
                        .w_full()
                        .gap(metrics::SPACE_2)
                        .child(div().flex_1().min_w_0().child(widgets::text_input(
                            &self.path,
                            "data-import-path",
                            error.is_some(),
                            window,
                            cx,
                        )))
                        .child(
                            ControlButton::new("data-import-browse", ControlKind::Neutral)
                                .label(strings::IMPORT_BROWSE)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.browse(window, cx)
                                })),
                        ),
                    *error,
                    cx,
                ))
                .child(
                    Switch::new("data-import-settings")
                        .checked(self.apply_settings || self.full_restore)
                        .disabled(self.full_restore)
                        .label(strings::IMPORT_SETTINGS)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_apply_settings(*checked, cx)
                        })),
                )
                .child(
                    v_flex()
                        .gap(metrics::SPACE_1)
                        .child(
                            Switch::new("data-import-full-restore")
                                .checked(self.full_restore)
                                .label(strings::IMPORT_FULL_RESTORE)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_full_restore(*checked, cx)
                                })),
                        )
                        .when(self.full_restore, |this| {
                            this.child(
                                div()
                                    .id("data-import-full-restore-warning")
                                    .test_support()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .text_color(theme.danger)
                                    .child(strings::IMPORT_FULL_RESTORE_WARNING),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(strings::IMPORT_HINT),
                )
                .into_any_element(),
            ImportStep::Password { error } => v_flex()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(strings::IMPORT_PASSWORD_HINT, cx))
                .child(crate::lock::password_field(
                    strings::IMPORT_PASSWORD,
                    &self.password,
                    "data-import-password",
                    error.is_some(),
                    false,
                    window,
                    cx,
                ))
                .when_some(*error, |this, error| {
                    this.child(widgets::error_text(error, cx))
                })
                .into_any_element(),
            ImportStep::Running => div()
                .id("data-import-running")
                .test_support()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.muted_foreground)
                .child(strings::IMPORT_RUNNING)
                .into_any_element(),
        };
        div()
            .id("data-import-dialog")
            .test_support()
            .w_full()
            .child(body)
    }
}

fn import_footer(dialog: &Entity<ImportDialog>, cx: &App) -> impl IntoElement {
    let step = dialog.read(cx).step.clone();
    let run = dialog.downgrade();
    let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
    match step {
        ImportStep::Summary(_) => row.child(
            ControlButton::new("data-import-close", ControlKind::Accent)
                .label(strings::CLOSE)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        ),
        _ => {
            let running = step == ImportStep::Running;
            row.child(
                ControlButton::new("data-import-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                ControlButton::new("data-import-run", ControlKind::Accent)
                    .label(if running {
                        strings::IMPORT_RUNNING
                    } else {
                        strings::IMPORT_NEXT
                    })
                    .disabled(running)
                    .on_click(move |_, window, cx| {
                        let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                    }),
            )
        }
    }
}

/// Opens Import Data over the current window. Returns its entity.
pub fn open_import(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ImportDialog> {
    let dialog = cx.new(|cx| ImportDialog::new(state, window, cx));
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let heading = match shown.read(cx).step {
            ImportStep::Summary(_) => strings::SUMMARY_TITLE,
            _ => strings::IMPORT_TITLE,
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading))
            .child(shown.clone())
            .footer(import_footer(&shown, cx))
    });
    dialog
}
