//! The import dialog of the Hosts screen (UI 2.5; New host ▸ Import and the
//! empty state's "Import from ~/.ssh/config"; night decision D39): the
//! source, then for `~/.ssh/config` the file, the preview with a checkbox
//! per new host and the passphrases of encrypted key files, and the summary.
//!
//! - Sources: the OpenSSH config (selected), a Shell++ export (goes on in
//!   Import Data, D38), known_hosts (goes on in the Known Hosts import),
//!   PuTTY sessions (Windows only; D140) and CSV (D141, [`csv_step`]: the
//!   file, a column mapping, the preview and the summary).
//! - PuTTY sessions are read from the registry ([`putty_registry`]), so
//!   their file step is skipped: Next goes from the sources to the same
//!   preview (a session is a host, its `.ppk` a key file) and summary.
//! - The suggested file is `~/.ssh/config`; tests and the demo set another
//!   one with [`set_default_path`] (never the real file in a test run).
//! - The preview comes from the core (`ssh_config_preview`), off the UI
//!   thread: each host with its status (new, already exists, not imported
//!   with the reason) and the options left out; the key files with theirs;
//!   the lines that could not be read. New hosts are checked by default.
//! - Import runs off the UI thread (`ssh_config_import`) and ends with
//!   "added, skipped, errors" and the keys.
//!
//! Nothing here logs paths, host names or passphrases.

pub mod csv_step;
pub mod putty_registry;
pub mod strings;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, ClickEvent, Context, ElementId, Entity, FontWeight, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use secrecy::SecretString;
use shellpp_core::service::dto::{
    CsvImportSummary, ImportStatus, KeyFileStatus, PuttyImportInput, SshConfigImportInput,
    SshConfigImportSummary, SshConfigPreview, SshConfigSource,
};

use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// Width of the dialog (a preview needs more room than a file step).
const DIALOG_WIDTH: f32 = 560.;
/// Height of the preview's scroll area.
const PREVIEW_HEIGHT: f32 = 360.;

/// The suggested OpenSSH config (UI 2.5 step 2), overridable for tests and
/// the demo.
#[derive(Clone, Debug, Default)]
pub struct DefaultPath(pub Option<PathBuf>);

impl Global for DefaultPath {}

/// Overrides the suggested config file for this app.
pub fn set_default_path(path: Option<PathBuf>, cx: &mut App) {
    cx.set_global(DefaultPath(path));
}

/// The override of [`set_default_path`], else nothing in a test run, else
/// `~/.ssh/config`.
pub fn default_path(cx: &App) -> Option<PathBuf> {
    if let Some(path) = cx.try_global::<DefaultPath>() {
        return path.0.clone();
    }
    if crate::dirs::is_test_run() {
        return None;
    }
    ::dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

/// The home folder for `~` in a config at `path`: the parent of its `.ssh`
/// folder, else the user's home.
fn home_for(path: &Path) -> PathBuf {
    match path.parent() {
        Some(dir) if dir.file_name().is_some_and(|name| name == ".ssh") => {
            dir.parent().map(Path::to_path_buf).unwrap_or_default()
        }
        _ => ::dirs::home_dir().unwrap_or_default(),
    }
}

/// `path` with the config's home folder written as `~` (`~/.ssh/id_web`),
/// so a key file fits on its row; other paths as they are.
fn tilde(path: &str, home: &Path) -> String {
    let home = home.display().to_string();
    match path.strip_prefix(home.as_str()) {
        Some(rest) if !home.is_empty() && (rest.starts_with('/') || rest.starts_with('\\')) => {
            format!("~{}", rest.replace('\\', "/"))
        }
        _ => path.to_owned(),
    }
}

/// The import sources of UI 2.5 step 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    SshConfig,
    Json,
    KnownHosts,
    Putty,
    Csv,
}

impl Source {
    const ALL: [Self; 5] = [
        Self::SshConfig,
        Self::Json,
        Self::KnownHosts,
        Self::Putty,
        Self::Csv,
    ];

