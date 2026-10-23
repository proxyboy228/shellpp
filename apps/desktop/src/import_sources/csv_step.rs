//! The CSV source of the import dialog (UI 2.5; run 8 decision D141): the
//! file (with "Save template…"), the column mapping, the preview and the
//! summary. The core reads the file (`csv_inspect`, `csv_preview`,
//! `csv_import`) off the UI thread.
//!
//! - Mapping: the character set and the delimiter (detected, changeable),
//!   "First row is a header", and each column with its role and its first
//!   values; a column mapped to `password` shows its values masked and a
//!   warning that the file holds passwords in clear text. Next needs a
//!   column mapped to `address`.
//! - Preview: every row with its status and reason, the groups the import
//!   creates; only new rows are imported.
//!
//! Nothing here logs paths or values.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, ElementId, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme as _, IconName, h_flex, v_flex};
use shellpp_core::CoreError;
use shellpp_core::import::csv::{self, Role};
use shellpp_core::service::dto::{
    CsvImportSummary, CsvInput, CsvInspection, CsvPreview, ImportStatus,
};
use zeroize::Zeroizing;

use super::{ImportSourcesDialog, PREVIEW_HEIGHT, Step, section, strings};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

/// Rows of the preview drawn (the counts cover them all).
const PREVIEW_ROWS: usize = 200;
/// Values of a column shown in the mapping.
const SAMPLE_VALUES: usize = 3;
/// The template's suggested file name.
pub const TEMPLATE_FILE_NAME: &str = "shellpp-hosts.csv";

/// The CSV file and its mapping.
#[derive(Clone)]
pub struct CsvState {
    /// The file, wiped when the dialog drops it (it may hold passwords).
    bytes: Arc<Zeroizing<Vec<u8>>>,
    /// The settings in effect, the columns and the first rows.
    pub inspection: CsvInspection,
    /// The role of each column, as the user mapped it.
    pub roles: Vec<Role>,
    pub preview: Option<CsvPreview>,
}

impl std::fmt::Debug for CsvState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvState")
            .field("roles", &self.roles)
            .finish_non_exhaustive()
    }
}

impl CsvState {
    fn input(&self) -> CsvInput {
        CsvInput {
            bytes: (*self.bytes).clone(),
            encoding: Some(self.inspection.encoding.clone()),
            delimiter: Some(self.inspection.delimiter),
            has_header: Some(self.inspection.has_header),
            roles: Some(self.roles.clone()),
        }
    }
}

/// The name of a role in the mapping's menu.
pub fn role_label(role: Role) -> &'static str {
    match role {
        Role::Ignore => strings::CSV_ROLE_IGNORE,
        Role::Label => "label",
        Role::Address => "address",
        Role::Port => "port",
        Role::Username => "username",
        Role::Group => "group",
        Role::Tags => "tags",
        Role::Notes => "notes",
        Role::Password => "password",
    }
}

/// How a delimiter is shown.
fn delimiter_label(delimiter: char) -> SharedString {
    match delimiter {
        '\t' => strings::CSV_TAB.into(),
        other => format!("{other}").into(),
    }
}

/// Reads at most [`csv::MAX_FILE_BYTES`] of `path`.
fn read_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > csv::MAX_FILE_BYTES as u64 => return Err(strings::CSV_TOO_LARGE),
        Ok(_) => {}
        Err(_) => return Err(strings::FILE_NOT_FOUND),
    }
    std::fs::read(path)
        .map(Zeroizing::new)
        .map_err(|_| strings::READ_FAILED)
}

impl ImportSourcesDialog {
    /// The CSV file and its mapping, once read.
    pub fn csv(&self) -> Option<&CsvState> {
        self.csv.as_ref()
    }

    fn csv_path(&self, cx: &App) -> PathBuf {
        PathBuf::from(self.csv_path.read(cx).value().trim())
    }

