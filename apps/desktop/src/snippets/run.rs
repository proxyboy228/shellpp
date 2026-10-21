//! Running a snippet on its targets (UI 9.4; D66): the form of the values
//! of its `{{name}}` variables, the confirmation that lists the hosts when
//! there are two or more, and the results table.
//!
//! Every run goes through exec channels of the session layer
//! (`SessionManager::run_on_hosts`: saved credentials and known host keys
//! only, at most 8 hosts at once, each with [`RUN_TIMEOUT`]). The table
//! shows each host's status, exit code and, on click, its output; the
//! split tab of UI 9.4 for up to four hosts is not built yet (D66).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, IntoElement, Render, SharedString, Task, Window,
    div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::service::dto::RecordRef;
use shellpp_core::service::snippets::SnippetView;
use shellpp_ssh::batch::{BatchEvent, TargetOutcome};
use uuid::Uuid;

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

/// How long each host has to connect and run the script.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(60);

const DIALOG_WIDTH: gpui::Pixels = px(440.);
const RESULTS_WIDTH: gpui::Pixels = px(720.);

/// Output as text: control characters other than new lines and tabs (and
/// so escape sequences and bidirectional overrides) are shown as `·`.
pub fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| {
            let bidi = matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
            if (c.is_control() && c != '\n' && c != '\t') || bidi {
                '·'
            } else {
                c
            }
        })
        .collect()
}

/// Asks the values of the snippet's variables and hands the filled script
/// to `then` (a snippet without variables: at once; the screen does not
/// call this then, as `then` updates the screen).
pub fn ask_values(
    snippet: &SnippetView,
    window: &mut Window,
    cx: &mut App,
    then: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    if snippet.variables.is_empty() {
        then(snippet.script.clone(), window, cx);
        return;
    }
    let script = snippet.script.clone();
    let inputs: Vec<(String, Entity<InputState>)> = snippet
        .variables
        .iter()
        .map(|name| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(name.clone()));
            (name.clone(), input)
        })
        .collect();
    if let Some((_, first)) = inputs.first() {
        first.update(cx, |input, cx| input.focus(window, cx));
    }
    let inputs = Rc::new(inputs);
    let then = Rc::new(then);
    window.open_dialog(cx, move |dialog, window, cx| {
        let submit = {
            let inputs = inputs.clone();
            let script = script.clone();
            let then = then.clone();
            move |window: &mut Window, cx: &mut App| {
                let values: HashMap<String, String> = inputs
                    .iter()
                    .map(|(name, input)| (name.clone(), input.read(cx).value().to_string()))
                    .collect();
                match shellpp_core::snippet::fill(&script, &values) {
                    Ok(filled) => {
                        window.close_dialog(cx);
                        then(filled, window, cx);
                    }
                    Err(_) => tracing::warn!("snippet variables could not be filled"),
                }
            }
        };
        let submit = Rc::new(submit);
        let on_ok = submit.clone();
        dialog
            .w(DIALOG_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::VARIABLES_TITLE),
            )
            .child(
                v_flex()
                    .id("snippet-variables-form")
                    .test_support()
                    .gap(metrics::SPACE_3)
                    .child(widgets::caption(strings::VARIABLES_NOTE, cx))
                    .children(inputs.iter().map(|(name, input)| {
                        widgets::field(
                            name.clone(),
                            widgets::text_input(
                                input,
                                SharedString::from(format!("snippet-variable-{name}")),
                                false,
                                window,
                                cx,
                            ),
                            None,
                            cx,
                        )
                    })),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("snippet-variables-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("snippet-variables-run", ControlKind::Accent)
                            .label(strings::RUN)
                            .on_click(move |_, window, cx| submit(window, cx)),
                    ),
            )
    });
}

/// "Run on these hosts?" with the list (9.4: two or more hosts).
pub fn confirm(
    title: String,
    hosts: Vec<(Uuid, String)>,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(&mut Window, &mut App) + 'static,
) {
    let start = Rc::new(start);
    let body: SharedString = strings::confirm_body(&title, hosts.len()).into();
    let hosts = Rc::new(hosts);
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme();
        let run = start.clone();
        let on_ok = start.clone();
        dialog
            .w(DIALOG_WIDTH)
            .bg(theme.popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::CONFIRM_TITLE),
            )
            .child(
                v_flex()
                    .id("snippet-confirm")
                    .test_support()
                    .gap(metrics::SPACE_2)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.muted_foreground)
                            .child(body.clone()),
                    )
                    .child(
                        v_flex()
                            .id("snippet-confirm-hosts")
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .gap(metrics::SPACE_1)
                            .children(hosts.iter().map(|(_, title)| {
                                div()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .child(format!("• {title}"))
                            })),
                    ),
            )
            // Enter: closed before the run opens its results, so the close
            // takes this dialog, not the results.
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
                        ControlButton::new("snippet-confirm-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("snippet-confirm-run", ControlKind::Accent)
                            .label(strings::RUN)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                run(window, cx);
                            }),
                    ),
            )
    });
}