    fn id(self) -> &'static str {
        match self {
            Self::SshConfig => "import-source-ssh-config",
            Self::Json => "import-source-json",
            Self::KnownHosts => "import-source-known-hosts",
            Self::Putty => "import-source-putty",
            Self::Csv => "import-source-csv",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SshConfig => strings::SOURCE_SSH_CONFIG,
            Self::Json => strings::SOURCE_JSON,
            Self::KnownHosts => strings::SOURCE_KNOWN_HOSTS,
            Self::Putty => strings::SOURCE_PUTTY,
            Self::Csv => strings::SOURCE_CSV,
        }
    }

    /// The file the source reads by default, shown under its name in the
    /// monospaced font (the name itself is plain words).
    fn path(self) -> Option<&'static str> {
        match self {
            Self::SshConfig => Some(strings::SOURCE_SSH_CONFIG_PATH),
            Self::KnownHosts => Some(strings::SOURCE_KNOWN_HOSTS_PATH),
            Self::Json | Self::Putty | Self::Csv => None,
        }
    }

    /// PuTTY sessions exist on Windows only (UI 2.5).
    fn shown(self) -> bool {
        self != Self::Putty || cfg!(windows)
    }
}

/// Where the dialog is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Source,
    File {
        error: Option<&'static str>,
    },
    /// Reading the preview or importing.
    Running,
    /// The column mapping of a CSV file (D141).
    Mapping,
    Preview,
    Summary(SshConfigImportSummary),
    CsvSummary(CsvImportSummary),
}

