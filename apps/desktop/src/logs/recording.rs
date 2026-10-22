//! The session recording viewer (UI 11.2; D75): the output of a recorded
//! session as plain text, read-only, with a search over its lines and
//! "Export to .txt" after a warning (the file is not encrypted, D16).
//!
//! The recording is read and decrypted off the app's thread; the viewer
//! opens in a dialog over the Logs screen. A recording that was not closed
//! (the app stopped) says so; one that is not on this device says that.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, FontWeight, IntoElement, Render, SharedString, Subscription, Window, div,
    uniform_list,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use uuid::Uuid;

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// The height of a line of the viewer.
const LINE_HEIGHT: gpui::Rems = metrics::rpx(18.);
/// Lines the viewer shows at most (Export to .txt has every line).
pub const MAX_LINES: usize = 200_000;

pub struct RecordingViewer {
    log_id: Uuid,
    title: SharedString,
    lines: Arc<Vec<SharedString>>,
    shown: Rc<Vec<usize>>,
    complete: bool,
    /// Longer than [`MAX_LINES`]: only the first lines are shown.
    cut: bool,
    search: Entity<InputState>,
    state: Entity<AppState>,
    _subscription: Subscription,
}

impl std::fmt::Debug for RecordingViewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingViewer")
            .field("lines", &self.lines.len())
            .finish_non_exhaustive()
    }
}

impl RecordingViewer {
    pub fn new(
        log_id: Uuid,
        title: String,
        text: &str,
        complete: bool,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::RECORDING_SEARCH));
        let subscription = cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        let all_lines = text.lines().count();
        let lines: Vec<SharedString> = text
            .lines()
            .take(MAX_LINES)
            .map(|line| line.to_owned().into())
            .collect();
        let cut = all_lines > MAX_LINES;
        let shown = Rc::new((0..lines.len()).collect());
        Self {
            log_id,
            title: title.into(),
            lines: Arc::new(lines),
            shown,
            complete,
            cut,
            search,
            state,
            _subscription: subscription,
        }
    }

    /// The lines on display (all, or those holding the search text).
    pub fn shown_lines(&self) -> Vec<&str> {
        self.shown
            .iter()
            .map(|index| self.lines[*index].as_ref())
            .collect()
    }

    pub fn search(&self) -> &Entity<InputState> {
        &self.search
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.shown = Rc::new(
            (0..self.lines.len())
                .filter(|index| {
                    query.is_empty() || self.lines[*index].to_lowercase().contains(&query)
                })
                .collect(),
        );
        cx.notify();
    }

    /// Export to .txt: a warning first, then where to save it.
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let viewer = cx.entity().downgrade();
        let go = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = viewer.update(cx, |viewer, cx| viewer.choose_path(window, cx));
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            let confirm = go.clone();
            let on_ok = go.clone();
            dialog
                .w(metrics::scaled(420., cx))
                .bg(cx.theme().popover)
                .rounded(metrics::RADIUS_OVERLAY)
                .close_button(false)
                .title(
                    div()
                        .text_size(metrics::FONT_SIZE_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(strings::EXPORT_TITLE),
                )
                .child(
                    div()
                        .id("recording-export-dialog")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().muted_foreground)
                        .child(strings::EXPORT_BODY),
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
                            ControlButton::new("recording-export-cancel", ControlKind::Neutral)
                                .label(strings::CANCEL)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            ControlButton::new("recording-export-confirm", ControlKind::Accent)
                                .label(strings::EXPORT_CONFIRM)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    confirm(window, cx);
                                }),
                        ),
                )
        });
    }

    fn choose_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = ::dirs::document_dir()
            .or_else(::dirs::home_dir)
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&directory, Some(strings::EXPORT_FILE_NAME));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let _ = this.update_in(cx, |viewer, window, cx| viewer.export_to(path, window, cx));
        })
        .detach();
    }

    /// Writes the recording as plain text to `path`, off the app's thread,
    /// and says how it went.
    pub fn export_to(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let log_id = self.log_id;
        let written = cx.background_spawn(async move { core.recording_export_txt(log_id, &path) });
        cx.spawn_in(window, async move |_, cx| {
            let message = match written.await {
                Ok(()) => strings::EXPORTED,
                Err(error) => {
                    tracing::warn!(code = error.code(), "recording not exported");
                    strings::EXPORT_FAILED
                }
            };
            let _ = cx.update(|window, cx| crate::shell::show_toast(message, window, cx));
        })
        .detach();
    }
}

impl Render for RecordingViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let lines = self.lines.clone();
        let shown = self.shown.clone();
        let foreground = theme.foreground;
        let count = shown.len();
        let list = uniform_list("recording-lines", count, move |range, _, _| {
            range
                .map(|index| {
                    let line = lines[shown[index]].clone();
                    div()
                        .h(LINE_HEIGHT)
                        .whitespace_nowrap()
                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                        .text_size(metrics::FONT_SIZE_DENSE)
                        .text_color(foreground)
                        .child(line)
                })
                .collect()
        })
        .flex_1()
        .min_h_0();
        let searching = !self.search.read(cx).value().trim().is_empty();
        v_flex()
            .id("recording-viewer")
            .test_support()
            .w_full()
            .h(metrics::rpx(440.))
            .gap(metrics::SPACE_2)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(self.title.clone()),
            )
            .child(widgets::text_input(
                &self.search,
                "recording-search",
                false,
                window,
                cx,
            ))
            .when(searching, |this| {
                this.child(widgets::caption(strings::matching_lines(count), cx))
            })
            .when(self.cut, |this| {
                this.child(widgets::caption(strings::RECORDING_CUT, cx))
            })
            .when(!self.complete, |this| {
                this.child(
                    div()
                        .id("recording-incomplete")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.warning)
                        .child(strings::RECORDING_INCOMPLETE),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .p(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_CONTROL)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .child(list),
            )
    }
}

/// Opens the recording viewer in a dialog.
pub fn open(viewer: Entity<RecordingViewer>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let export = viewer.clone();
        dialog
            .w(metrics::scaled(760., cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::RECORDING_TITLE),
            )
            .child(viewer.clone())
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("recording-export", ControlKind::Neutral)
                            .label(strings::EXPORT_TXT)
                            .on_click(move |_, window, cx| {
                                export.update(cx, |viewer, cx| viewer.export(window, cx))
                            }),
                    )
                    .child(
                        ControlButton::new("recording-close", ControlKind::Accent)
                            .label(strings::CLOSE)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}
