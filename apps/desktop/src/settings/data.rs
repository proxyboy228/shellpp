//! The Account & Sync, SFTP and Logs sections of the Settings window
//! (UI 12.2, 12.6; stage 6, D67).
//!
//! - Account & Sync without a server: "Connect sync server" (disabled until
//!   sync arrives, stage 7) and Export Data / Import Data.
//! - SFTP: hidden files, delete confirmation, the answer to name
//!   conflicts, parallel transfers and the program for Open.
//! - Logs: the retention period, session output recording (disabled until
//!   the recorder arrives) and Clear the log with a confirmation.
//!
//! Every change is written at once (`settings_set`); there is no Save.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, FontWeight, IntoElement, SharedString, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use serde_json::{Map, Value, json};
use shellpp_core::service::dto::SettingsSetInput;

use super::{SettingsView, strings};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

/// The answers to name conflicts (5.7), as stored.
pub const CONFLICTS: [(&str, &str); 4] = [
    ("ask", strings::CONFLICT_ASK),
    ("replace", strings::CONFLICT_REPLACE),
    ("skip", strings::CONFLICT_SKIP),
    ("rename", strings::CONFLICT_RENAME),
];

/// Parallel transfers offered (the setting takes 1 to 10).
pub const PARALLEL: [u64; 6] = [1, 2, 3, 4, 6, 10];

/// Retention periods offered, in days.
pub const RETENTION: [u64; 5] = [7, 30, 90, 180, 365];

fn heading(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_HEADING)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(cx.theme().popover_foreground)
        .child(text)
}

fn label(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(cx.theme().popover_foreground)
        .child(text)
}

/// A setting with a switch on the right and its explanation below.
fn switch_row(
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .id(id)
        .test_support()
        .gap(metrics::SPACE_2)
        .child(
            h_flex()
                .justify_between()
                .gap(metrics::SPACE_3)
                .child(label(title, cx))
                .child(control),
        )
        .child(widgets::caption(hint, cx))
}

impl SettingsView {
    /// The stored settings (defaults included).
    pub(super) fn values(&self, cx: &App) -> Map<String, Value> {
        self.state
            .read(cx)
            .core()
            .and_then(|core| core.settings_get().ok())
            .map(|values| values.into_iter().collect())
            .unwrap_or_default()
    }