pub struct ImportSourcesDialog {
    state: Entity<AppState>,
    source: Source,
    path: Entity<InputState>,
    /// The CSV file field (no file is suggested).
    csv_path: Entity<InputState>,
    /// The CSV file and its mapping, once read.
    csv: Option<csv_step::CsvState>,
    step: Step,
    preview: Option<SshConfigPreview>,
    /// Aliases (lowercase) of the new hosts that are checked.
    chosen: HashSet<String>,
    /// The passphrase field of each encrypted key file, by its path.
    passphrases: Vec<(String, Entity<InputState>)>,
    save_passphrases: bool,
    /// The preview scrolls (with a scrollbar) when it is taller than
    /// [`PREVIEW_HEIGHT`].
    preview_scroll: ScrollHandle,
    /// Why the PuTTY sessions could not be read or imported.
    putty_error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl std::fmt::Debug for ImportSourcesDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportSourcesDialog")
            .field("source", &self.source)
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

impl ImportSourcesDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value = default_path(cx)
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let path = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let csv_path = cx.new(|cx| InputState::new(window, cx));
        Self {
            state,
            source: Source::SshConfig,
            path,
            csv_path,
            csv: None,
            step: Step::Source,
            preview: None,
            chosen: HashSet::new(),
            passphrases: Vec::new(),
            save_passphrases: false,
            preview_scroll: ScrollHandle::new(),
            putty_error: None,
            task: None,
        }
    }

    pub fn step(&self) -> &Step {
        &self.step
    }

    pub fn preview(&self) -> Option<&SshConfigPreview> {
        self.preview.as_ref()
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn select(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        self.putty_error = None;
        cx.notify();
    }

    /// The file field of the current source.
    fn path_input(&self) -> &Entity<InputState> {
        match self.source {
            Source::Csv => &self.csv_path,
            _ => &self.path,
        }
    }

    pub fn set_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = path.display().to_string();
        self.path_input()
            .clone()
            .update(cx, |input, cx| input.set_value(text, window, cx));
        if let Step::File { error } = &mut self.step {
            *error = None;
        }
        cx.notify();
    }

    /// Checks or unchecks a new host of the preview.
    pub fn set_chosen(&mut self, alias: &str, chosen: bool, cx: &mut Context<Self>) {
        let alias = alias.to_lowercase();
        if chosen {
            self.chosen.insert(alias);
        } else {
            self.chosen.remove(&alias);
        }
        cx.notify();
    }

    /// Fills the passphrase field of the key file `path` (tests).
    pub fn set_passphrase(
        &mut self,
        path: &str,
        passphrase: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = passphrase.to_owned();
        if let Some((_, input)) = self.passphrases.iter().find(|(key, _)| key == path) {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    pub fn set_save_passphrases(&mut self, save: bool, cx: &mut Context<Self>) {
        self.save_passphrases = save;
        cx.notify();
    }

    fn typed_path(&self, cx: &App) -> PathBuf {
        PathBuf::from(self.path.read(cx).value().trim())
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
            if let Some(path) = paths.into_iter().next() {
                let _ = this.update_in(cx, |dialog, window, cx| dialog.set_path(&path, window, cx));
            }
        })
        .detach();
    }

    /// The primary button of the current step.
    pub fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            Step::Source => match self.source {
                Source::SshConfig => {
                    self.step = Step::File { error: None };
                    cx.notify();
                }
                Source::Json => {
                    let state = self.state.clone();
                    window.close_dialog(cx);
                    crate::data_transfer::open_import(state, window, cx);
                }
                Source::KnownHosts => {
                    let state = self.state.clone();
                    window.close_dialog(cx);
                    crate::known_hosts::open_import_dialog(state, window, cx);
                }
                Source::Putty => self.read_putty_preview(window, cx),
                Source::Csv => {
                    self.step = Step::File { error: None };
                    cx.notify();
                }
            },
            Step::File { .. } if self.source == Source::Csv => self.read_csv(window, cx),
            Step::File { .. } => self.read_preview(window, cx),
            Step::Mapping => self.read_csv_preview(window, cx),
            Step::Preview if self.source == Source::Csv => self.import_csv(window, cx),
            Step::Preview => self.import(window, cx),
            Step::Running | Step::Summary(_) | Step::CsvSummary(_) => {}
        }
    }

    /// "Back": from the file step to the sources, from the preview to the
    /// file step (to the sources for PuTTY, which has none; to the mapping
    /// for CSV), from the mapping to the file step.
    pub fn back(&mut self, cx: &mut Context<Self>) {
        if matches!(self.step, Step::File { .. } | Step::Mapping | Step::Preview) {
            self.step = match (&self.step, self.source) {
                (Step::Preview, Source::Csv) => Step::Mapping,
                (Step::Preview, Source::Putty) => Step::Source,
                (Step::Preview | Step::Mapping, _) => Step::File { error: None },
                _ => Step::Source,
            };
            self.putty_error = None;
            cx.notify();
        }
    }

    /// The PuTTY error shown under the sources: the registry could not be
    /// read, or the import failed.
    pub fn putty_error(&self) -> Option<&'static str> {
        self.putty_error
    }

    /// Reads the saved PuTTY sessions into the preview, off the UI thread.
    fn read_putty_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let source = putty_registry::source(cx);
        self.putty_error = None;
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.putty_preview(&*source) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.task = None;
                match result {
                    Ok(preview) => dialog.show_preview(preview, window, cx),
                    Err(error) => {
                        tracing::info!(code = error.code(), "putty sessions not read");
                        dialog.putty_error = Some(strings::PUTTY_READ_FAILED);
                        dialog.step = Step::Source;
                    }
                }
                cx.notify();
            });
        }));
    }

    fn read_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.typed_path(cx);
        if path.as_os_str().is_empty() {
            self.step = Step::File {
                error: Some(strings::FILE_MISSING),
            };
            cx.notify();
            return;
        }
        if !path.is_file() {
            self.step = Step::File {
                error: Some(strings::FILE_NOT_FOUND),
            };
            cx.notify();
            return;
        }
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let source = SshConfigSource {
            home: home_for(&path),
            path,
        };
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.ssh_config_preview(source) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.task = None;
                match result {
                    Ok(preview) => dialog.show_preview(preview, window, cx),
                    Err(error) => {
                        tracing::info!(code = error.code(), "ssh config not read");
                        dialog.step = Step::File {
                            error: Some(strings::READ_FAILED),
                        };
                    }
                }
                cx.notify();
            });
        }));
    }

    fn show_preview(
        &mut self,
        preview: SshConfigPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chosen = preview
            .hosts
            .iter()
            .filter(|host| host.status == ImportStatus::New)
            .map(|host| host.alias.to_lowercase())
            .collect();
        self.passphrases = preview
            .keys
            .iter()
            .filter(|key| key.status == KeyFileStatus::NeedsPassphrase)
            .map(|key| {
                let input = cx.new(|cx| InputState::new(window, cx).masked(true));
                (key.path.clone(), input)
            })
            .collect();
        self.preview = Some(preview);
        self.step = Step::Preview;
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let path = self.typed_path(cx);
        let passphrases: BTreeMap<String, SecretString> = self
            .passphrases
            .iter()
            .filter_map(|(key, input)| {
                let value = input.read(cx).value().to_string();
                (!value.is_empty()).then(|| (key.clone(), SecretString::from(value)))
            })
            .collect();
        if self.source == Source::Putty {
            let source = putty_registry::source(cx);
            let input = PuttyImportInput {
                sessions: Some(self.chosen.iter().cloned().collect()),
                passphrases,
                save_passphrases: self.save_passphrases,
            };
            self.step = Step::Running;
            cx.notify();
            let work = cx.background_spawn(async move { core.putty_import(&*source, input) });
            self.task = Some(cx.spawn_in(window, async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |dialog, cx| {
                    dialog.task = None;
                    dialog.step = match result {
                        Ok(summary) => Step::Summary(summary),
                        Err(error) => {
                            tracing::warn!(code = error.code(), "putty sessions not imported");
                            dialog.putty_error = Some(strings::IMPORT_FAILED);
                            Step::Source
                        }
                    };
                    cx.notify();
                });
            }));
            return;
        }
        let input = SshConfigImportInput {
            source: SshConfigSource {
                home: home_for(&path),
                path,
            },
            aliases: Some(self.chosen.iter().cloned().collect()),
            passphrases,
            save_passphrases: self.save_passphrases,
        };
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.ssh_config_import(input) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                dialog.step = match result {
                    Ok(summary) => Step::Summary(summary),
                    Err(error) => {
                        tracing::warn!(code = error.code(), "ssh config not imported");
                        Step::File {
                            error: Some(strings::IMPORT_FAILED),
                        }
                    }
                };
                cx.notify();
            });
        }));
    }

    fn render_sources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex()
            .id("import-sources")
            .test_support()
            .gap(metrics::SPACE_2);
        for source in Source::ALL.into_iter().filter(|source| source.shown()) {
            let row = v_flex()
                .gap(metrics::SPACE_1)
                .child(
                    Radio::new(source.id())
                        .label(source.label())
                        .checked(self.source == source)
                        .on_click(
                            cx.listener(move |this, _: &bool, _, cx| this.select(source, cx)),
                        ),
                )
                .children(source.path().map(|path| {
                    div()
                        .pl(metrics::rpx(24.))
                        .child(widgets::caption(path, cx))
                }));
            let error = self.putty_error.filter(|_| source == Source::Putty);
            list = list.child(if let Some(error) = error {
                row.child(
                    div()
                        .id("import-putty-error")
                        .test_support()
                        .pl(metrics::rpx(24.))
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            } else {
                row
            });
        }
        list
    }

    fn render_file(
        &self,
        error: Option<&'static str>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::field(
                strings::FILE,
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .child(div().flex_1().min_w_0().child(widgets::text_input(
                        self.path_input(),
                        "import-path",
                        error.is_some(),
                        window,
                        cx,
                    )))
                    .child(
                        ControlButton::new("import-browse", ControlKind::Neutral)
                            .label(strings::BROWSE)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.browse(window, cx)
                            })),
                    ),
                error,
                cx,
            ))
            .map(|this| {
                if self.source == Source::Csv {
                    this.child(self.render_csv_file_extras(cx).into_any_element())
                } else {
                    this.child(widgets::caption(strings::FILE_HINT, cx))
                }
            })
    }

    fn render_preview(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(preview) = &self.preview else {
            return div().into_any_element();
        };
        let new_hosts = preview
            .hosts
            .iter()
            .filter(|host| host.status == ImportStatus::New)
            .count();
        let mut hosts = v_flex().gap(metrics::SPACE_2);
        if preview.hosts.is_empty() {
            hosts = hosts.child(widgets::caption(
                match self.source {
                    Source::Putty => strings::PREVIEW_EMPTY_PUTTY,
                    _ => strings::PREVIEW_EMPTY,
                },
                cx,
            ));
        }
        for host in &preview.hosts {
            let alias = host.alias.clone();
            let status = match host.status {
                ImportStatus::New => strings::STATUS_NEW,
                ImportStatus::Exists => strings::STATUS_EXISTS,
                ImportStatus::Error => strings::STATUS_ERROR,
            };
            let mut target = host.address.clone();
            if let Some(user) = &host.username {
                target = format!("{user}@{target}");
            }
            if let Some(port) = host.port.filter(|port| *port != 22) {
                target = format!("{target}:{port}");
            }
            let check =
                Checkbox::new(ElementId::Name(
                    format!("import-host-{}", alias.to_lowercase()).into(),
                ))
                .label(SharedString::from(format!("{alias}  ·  {target}")))
                .checked(self.chosen.contains(&alias.to_lowercase()))
                .disabled(host.status != ImportStatus::New)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.set_chosen(&alias, *checked, cx)
                }));
            let mut row = v_flex().gap(metrics::rpx(2.)).child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap(metrics::SPACE_2)
                    .child(div().flex_1().min_w_0().child(check))
                    .child(
                        div()
                            .flex_none()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(match host.status {
                                ImportStatus::Error => theme.danger,
                                _ => theme.muted_foreground,
                            })
                            .child(status),
                    ),
            );
            for note in host.reason.iter().chain(host.notes.iter()) {
                row = row.child(
                    div()
                        .pl(metrics::rpx(24.))
                        .child(widgets::caption(note.clone(), cx)),
                );
            }
            hosts = hosts.child(row);
        }
        let mut body = v_flex()
            .gap(metrics::SPACE_3)
            .child(section(strings::PREVIEW_HOSTS, cx))
            .child(widgets::caption(
                strings::selected(self.chosen.len(), new_hosts),
                cx,
            ))
            .child(hosts);
        if preview.templates > 0 {
            body = body.child(widgets::caption(strings::templates(preview.templates), cx));
        }
        if !preview.keys.is_empty() {
            let home = home_for(Path::new(self.path.read(cx).value().as_ref()));
            let mut keys = v_flex().gap(metrics::SPACE_2);
            for key in &preview.keys {
                let status = match &key.status {
                    KeyFileStatus::New => strings::KEY_NEW.to_owned(),
                    KeyFileStatus::Exists => strings::KEY_EXISTS.to_owned(),
                    KeyFileStatus::NeedsPassphrase => strings::KEY_PASSPHRASE.to_owned(),
                    KeyFileStatus::Missing => strings::KEY_MISSING.to_owned(),
                    KeyFileStatus::Invalid(reason) => reason.clone(),
                };
                let mut row = v_flex().gap(metrics::rpx(2.)).child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap(metrics::SPACE_2)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .child(tilde(&key.path, &home)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(metrics::FONT_SIZE_CAPTION)
                                .text_color(theme.muted_foreground)
                                .child(status),
                        ),
                );
                if let Some(index) = self
                    .passphrases
                    .iter()
                    .position(|(path, _)| path == &key.path)
                {
                    let (_, input) = &self.passphrases[index];
                    row = row.child(crate::lock::password_field(
                        strings::PASSPHRASE,
                        input,
                        passphrase_id(index),
                        false,
                        false,
                        window,
                        cx,
                    ));
                }
                keys = keys.child(row);
            }
            body = body.child(section(strings::PREVIEW_KEYS, cx)).child(keys);
            if !self.passphrases.is_empty() {
                body = body.child(
                    Checkbox::new("import-save-passphrases")
                        .label(strings::SAVE_PASSPHRASES)
                        .checked(self.save_passphrases)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.set_save_passphrases(*checked, cx)
                        })),
                );
            }
        }
        if !preview.problems.is_empty() {
            let mut problems = v_flex().gap(metrics::rpx(2.));
            for problem in &preview.problems {
                problems = problems.child(widgets::caption(problem.clone(), cx));
            }
            body = body
                .child(section(strings::PREVIEW_PROBLEMS, cx))
                .child(problems);
        }
        div()
            .relative()
            .w_full()
            .child(
                v_flex()
                    .id("import-preview")
                    .test_support()
                    .w_full()
                    .max_h(metrics::rpx(PREVIEW_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&self.preview_scroll)
                    .pr(metrics::SPACE_3)
                    .child(body),
            )
            .child(Scrollbar::vertical(&self.preview_scroll))
            .into_any_element()
    }

    fn render_summary(&self, summary: &SshConfigImportSummary, cx: &App) -> impl IntoElement {
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
        let mut errors = v_flex().gap(metrics::rpx(2.));
        for (alias, reason) in &summary.errors {
            errors = errors.child(widgets::caption(format!("{alias}: {reason}"), cx));
        }
        v_flex()
            .id("import-summary")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(row("import-added", strings::SUMMARY_ADDED, summary.added))
            .child(row(
                "import-skipped",
                strings::SUMMARY_SKIPPED,
                summary.skipped,
            ))
            .child(row(
                "import-errors",
                strings::SUMMARY_ERRORS,
                summary.errors.len(),
            ))
            .child(errors)
            .child(row(
                "import-keys-added",
                strings::SUMMARY_KEYS_ADDED,
                summary.keys_added,
            ))
            .child(row(
                "import-keys-existing",
                strings::SUMMARY_KEYS_EXISTING,
                summary.keys_existing,
            ))
            .child(row(
                "import-keys-skipped",
                strings::SUMMARY_KEYS_SKIPPED,
                summary.keys_skipped,
            ))
            // Only when they happened: hosts left without their jump hosts
            // (their notes say why) and the commands of the file.
            .when(summary.jump_chains_left_out > 0, |this| {
                this.child(row(
                    "import-jump-left-out",
                    strings::SUMMARY_JUMP_LEFT_OUT,
                    summary.jump_chains_left_out,
                ))
            })
            .when(summary.commands_skipped > 0, |this| {
                this.child(row(
                    "import-commands-skipped",
                    strings::SUMMARY_COMMANDS_SKIPPED,
                    summary.commands_skipped,
                ))
            })
    }
}

