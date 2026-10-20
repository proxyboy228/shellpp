//! The import dialog of the Known Hosts screen (UI 2.5 with the known_hosts
//! source, 10.1): a file path (the suggested file filled in, a button to
//! choose another), then the summary of four numbers (imported, duplicates,
//! skipped, invalid lines). The file is read and parsed off the UI thread;
//! files over 16 MiB are refused before they are read in full. Nothing here
//! logs the path or the file's content.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _, Task, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::CoreError;
use shellpp_core::known_hosts::openssh::MAX_IMPORT_BYTES;
use shellpp_core::service::dto::KnownHostsImportSummary;

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// Width of the import dialog (a path needs more room than the 420 px of
/// the small dialogs; on the 4 px grid).
const DIALOG_WIDTH: f32 = 480.;

/// Where the dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportStep {
    /// The file step, with the reason of the last failed attempt.
    File { error: Option<&'static str> },
    /// Reading and importing.
    Running,
    /// Done: the four numbers.
    Summary(KnownHostsImportSummary),
}

pub struct ImportDialog {
    state: Entity<AppState>,
    path: Entity<InputState>,
    step: ImportStep,
    _subscription: gpui::Subscription,
    task: Option<Task<()>>,
}

impl std::fmt::Debug for ImportDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportDialog")
            .field("step", &self.step)
            .finish_non_exhaustive()
    }
}

/// `~` and `~/…` (or `~\…`) in a typed path stand for the home directory.
fn expand_home(text: &str) -> PathBuf {
    let rest = if text == "~" {
        Some("")
    } else {
        text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\"))
    };
    match (rest, ::dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(text),
    }
}

/// Reads a known_hosts file of at most 16 MiB.
fn read_file(path: &Path) -> Result<Vec<u8>, &'static str> {
    let file = std::fs::File::open(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => strings::FILE_NOT_FOUND,
        _ => strings::FILE_UNREADABLE,
    })?;
    let metadata = file.metadata().map_err(|_| strings::FILE_UNREADABLE)?;
    if !metadata.is_file() {
        return Err(strings::FILE_UNREADABLE);
    }
    if metadata.len() > MAX_IMPORT_BYTES as u64 {
        return Err(strings::FILE_TOO_LARGE);
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| strings::FILE_UNREADABLE)?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(strings::FILE_TOO_LARGE);
    }
    Ok(bytes)
}

impl ImportDialog {
    pub fn new(
        state: Entity<AppState>,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = path
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let path = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let subscription = cx.subscribe_in(
            &path,
            window,
            |this: &mut Self, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.run(window, cx),
                InputEvent::Change => {
                    if let ImportStep::File { error: Some(_) } = this.step {
                        this.step = ImportStep::File { error: None };
                        cx.notify();
                    }
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        Self {
            state,
            path,
            step: ImportStep::File { error: None },
            _subscription: subscription,
            task: None,
        }
    }

    pub fn step(&self) -> ImportStep {
        self.step
    }

    /// The path in the field.
    pub fn path_text(&self, cx: &App) -> String {
        self.path.read(cx).value().to_string()
    }

    pub fn set_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = path.display().to_string();
        self.path
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.step = ImportStep::File { error: None };
        cx.notify();
    }

    /// "Choose file": the system open dialog.
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

    /// Import: reads the file and imports it off the UI thread.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.step, ImportStep::File { .. }) {
            return;
        }
        let text = self.path_text(cx).trim().to_owned();
        if text.is_empty() {
            self.step = ImportStep::File {
                error: Some(strings::FILE_MISSING),
            };
            cx.notify();
            return;
        }
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let path = expand_home(&text);
        self.step = ImportStep::Running;
        cx.notify();
        let work = cx.background_spawn(async move {
            let bytes = read_file(&path)?;
            core.known_hosts_import(&bytes)
                .map_err(|error| match error {
                    CoreError::Validation(_) => strings::FILE_TOO_LARGE,
                    other => {
                        tracing::warn!(code = other.code(), "known_hosts import failed");
                        strings::IMPORT_FAILED
                    }
                })
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.step = match result {
                    Ok(summary) => ImportStep::Summary(summary),
                    Err(error) => {
                        tracing::info!("known_hosts file not imported");
                        ImportStep::File { error: Some(error) }
                    }
                };
                dialog.task = None;
                cx.notify();
            });
        }));
    }

    fn render_summary(&self, summary: KnownHostsImportSummary, cx: &App) -> impl IntoElement {
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
            .id("kh-import-summary")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(row(
                "kh-import-imported",
                strings::SUMMARY_IMPORTED,
                summary.imported,
            ))
            .child(row(
                "kh-import-duplicates",
                strings::SUMMARY_DUPLICATES,
                summary.duplicates,
            ))
            .child(row(
                "kh-import-skipped",
                strings::SUMMARY_SKIPPED,
                summary.skipped,
            ))
            .child(row(
                "kh-import-invalid",
                strings::SUMMARY_INVALID,
                summary.invalid,
            ))
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = match self.step {
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
                            "kh-import-path",
                            error.is_some(),
                            window,
                            cx,
                        )))
                        .child(
                            ControlButton::new("kh-import-browse", ControlKind::Neutral)
                                .label(strings::IMPORT_BROWSE)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.browse(window, cx)
                                })),
                        ),
                    error,
                    cx,
                ))
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(strings::IMPORT_HINT),
                )
                .into_any_element(),
            ImportStep::Running => div()
                .id("kh-import-running")
                .test_support()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.muted_foreground)
                .child(strings::IMPORT_RUNNING)
                .into_any_element(),
        };
        div()
            .id("kh-import-dialog")
            .test_support()
            .w_full()
            .child(body)
    }
}

