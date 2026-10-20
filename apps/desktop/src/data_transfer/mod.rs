//! Export Data and Import Data (SPEC principle 4; app menu, UI 1.4; night
//! decisions D34 and D38): the native JSON export of `shellpp_core`, plain
//! or encrypted, and its import.
//!
//! - Export: "Encrypt with a password" is on by default (password and
//!   repeat, at least 8 characters); turning it off shows the warning that
//!   the file holds every secret in clear text and asks for "I understand".
//!   The core builds the file off the UI thread (Argon2id for the encrypted
//!   one), then the save dialog suggests the core's file name; the file is
//!   written with mode 0600 on Unix and a toast says how many records it
//!   holds.
//! - Import: a file (typed or chosen), "Apply the settings in the file"
//!   (on); an encrypted file asks for its password; the import runs off the
//!   UI thread and ends with the summary (added, already here, imported as
//!   copies, skipped, errors, settings; the settings an import never
//!   applies, D41, when the file has other values for them).
//!
//! Nothing here logs paths, passwords or the file's content; the text of a
//! file is held as a secret until the import has run.

pub mod strings;

#[cfg(test)]
mod tests;

use std::io::Read as _;
use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Task,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use secrecy::{ExposeSecret, SecretString};
use shellpp_core::service::dto::{DataExportInput, DataImportInput, DataImportSummary};
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
}

pub struct ExportDialog {
    state: Entity<AppState>,
    encrypt: bool,
    understood: bool,
    password: Entity<InputState>,
    repeat: Entity<InputState>,
    step: ExportStep,
    password_error: Option<&'static str>,
    repeat_error: Option<&'static str>,
    error: Option<&'static str>,
    task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ExportDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportDialog")
            .field("step", &self.step)
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
            encrypt: true,
            understood: false,
            password,
            repeat,
            step: ExportStep::Form,
            password_error: None,
            repeat_error: None,
            error: None,
            task: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn step(&self) -> ExportStep {
        self.step
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

    /// Export is enabled: not running, and "I understand" for a plain file.
    pub fn can_run(&self) -> bool {
        self.step == ExportStep::Form && (self.encrypt || self.understood)
    }

    /// Checks the form, builds the file off the UI thread, then asks where
    /// to save it.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_run() {
            return;
        }
        let password = if self.encrypt {
            let first = self.password.read(cx).value().to_string();
            let second = self.repeat.read(cx).value().to_string();
            self.password_error =
                (chars(&first) < MIN_PASSWORD_CHARS).then_some(strings::PASSWORD_TOO_SHORT);
            self.repeat_error = (self.password_error.is_none() && first != second)
                .then_some(strings::PASSWORD_MISMATCH);
            if self.password_error.is_some() || self.repeat_error.is_some() {
                cx.notify();
                return;
            }
            Some(SecretString::from(first))
        } else {
            None
        };
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.step = ExportStep::Running;
        self.error = None;
        cx.notify();
        let built =
            cx.background_spawn(async move { core.data_export(DataExportInput { password }) });
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

    fn back(&mut self, cx: &mut Context<Self>) {
        self.step = ExportStep::Form;
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

/// Writes `text` to `path`, readable by the owner only on Unix.
fn write_private(path: &Path, text: &str) -> Result<(), ()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| ())?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|_| ())?;
    file.sync_all().map_err(|_| ())
}

impl Render for ExportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let running = self.step == ExportStep::Running;
        let mut body = v_flex().w_full().gap(metrics::SPACE_3).child(
            v_flex()
                .gap(metrics::SPACE_1)
                .child(
                    Switch::new("data-export-encrypt")
                        .checked(self.encrypt)
                        .label(strings::EXPORT_ENCRYPT)
                        .disabled(running)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_encrypt(*checked, cx);
                        })),
                )
                .child(widgets::caption(strings::EXPORT_ENCRYPT_HINT, cx)),
        );
        if self.encrypt {
            body = body
                .child(crate::lock::password_field(
                    strings::EXPORT_PASSWORD,
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
                    strings::EXPORT_REPEAT,
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
        } else {
            body = body
                .child(
                    div()
                        .id("data-export-warning")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.danger)
                        .child(strings::EXPORT_PLAIN_WARNING),
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
    let running = dialog.read(cx).step == ExportStep::Running;
    let enabled = dialog.read(cx).can_run();
    let run = dialog.downgrade();
    h_flex()
        .w_full()
        .justify_end()
        .gap(metrics::SPACE_2)
        .child(
            ControlButton::new("data-export-cancel", ControlKind::Neutral)
                .label(strings::CANCEL)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .child(
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
            .w(px(DIALOG_WIDTH))
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
    File { error: Option<&'static str> },
    Password { error: Option<&'static str> },
    Running,
    Summary(DataImportSummary),
}

pub struct ImportDialog {
    state: Entity<AppState>,
    path: Entity<InputState>,
    password: Entity<InputState>,
    apply_settings: bool,
    /// The content of an encrypted file while its password is asked.
    text: Option<SecretString>,
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

/// Reads an export file within the limit; `Err` is the message.
fn read_file(path: &Path) -> Result<String, &'static str> {
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
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| strings::FILE_NOT_EXPORT)?;
    Ok(text)
}

/// Whether a file's JSON names the encrypted format; `None` when it is not
/// an export at all.
fn encrypted(text: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    match value.get("format")?.as_str()? {
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
            text: None,
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

    /// The primary button: reads the file (File step) or imports with the
    /// typed password (Password step).
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            ImportStep::File { .. } => self.read(window, cx),
            ImportStep::Password { .. } => {
                let password = self.password.read(cx).value().to_string();
                if password.is_empty() {
                    return;
                }
                if let Some(text) = self.text.clone() {
                    self.import(text, Some(SecretString::from(password)), window, cx);
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
            let encrypted = encrypted(&text).ok_or(strings::FILE_NOT_EXPORT)?;
            Ok::<_, &'static str>((SecretString::from(text), encrypted))
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
                Ok((text, false)) => dialog.import(text, None, window, cx),
                Err(error) => {
                    dialog.step = ImportStep::File { error: Some(error) };
                    dialog.task = None;
                    cx.notify();
                }
            });
        }));
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
        let apply_settings = self.apply_settings;
        self.step = ImportStep::Running;
        cx.notify();
        let work = cx.background_spawn(async move {
            core.data_import(DataImportInput {
                text,
                password,
                apply_settings,
            })
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                dialog.step = match result {
                    Ok(summary) => {
                        dialog.text = None;
                        ImportStep::Summary(summary)
                    }
                    Err(CoreError::Validation(errors))
                        if encrypted_file
                            && errors.iter().any(|error| {
                                error.field == "password"
                                    && error.reason == ValidationReason::Invalid
                            }) =>
                    {
                        ImportStep::Password {
                            error: Some(strings::PASSWORD_WRONG),
                        }
                    }
                    Err(CoreError::Validation(_)) => {
                        dialog.text = None;
                        ImportStep::File {
                            error: Some(strings::FILE_NOT_EXPORT),
                        }
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "data not imported");
                        dialog.text = None;
                        ImportStep::File {
                            error: Some(strings::IMPORT_FAILED),
                        }
                    }
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
                .h(px(28.))
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
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = match &self.step {
            ImportStep::Summary(summary) => self.render_summary(summary, cx).into_any_element(),
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
                        .checked(self.apply_settings)
                        .label(strings::IMPORT_SETTINGS)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_apply_settings(*checked, cx)
                        })),
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
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading))
            .child(shown.clone())
            .footer(import_footer(&shown, cx))
    });
    dialog
}