    /// The file step's Next: reads the file and detects its settings.
    pub(super) fn read_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.csv_path(cx);
        if path.as_os_str().is_empty() {
            self.step = Step::File {
                error: Some(strings::FILE_MISSING),
            };
            cx.notify();
            return;
        }
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move {
            let bytes = Arc::new(read_file(&path)?);
            let input = CsvInput {
                bytes: (*bytes).clone(),
                ..CsvInput::default()
            };
            match core.csv_inspect(&input) {
                Ok(inspection) => Ok((bytes, inspection)),
                Err(error) => {
                    tracing::info!(code = error.code(), "csv not read");
                    Err(match &error {
                        CoreError::TooLarge => strings::CSV_TOO_LARGE,
                        CoreError::Validation(fields)
                            if fields.iter().any(|field| field.field == "quote") =>
                        {
                            strings::CSV_OPEN_QUOTE
                        }
                        _ => strings::READ_FAILED,
                    })
                }
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                match result {
                    Ok((bytes, inspection)) => {
                        dialog.csv = Some(CsvState {
                            bytes,
                            roles: inspection.roles.clone(),
                            inspection,
                            preview: None,
                        });
                        dialog.step = Step::Mapping;
                    }
                    Err(error) => dialog.step = Step::File { error: Some(error) },
                }
                cx.notify();
            });
        }));
    }

    /// Reads the file again with other settings (the columns may change, so
    /// their roles are guessed again).
    fn reinspect(
        &mut self,
        encoding: Option<String>,
        delimiter: Option<char>,
        has_header: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(csv), Some(core)) = (self.csv.clone(), self.state.read(cx).core().cloned())
        else {
            return;
        };
        let input = CsvInput {
            bytes: (*csv.bytes).clone(),
            encoding: encoding.or(Some(csv.inspection.encoding.clone())),
            delimiter: delimiter.or(Some(csv.inspection.delimiter)),
            has_header: has_header.or(Some(csv.inspection.has_header)),
            roles: None,
        };
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.csv_inspect(&input) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                if let (Ok(inspection), Some(csv)) = (result, dialog.csv.as_mut()) {
                    csv.roles = inspection.roles.clone();
                    csv.inspection = inspection;
                    csv.preview = None;
                }
                dialog.step = Step::Mapping;
                cx.notify();
            });
        }));
    }

    pub fn set_csv_encoding(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reinspect(Some(label.to_owned()), None, None, window, cx);
    }

    pub fn set_csv_delimiter(
        &mut self,
        delimiter: char,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reinspect(None, Some(delimiter), None, window, cx);
    }

    pub fn set_csv_header(&mut self, header: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.reinspect(None, None, Some(header), window, cx);
    }

    /// Maps column `index` to `role`; a role other than "not imported"
    /// leaves the column that had it.
    pub fn set_csv_role(&mut self, index: usize, role: Role, cx: &mut Context<Self>) {
        let Some(csv) = self.csv.as_mut() else {
            return;
        };
        if role != Role::Ignore {
            for other in csv.roles.iter_mut() {
                if *other == role {
                    *other = Role::Ignore;
                }
            }
        }
        if let Some(slot) = csv.roles.get_mut(index) {
            *slot = role;
        }
        csv.preview = None;
        cx.notify();
    }

    /// The mapping's Next: the preview of every row.
    pub(super) fn read_csv_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(csv), Some(core)) = (self.csv.clone(), self.state.read(cx).core().cloned())
        else {
            return;
        };
        if !csv.roles.contains(&Role::Address) {
            return;
        }
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.csv_preview(&csv.input()) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                match result {
                    Ok(preview) => {
                        if let Some(csv) = dialog.csv.as_mut() {
                            csv.preview = Some(preview);
                        }
                        dialog.step = Step::Preview;
                    }
                    Err(error) => {
                        tracing::info!(code = error.code(), "csv preview failed");
                        dialog.step = Step::Mapping;
                    }
                }
                cx.notify();
            });
        }));
    }

    /// The preview's Import.
    pub(super) fn import_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(csv), Some(core)) = (self.csv.clone(), self.state.read(cx).core().cloned())
        else {
            return;
        };
        self.step = Step::Running;
        cx.notify();
        let work = cx.background_spawn(async move { core.csv_import(&csv.input()) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.task = None;
                dialog.step = match result {
                    Ok(summary) => Step::CsvSummary(summary),
                    Err(error) => {
                        tracing::warn!(code = error.code(), "csv not imported");
                        Step::File {
                            error: Some(strings::IMPORT_FAILED),
                        }
                    }
                };
                cx.notify();
            });
        }));
    }

    /// "Save template…": the system save dialog, then [`Self::write_template`].
    pub fn save_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = ::dirs::document_dir()
            .or_else(::dirs::home_dir)
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(TEMPLATE_FILE_NAME));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.write_template(&path, window, cx)
            });
        })
        .detach();
    }

    /// Writes the CSV template to `path` and says how it went in a toast.
    pub fn write_template(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let message = match std::fs::write(path, csv::TEMPLATE) {
            Ok(()) => strings::CSV_TEMPLATE_SAVED,
            Err(_) => {
                tracing::warn!("csv template not written");
                strings::CSV_TEMPLATE_FAILED
            }
        };
        crate::shell::show_toast(message, window, cx);
    }

    /// The template button and hint under the CSV file field.
    pub(super) fn render_csv_file_extras(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(metrics::SPACE_2)
            .child(widgets::caption(strings::CSV_FILE_HINT, cx))
            .child(
                h_flex().child(
                    ControlButton::new("import-csv-template", ControlKind::Neutral)
                        .label(strings::CSV_TEMPLATE)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.save_template(window, cx)
                        })),
                ),
            )
    }

    pub(super) fn render_csv_mapping(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(csv) = &self.csv else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let dialog = cx.entity().downgrade();
        let current_encoding = csv.inspection.encoding.clone();
        let encoding_menu = {
            let dialog = dialog.clone();
            let current = current_encoding.clone();
            ControlButton::new("import-csv-encoding", ControlKind::Neutral)
                .label(current_encoding.clone())
                .trailing_icon(IconName::ChevronDown)
                .dropdown_menu(move |mut menu, _, cx| {
                    menu = menu.min_w(metrics::scaled(180., cx));
                    for label in csv::ENCODINGS {
                        let dialog = dialog.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(current == label)
                                .on_click(move |_, window, cx| {
                                    let _ = dialog.update(cx, |dialog, cx| {
                                        dialog.set_csv_encoding(label, window, cx)
                                    });
                                }),
                        );
                    }
                    menu
                })
        };
        let current_delimiter = csv.inspection.delimiter;
        let delimiter_menu = {
            let dialog = dialog.clone();
            ControlButton::new("import-csv-delimiter", ControlKind::Neutral)
                .label(delimiter_label(current_delimiter))
                .trailing_icon(IconName::ChevronDown)
                .dropdown_menu(move |mut menu, _, cx| {
                    menu = menu.min_w(metrics::scaled(120., cx));
                    for delimiter in csv::DELIMITERS {
                        let dialog = dialog.clone();
                        menu = menu.item(
                            PopupMenuItem::new(delimiter_label(delimiter))
                                .checked(current_delimiter == delimiter)
                                .on_click(move |_, window, cx| {
                                    let _ = dialog.update(cx, |dialog, cx| {
                                        dialog.set_csv_delimiter(delimiter, window, cx)
                                    });
                                }),
                        );
                    }
                    menu
                })
        };
        let header = csv.inspection.has_header;
        let settings = h_flex()
            .w_full()
            .gap(metrics::SPACE_3)
            .items_end()
            .child(widgets::field(
                strings::CSV_ENCODING,
                encoding_menu,
                None,
                cx,
            ))
            .child(widgets::field(
                strings::CSV_DELIMITER,
                delimiter_menu,
                None,
                cx,
            ))
            .child(
                div().pb(metrics::SPACE_1).child(
                    Checkbox::new("import-csv-header")
                        .label(strings::CSV_HEADER)
                        .checked(header)
                        .on_click(cx.listener(|this, checked: &bool, window, cx| {
                            this.set_csv_header(*checked, window, cx)
                        })),
                ),
            );

        let mut columns = v_flex().gap(metrics::SPACE_2);
        for (index, name) in csv.inspection.columns.iter().enumerate() {
            let role = csv.roles.get(index).copied().unwrap_or(Role::Ignore);
            let values: Vec<String> = csv
                .inspection
                .sample
                .iter()
                .filter_map(|row| row.get(index))
                .filter(|value| !value.trim().is_empty())
                .take(SAMPLE_VALUES)
                .map(|value| {
                    if role == Role::Password {
                        "••••••".to_owned()
                    } else {
                        value.clone()
                    }
                })
                .collect();
            let dialog = dialog.clone();
            let role_menu = ControlButton::new(
                ElementId::Name(format!("import-csv-role-{index}").into()),
                ControlKind::Neutral,
            )
            .label(role_label(role))
            .trailing_icon(IconName::ChevronDown)
            .dropdown_menu(move |mut menu, _, cx| {
                menu = menu.min_w(metrics::scaled(160., cx));
                for candidate in Role::ALL {
                    let dialog = dialog.clone();
                    menu = menu.item(
                        PopupMenuItem::new(role_label(candidate))
                            .checked(candidate == role)
                            .on_click(move |_, _, cx| {
                                let _ = dialog.update(cx, |dialog, cx| {
                                    dialog.set_csv_role(index, candidate, cx)
                                });
                            }),
                    );
                }
                menu
            });
            columns = columns.child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme.foreground)
                                    .truncate()
                                    .child(name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_CAPTION)
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(values.join("  ·  ")),
                            ),
                    )
                    .child(div().flex_none().child(role_menu)),
            );
        }
        let has_address = csv.roles.contains(&Role::Address);
        // The warnings stand above the columns, where they are seen first.
        let body = v_flex()
            .gap(metrics::SPACE_3)
            .child(settings)
            .when(csv.roles.contains(&Role::Password), |this| {
                this.child(password_warning(cx))
            })
            .when(!has_address, |this| {
                this.child(
                    div()
                        .id("import-csv-no-address")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.danger)
                        .child(strings::CSV_NO_ADDRESS),
                )
            })
            .child(section(strings::CSV_COLUMNS, cx))
            .child(widgets::caption(strings::csv_rows(csv.inspection.rows), cx))
            .child(columns);
        div()
            .relative()
            .w_full()
            .child(
                v_flex()
                    .id("import-csv-mapping")
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

    pub(super) fn render_csv_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(preview) = self.csv.as_ref().and_then(|csv| csv.preview.as_ref()) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let count = |status| {
            preview
                .rows
                .iter()
                .filter(|row| row.status == status)
                .count()
        };
        let mut rows = v_flex().gap(metrics::SPACE_2);
        if preview.rows.is_empty() {
            rows = rows.child(widgets::caption(strings::CSV_EMPTY, cx));
        }
        for row in preview.rows.iter().take(PREVIEW_ROWS) {
            let mut target = row.address.clone();
            if let Some(user) = &row.username {
                target = format!("{user}@{target}");
            }
            if let Some(port) = row.port.filter(|port| *port != 22) {
                target = format!("{target}:{port}");
            }
            let title = match &row.label {
                Some(label) => format!("{label}  ·  {target}"),
                None if target.is_empty() => strings::csv_line(row.line),
                None => target,
            };
            let status = match row.status {
                ImportStatus::New => strings::STATUS_NEW,
                ImportStatus::Exists => strings::STATUS_EXISTS,
                ImportStatus::Error => strings::STATUS_ERROR,
            };
            let mut item = v_flex().gap(metrics::rpx(2.)).child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap(metrics::SPACE_2)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(match row.status {
                                ImportStatus::Error => theme.danger,
                                _ => theme.muted_foreground,
                            })
                            .child(status),
                    ),
            );
            for note in row
                .group
                .iter()
                .map(|group| strings::csv_group(group))
                .chain(
                    row.reason
                        .iter()
                        .map(|reason| strings::csv_reason(row.line, reason)),
                )
            {
                item = item.child(widgets::caption(note, cx));
            }
            rows = rows.child(item);
        }
        if preview.rows.len() > PREVIEW_ROWS {
            rows = rows.child(widgets::caption(
                strings::csv_more(preview.rows.len() - PREVIEW_ROWS),
                cx,
            ));
        }
        let body = v_flex()
            .gap(metrics::SPACE_3)
            .child(section(strings::PREVIEW_HOSTS, cx))
            .child(widgets::caption(
                strings::csv_counts(
                    count(ImportStatus::New),
                    count(ImportStatus::Exists),
                    count(ImportStatus::Error),
                    preview.groups_new,
                ),
                cx,
            ))
            .when(preview.passwords > 0, |this| {
                this.child(password_warning(cx))
            })
            .child(rows);
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

    pub(super) fn render_csv_summary(
        &self,
        summary: &CsvImportSummary,
        cx: &App,
    ) -> impl IntoElement {
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
        for (line, reason) in summary.errors.iter().take(PREVIEW_ROWS) {
            errors = errors.child(widgets::caption(strings::csv_reason(*line, reason), cx));
        }
        v_flex()
            .id("import-summary")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1)
            .child(row("import-added", strings::SUMMARY_ADDED, summary.added))
            .child(row(
                "import-skipped",
                strings::CSV_SUMMARY_SKIPPED,
                summary.skipped,
            ))
            .child(row(
                "import-errors",
                strings::SUMMARY_ERRORS,
                summary.errors.len(),
            ))
            .child(errors)
            .child(row(
                "import-groups-added",
                strings::CSV_SUMMARY_GROUPS,
                summary.groups_added,
            ))
            .when(summary.passwords > 0, |this| {
                this.child(
                    div()
                        .id("import-csv-delete-file")
                        .test_support()
                        .pt(metrics::SPACE_2)
                        .child(widgets::caption(strings::CSV_DELETE_FILE, cx)),
                )
            })
    }
}

/// The warning shown while a column is mapped to passwords or the preview
/// has rows with one.
fn password_warning(cx: &App) -> impl IntoElement {
    div()
        .id("import-csv-passwords")
        .test_support()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().warning)
        .child(strings::CSV_PASSWORDS)
}