/// Where a host's run is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    Waiting,
    Running,
    Done(TargetOutcome),
    /// The session layer is not running (tests, a failed start).
    Unavailable,
}

/// One host of a run.
#[derive(Clone, Debug)]
pub struct RunRow {
    pub host: Uuid,
    pub title: String,
    pub state: RunState,
    pub expanded: bool,
}

impl RunRow {
    /// The status words and whether they mean failure.
    pub fn status(&self) -> (String, bool) {
        match &self.state {
            RunState::Waiting => (strings::WAITING.to_owned(), false),
            RunState::Running => (strings::RUNNING.to_owned(), false),
            RunState::Unavailable => (strings::FAILED.to_owned(), true),
            RunState::Done(outcome) if outcome.succeeded() => (strings::SUCCESS.to_owned(), false),
            RunState::Done(TargetOutcome::TimedOut(_)) => (strings::TIMED_OUT.to_owned(), true),
            RunState::Done(_) => (strings::FAILED.to_owned(), true),
        }
    }

    /// The exit code, or why there is none.
    pub fn detail(&self) -> String {
        let reason = |kind| {
            crate::session_ui::bridge::failure_of(crate::sessions::SessionError::Ssh(kind))
                .text()
                .to_owned()
        };
        match &self.state {
            RunState::Done(TargetOutcome::Finished(output)) => output
                .exit_status
                .map(strings::exit_code)
                .unwrap_or_default(),
            RunState::Done(TargetOutcome::ConnectFailed(error))
            | RunState::Done(TargetOutcome::ExecFailed(error)) => reason(error.kind),
            RunState::Unavailable => strings::NOT_CONNECTED.to_owned(),
            _ => String::new(),
        }
    }

    /// The output kept, as text.
    pub fn output(&self) -> Option<(String, bool)> {
        match &self.state {
            RunState::Done(TargetOutcome::Finished(output))
            | RunState::Done(TargetOutcome::TimedOut(output)) => {
                Some((output_text(&output.stdout), output.truncated))
            }
            _ => None,
        }
    }
}

/// The results table of one run.
pub struct RunResults {
    title: String,
    rows: Vec<RunRow>,
    _run: Option<Task<()>>,
}

impl std::fmt::Debug for RunResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunResults")
            .field("rows", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl RunResults {
    /// Starts the run of `script` on `targets` (host id, title).
    pub fn start(
        title: String,
        script: String,
        vault_id: Uuid,
        targets: Vec<(Uuid, String)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut rows: Vec<RunRow> = targets
            .iter()
            .map(|(host, title)| RunRow {
                host: *host,
                title: title.clone(),
                state: RunState::Waiting,
                expanded: false,
            })
            .collect();
        let run = match crate::sessions::ui::manager(cx) {
            None => {
                for row in &mut rows {
                    row.state = RunState::Unavailable;
                }
                None
            }
            Some(manager) => {
                let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
                let hosts = targets
                    .iter()
                    .map(|(host, _)| RecordRef {
                        vault_id,
                        record_id: *host,
                    })
                    .collect();
                let done = manager.run_on_hosts(script, hosts, RUN_TIMEOUT, Some(events));
                tracing::info!(targets = targets.len(), "snippet run started");
                Some(cx.spawn(async move |this, cx| {
                    while let Some(event) = received.recv().await {
                        if this
                            .update(cx, |results, cx| results.apply(event, cx))
                            .is_err()
                        {
                            return;
                        }
                    }
                    let outcomes = done.await;
                    let _ = this.update(cx, |results, cx| {
                        for (row, outcome) in results.rows.iter_mut().zip(outcomes) {
                            row.state = RunState::Done(outcome);
                        }
                        let succeeded = results.succeeded();
                        tracing::info!(succeeded, total = results.rows.len(), "snippet run ended");
                        cx.notify();
                    });
                }))
            }
        };
        Self {
            title,
            rows,
            _run: run,
        }
    }

    /// Results shown as they are, with no run behind them (the screenshot
    /// run: nothing connects).
    pub fn preview(title: String, rows: Vec<RunRow>) -> Self {
        Self {
            title,
            rows,
            _run: None,
        }
    }

    fn apply(&mut self, event: BatchEvent, cx: &mut Context<Self>) {
        match event {
            BatchEvent::Started(index) => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.state = RunState::Running;
                }
            }
            BatchEvent::Finished(index, outcome) => {
                if let Some(row) = self.rows.get_mut(index) {
                    row.state = RunState::Done(outcome);
                }
            }
        }
        cx.notify();
    }

    pub fn rows(&self) -> &[RunRow] {
        &self.rows
    }

    /// Every host is done (or cannot run).
    pub fn finished(&self) -> bool {
        self.rows
            .iter()
            .all(|row| matches!(row.state, RunState::Done(_) | RunState::Unavailable))
    }

    pub fn succeeded(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(&row.state, RunState::Done(outcome) if outcome.succeeded()))
            .count()
    }

    pub fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get_mut(index) {
            row.expanded = !row.expanded;
            cx.notify();
        }
    }

    /// Sets a row by hand (the screenshot run: nothing connects).
    pub fn show_row(
        &mut self,
        index: usize,
        state: RunState,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.rows.get_mut(index) {
            row.state = state;
            row.expanded = expanded;
            cx.notify();
        }
    }
}