/// The footer for the dialog's current step.
fn footer(dialog: &Entity<ImportDialog>, cx: &App) -> impl IntoElement {
    let step = dialog.read(cx).step;
    let run = dialog.downgrade();
    let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
    match step {
        ImportStep::Summary(_) => row.child(
            ControlButton::new("kh-import-close", ControlKind::Accent)
                .label(strings::CLOSE)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        ),
        ImportStep::File { .. } | ImportStep::Running => {
            let running = step == ImportStep::Running;
            row.child(
                ControlButton::new("kh-import-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                ControlButton::new("kh-import-run", ControlKind::Accent)
                    .label(if running {
                        strings::IMPORT_RUNNING
                    } else {
                        strings::IMPORT_RUN
                    })
                    .disabled(running)
                    .on_click(move |_, window, cx| {
                        let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                    }),
            )
        }
    }
}

/// Opens `dialog` as a modal (title, body, footer by step); Esc closes it.
pub(super) fn open(dialog: Entity<ImportDialog>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |modal, _, cx| {
        let title = match dialog.read(cx).step {
            ImportStep::Summary(_) => strings::SUMMARY_TITLE,
            _ => strings::IMPORT_TITLE,
        };
        modal
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(dialog.clone())
            .footer(footer(&dialog, cx))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_expanded_in_typed_paths() {
        if let Some(home) = ::dirs::home_dir() {
            assert_eq!(expand_home("~"), home);
            assert_eq!(
                expand_home("~/x/known_hosts"),
                home.join("x").join("known_hosts")
            );
        }
        assert_eq!(expand_home("/tmp/kh"), PathBuf::from("/tmp/kh"));
        assert_eq!(expand_home("a~/b"), PathBuf::from("a~/b"));
    }

    #[test]
    fn files_are_read_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small");
        std::fs::write(&small, b"# nothing\n").unwrap();
        assert_eq!(read_file(&small).unwrap(), b"# nothing\n");
        assert_eq!(
            read_file(&dir.path().join("missing")),
            Err(strings::FILE_NOT_FOUND)
        );
        assert_eq!(read_file(dir.path()), Err(strings::FILE_UNREADABLE));
        let large = dir.path().join("large");
        let file = std::fs::File::create(&large).unwrap();
        file.set_len(MAX_IMPORT_BYTES as u64 + 1).unwrap();
        assert_eq!(read_file(&large), Err(strings::FILE_TOO_LARGE));
    }
}