    /// Writes one setting now.
    pub fn set_value(&mut self, key: &str, value: Value, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let written = core.settings_set(SettingsSetInput {
            values: [(key.to_owned(), value)].into_iter().collect(),
            reset: Vec::new(),
        });
        self.error = match written {
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(code = error.code(), "setting not saved");
                Some(strings::FAILED.into())
            }
        };
        cx.notify();
    }

    /// The program for Open (the field writes as it is typed).
    pub(super) fn open_program_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.open_program.read(cx).value().trim().to_owned();
        let value = if text.is_empty() {
            Value::Null
        } else {
            json!(text)
        };
        self.set_value("sftp.open_program", value, cx);
    }

    /// Clear the log (12.6): asks first.
    pub fn confirm_clear_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        let clear = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = view.update(cx, |view, cx| view.clear_log(window, cx));
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            let confirm = clear.clone();
            let on_ok = clear.clone();
            dialog
                .w(px(420.))
                .bg(cx.theme().popover)
                .rounded(metrics::RADIUS_OVERLAY)
                .close_button(false)
                .title(
                    div()
                        .text_size(metrics::FONT_SIZE_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(strings::CLEAR_LOG_TITLE),
                )
                .child(
                    div()
                        .id("settings-clear-log-dialog")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().muted_foreground)
                        .child(strings::CLEAR_LOG_BODY),
                )
                .on_ok(move |_, window, cx| {
                    window.close_dialog(cx);
                    on_ok(window, cx);
                    false
                })
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap(metrics::SPACE_2)
                        .child(
                            ControlButton::new("settings-clear-log-cancel", ControlKind::Neutral)
                                .label(strings::CANCEL)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            ControlButton::new("settings-clear-log-confirm", ControlKind::Danger)
                                .label(strings::CLEAR_LOG)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    confirm(window, cx);
                                }),
                        ),
                )
        });
    }

    pub fn clear_log(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.connection_log_clear() {
            Ok(count) => {
                self.cleared = Some(count);
                self.error = None;
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "connection log not cleared");
                self.error = Some(strings::FAILED.into());
            }
        }
        // The core reports the change: the Logs screen reads the log again.
        cx.notify();
    }

    pub(super) fn render_account(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state.clone();
        let import_state = self.state.clone();
        v_flex()
            .id("settings-account")
            .test_support()
            .gap(metrics::SPACE_4)
            .child(heading(strings::ACCOUNT_SYNC, cx))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::SYNC_SERVER, cx))
                    .child(widgets::caption(strings::SYNC_SERVER_HINT, cx))
                    .child(
                        h_flex().child(
                            ControlButton::new("settings-connect-sync", ControlKind::Neutral)
                                .label(strings::CONNECT_SYNC)
                                .disabled(true),
                        ),
                    ),
            )
            .child(widgets::divider(cx))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::DATA, cx))
                    .child(widgets::caption(strings::DATA_HINT, cx))
                    .child(
                        h_flex()
                            .gap(metrics::SPACE_2)
                            .child(
                                ControlButton::new("settings-export-data", ControlKind::Neutral)
                                    .label(strings::EXPORT_DATA)
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        crate::data_transfer::open_export(
                                            state.clone(),
                                            window,
                                            cx,
                                        );
                                    }),
                            )
                            .child(
                                ControlButton::new("settings-import-data", ControlKind::Neutral)
                                    .label(strings::IMPORT_DATA)
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        crate::data_transfer::open_import(
                                            import_state.clone(),
                                            window,
                                            cx,
                                        );
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_sftp(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let values = self.values(cx);
        let flag =
            |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let conflict = values
            .get("sftp.name_conflict")
            .and_then(Value::as_str)
            .unwrap_or("ask")
            .to_owned();
        let parallel = values
            .get("sftp.parallel_transfers")
            .and_then(Value::as_u64)
            .unwrap_or(3);
        let hidden = Switch::new("settings-sftp-hidden")
            .checked(flag("sftp.show_hidden", false))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_value("sftp.show_hidden", json!(*checked), cx)
            }));
        let confirm = Switch::new("settings-sftp-confirm-delete")
            .checked(flag("sftp.confirm_delete", true))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_value("sftp.confirm_delete", json!(*checked), cx)
            }));
        let conflicts = widgets::segments(CONFLICTS.iter().map(|(value, text)| {
            let value = *value;
            widgets::segment(
                SharedString::from(format!("settings-sftp-conflict-{value}")),
                *text,
                conflict == value,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_value("sftp.name_conflict", json!(value), cx)
            }))
            .into_any_element()
        }));
        let parallel = widgets::segments(PARALLEL.iter().map(|count| {
            let count = *count;
            widgets::segment(
                SharedString::from(format!("settings-sftp-parallel-{count}")),
                count.to_string(),
                parallel == count,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_value("sftp.parallel_transfers", json!(count), cx)
            }))
            .into_any_element()
        }));
        v_flex()
            .id("settings-sftp")
            .test_support()
            .gap(metrics::SPACE_4)
            .child(heading(strings::SFTP, cx))
            .child(switch_row(
                "settings-sftp-hidden-row",
                strings::SHOW_HIDDEN,
                strings::SHOW_HIDDEN_HINT,
                hidden,
                cx,
            ))
            .child(switch_row(
                "settings-sftp-confirm-row",
                strings::CONFIRM_DELETE,
                strings::CONFIRM_DELETE_HINT,
                confirm,
                cx,
            ))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::NAME_CONFLICT, cx))
                    .child(conflicts),
            )
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::PARALLEL, cx))
                    .child(parallel),
            )
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::OPEN_PROGRAM, cx))
                    .child(widgets::caption(strings::OPEN_PROGRAM_HINT, cx))
                    .child(widgets::text_input(
                        &self.open_program,
                        "settings-sftp-open-program",
                        false,
                        window,
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_logs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let values = self.values(cx);
        let retention = values
            .get("logs.retention_days")
            .and_then(Value::as_u64)
            .unwrap_or(90);
        let periods = widgets::segments(RETENTION.iter().map(|days| {
            let days = *days;
            widgets::segment(
                SharedString::from(format!("settings-logs-retention-{days}")),
                strings::days(days),
                retention == days,
                cx,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_value("logs.retention_days", json!(days), cx)
            }))
            .into_any_element()
        }));
        let record = Switch::new("settings-logs-record")
            .checked(false)
            .disabled(true);
        let danger = theme.danger;
        let hover = theme.secondary_hover;
        let clear = div()
            .id("settings-clear-log")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .h(metrics::HEIGHT_CONTROL)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CONTROL)
            .border_1()
            .border_color(theme.input)
            .text_size(metrics::FONT_SIZE_BODY)
            .font_weight(FontWeight::MEDIUM)
            .text_color(danger)
            .cursor_default()
            .hover(move |style| style.bg(hover))
            .child(strings::CLEAR_LOG)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_clear_log(window, cx)),
            );
        v_flex()
            .id("settings-logs")
            .test_support()
            .gap(metrics::SPACE_4)
            .child(heading(strings::LOGS, cx))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::RETENTION, cx))
                    .child(widgets::caption(strings::RETENTION_HINT, cx))
                    .child(periods),
            )
            .child(switch_row(
                "settings-logs-record-row",
                strings::RECORD_OUTPUT,
                strings::RECORD_OUTPUT_HINT,
                record,
                cx,
            ))
            .child(widgets::divider(cx))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(label(strings::CLEAR_LOG, cx))
                    .child(widgets::caption(strings::CLEAR_LOG_HINT, cx))
                    .child(h_flex().child(clear))
                    .when_some(self.cleared, |this, count| {
                        this.child(
                            div()
                                .id("settings-log-cleared")
                                .test_support()
                                .child(widgets::caption(strings::cleared(count), cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}