impl Render for RunResults {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = crate::theme::tokens(cx);
        let hover = theme.secondary_hover;
        let summary = if self.finished() {
            strings::summary(self.succeeded(), self.rows.len())
        } else {
            strings::RUNNING.to_owned()
        };
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let (status, failed) = row.status();
            let color = if failed {
                theme.danger
            } else if status == strings::SUCCESS {
                theme.success
            } else {
                theme.muted_foreground
            };
            let output = row.expanded.then(|| row.output()).flatten();
            let output = output.map(|(text, cut)| {
                v_flex()
                    .id(SharedString::from(format!("snippet-output-{}", row.host)))
                    .test_support()
                    .w_full()
                    .gap(metrics::SPACE_1)
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "snippet-output-text-{}",
                                row.host
                            )))
                            .w_full()
                            .max_h(px(200.))
                            .overflow_y_scroll()
                            .p(metrics::SPACE_2)
                            .rounded(metrics::RADIUS_CONTROL)
                            .bg(tokens.bg_input.hsla())
                            .border_1()
                            .border_color(theme.border)
                            .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .whitespace_normal()
                            .child(if text.is_empty() {
                                strings::NO_OUTPUT.to_owned()
                            } else {
                                text
                            }),
                    )
                    .when(cut, |this| {
                        this.child(widgets::caption(strings::OUTPUT_CUT, cx))
                    })
            });
            v_flex()
                .w_full()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .id(SharedString::from(format!("snippet-result-{}", row.host)))
                        .test_support()
                        .w_full()
                        .h(metrics::HEIGHT_ROW)
                        .px(metrics::SPACE_2)
                        .gap(metrics::SPACE_3)
                        .items_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(
                            div()
                                .flex_none()
                                .w(px(240.))
                                .truncate()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .child(row.title.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(96.))
                                .text_size(metrics::FONT_SIZE_CAPTION)
                                .text_color(color)
                                .child(status),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_CAPTION)
                                .text_color(theme.muted_foreground)
                                .child(row.detail()),
                        )
                        .on_click(cx.listener(move |results, _: &ClickEvent, _, cx| {
                            results.toggle(index, cx)
                        })),
                )
                .children(output.map(|output| {
                    div()
                        .px(metrics::SPACE_2)
                        .pb(metrics::SPACE_2)
                        .child(output)
                }))
        });
        v_flex()
            .id("snippet-results")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .id("snippet-results-summary")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    ),
            )
            .child(
                v_flex()
                    .id("snippet-results-rows")
                    .w_full()
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .border_t_1()
                    .border_color(theme.border)
                    .children(rows),
            )
    }
}

/// The results dialog (its table updates as hosts finish).
pub fn open_results(results: Entity<RunResults>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .w(RESULTS_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::RESULTS_TITLE),
            )
            .child(results.clone())
            .footer(
                h_flex().w_full().justify_end().child(
                    ControlButton::new("snippet-results-close", ControlKind::Neutral)
                        .label(strings::CLOSE)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}