/// The element id of the passphrase field of the `index`-th encrypted key.
fn passphrase_id(index: usize) -> &'static str {
    const IDS: [&str; 8] = [
        "import-passphrase-0",
        "import-passphrase-1",
        "import-passphrase-2",
        "import-passphrase-3",
        "import-passphrase-4",
        "import-passphrase-5",
        "import-passphrase-6",
        "import-passphrase-7",
    ];
    IDS.get(index).copied().unwrap_or("import-passphrase-more")
}

fn section(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().foreground)
        .child(text)
}

impl Render for ImportSourcesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.step.clone() {
            Step::Source => self.render_sources(cx).into_any_element(),
            Step::File { error } => self.render_file(error, window, cx).into_any_element(),
            Step::Mapping => self.render_csv_mapping(cx).into_any_element(),
            Step::Preview if self.source == Source::Csv => {
                self.render_csv_preview(cx).into_any_element()
            }
            Step::Preview => self.render_preview(window, cx).into_any_element(),
            Step::Summary(summary) => self.render_summary(&summary, cx).into_any_element(),
            Step::CsvSummary(summary) => self.render_csv_summary(&summary, cx).into_any_element(),
            Step::Running => div()
                .id("import-running")
                .test_support()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(cx.theme().muted_foreground)
                .child(strings::READING)
                .into_any_element(),
        };
        div()
            .id("import-dialog")
            .test_support()
            .w_full()
            .child(body)
    }
}

fn footer(dialog: &Entity<ImportSourcesDialog>, cx: &App) -> impl IntoElement {
    let step = dialog.read(cx).step.clone();
    let no_address = dialog.read(cx).csv.as_ref().is_some_and(|csv| {
        !csv.roles
            .contains(&shellpp_core::import::csv::Role::Address)
    });
    let next = dialog.downgrade();
    let back = dialog.downgrade();
    let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
    if let Step::Summary(_) | Step::CsvSummary(_) = step {
        return row.child(
            ControlButton::new("import-close", ControlKind::Accent)
                .label(strings::CLOSE)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        );
    }
    let running = step == Step::Running;
    let label = match step {
        Step::Preview => strings::IMPORT,
        Step::Running => strings::IMPORTING,
        _ => strings::NEXT,
    };
    let disabled = running || (step == Step::Mapping && no_address);
    row.child(
        ControlButton::new("import-cancel", ControlKind::Neutral)
            .label(strings::CANCEL)
            .on_click(|_, window, cx| window.close_dialog(cx)),
    )
    .when(
        matches!(step, Step::File { .. } | Step::Mapping | Step::Preview),
        |row| {
            row.child(
                ControlButton::new("import-back", ControlKind::Neutral)
                    .label(strings::BACK)
                    .on_click(move |_, _, cx| {
                        let _ = back.update(cx, |dialog, cx| dialog.back(cx));
                    }),
            )
        },
    )
    .child(
        ControlButton::new("import-next", ControlKind::Accent)
            .label(label)
            .disabled(disabled)
            .on_click(move |_, window, cx| {
                let _ = next.update(cx, |dialog, cx| dialog.next(window, cx));
            }),
    )
}

/// Opens the import dialog of UI 2.5 over the current window.
pub fn open(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ImportSourcesDialog> {
    let dialog = cx.new(|cx| ImportSourcesDialog::new(state, window, cx));
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let heading = match shown.read(cx).step {
            Step::Summary(_) | Step::CsvSummary(_) => strings::SUMMARY_TITLE,
            _ => strings::TITLE,
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(heading),
            )
            .child(shown.clone())
            .footer(footer(&shown, cx))
    });
    dialog
}
