//! The Logs screen (UI 11; stage 6, D67): the connection log as a table.
//!
//! - Date: the session's start date, its start and end times below
//!   (`(+1d)` when it ended on a later day, "now" while it is open); the
//!   header toggles newest or oldest first. Times are UTC, as elsewhere
//!   until the app formats dates in the local time zone (D65).
//! - User: the device ("This device" in local mode: there are no accounts
//!   before stage 8, UI 11.1).
//! - Host: the OS icon, the host's title (its name now, else the snapshot)
//!   and the protocol with the remote user.
//! - Saved: the bookmark; saved entries are kept by the auto-cleanup. The
//!   header shows only the saved entries.
//! - A click opens the connection's details on the right (11.2: the output
//!   is not recorded yet, so the details are what there is).
//!
//! The log is read on a background thread and again when records change.
//! Nothing here logs addresses or names.

pub mod strings;

#[cfg(test)]
mod tests;

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, FontWeight, IntoElement, Pixels, Render, SharedString,
    Subscription, Task, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use shellpp_core::service::dto::ConnectionLogView;
use uuid::Uuid;

use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::shell::details::DetailsHeader;
use crate::state::{AppState, AppStateEvent};
use crate::theme::metrics;

const CONTENT_PADDING: Pixels = metrics::SPACE_5;
const DATE_COLUMN: Pixels = px(200.);
const USER_COLUMN: Pixels = px(160.);
const SAVED_COLUMN: Pixels = px(72.);

/// Stable element id of a row and of its bookmark.
pub fn row_id(id: Uuid) -> SharedString {
    format!("log-row-{id}").into()
}

pub fn saved_id(id: Uuid) -> SharedString {
    format!("log-saved-{id}").into()
}

/// The date and the time of a UTC timestamp (`2026-10-21T18:04:05.123Z`).
fn date_time(timestamp: &str) -> (&str, &str) {
    (
        timestamp.get(..10).unwrap_or(timestamp),
        timestamp.get(11..16).unwrap_or(""),
    )
}

/// Days between two `YYYY-MM-DD` dates (0 when unreadable).
fn days_between(start: &str, end: &str) -> i64 {
    let day = |date: &str| -> Option<i64> {
        let year: i64 = date.get(..4)?.parse().ok()?;
        let month: i64 = date.get(5..7)?.parse().ok()?;
        let day: i64 = date.get(8..10)?.parse().ok()?;
        // Days from civil (proleptic Gregorian), enough to compare dates.
        let (y, m) = if month <= 2 {
            (year - 1, month + 9)
        } else {
            (year, month - 3)
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146_097 + doe)
    };
    match (day(start), day(end)) {
        (Some(start), Some(end)) => end - start,
        _ => 0,
    }
}

/// The two lines of the Date column: the start date, then "18:04 – 19:10
/// UTC", "23:50 – 00:20 (+1d) UTC" or "18:04 – now".
pub fn date_lines(entry: &ConnectionLogView) -> (String, String) {
    let (date, start) = date_time(&entry.started_at);
    let times = match &entry.ended_at {
        _ if entry.is_active() => format!("{start} – {}", strings::NOW),
        Some(ended) => {
            let (end_date, end) = date_time(ended);
            match days_between(date, end_date) {
                0 => format!("{start} – {end} UTC"),
                days => format!("{start} – {end} (+{days}d) UTC"),
            }
        }
        None => format!("{start} – {}", strings::UNKNOWN_END),
    };
    (date.to_owned(), times)
}

/// How the session ended, in words.
pub fn end_text(entry: &ConnectionLogView) -> String {
    match entry.end_reason.as_deref() {
        _ if entry.is_active() => strings::ACTIVE.to_owned(),
        Some("closed") => strings::END_CLOSED.to_owned(),
        Some("exit_status") => strings::end_exit(entry.exit_status),
        Some("dropped") => strings::END_DROPPED.to_owned(),
        _ => strings::END_UNKNOWN.to_owned(),
    }
}

pub struct LogsScreen {
    state: Entity<AppState>,
    entries: Vec<ConnectionLogView>,
    newest_first: bool,
    saved_only: bool,
    selected: Option<Uuid>,
    loading: Option<Task<()>>,
    /// The records changed while a load ran: one more after it.
    stale: bool,
    failed: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for LogsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogsScreen")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl LogsScreen {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
            AppStateEvent::RecordsChanged => this.reload(cx),
            AppStateEvent::LockChanged(_) => {
                this.selected = None;
                this.entries = Vec::new();
                this.reload(cx);
            }
            AppStateEvent::SelectionChanged
            | AppStateEvent::LocationChanged(_)
            | AppStateEvent::AutoLockDue => {}
        });
        let mut screen = Self {
            state,
            entries: Vec::new(),
            newest_first: true,
            saved_only: false,
            selected: None,
            loading: None,
            stale: false,
            failed: false,
            _subscriptions: vec![subscription],
        };
        screen.reload(cx);
        screen
    }

    // -- Reading -----------------------------------------------------------------

    pub fn entries(&self) -> &[ConnectionLogView] {
        &self.entries
    }

    /// The entries on display, in display order.
    pub fn shown(&self) -> Vec<&ConnectionLogView> {
        let mut shown: Vec<&ConnectionLogView> = self
            .entries
            .iter()
            .filter(|entry| !self.saved_only || entry.saved)
            .collect();
        if !self.newest_first {
            shown.reverse();
        }
        shown
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    pub fn newest_first(&self) -> bool {
        self.newest_first
    }

    pub fn saved_only(&self) -> bool {
        self.saved_only
    }

    /// The panel's width while an entry is open (the shell collapses its
    /// sidebar by it, UI 1.2).
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.selected_entry().map(|_| metrics::WIDTH_DETAILS)
    }

    fn selected_entry(&self) -> Option<&ConnectionLogView> {
        let id = self.selected?;
        self.entries.iter().find(|entry| entry.record_id == id)
    }

    // -- Loading -------------------------------------------------------------------

    /// Reads the log again on a background thread (empty while locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            self.stale = true;
            return;
        }
        let state = self.state.read(cx);
        let Some(core) = state.core().filter(|_| state.is_unlocked()).cloned() else {
            self.entries = Vec::new();
            cx.notify();
            return;
        };
        let listed = cx.background_spawn(async move { core.connection_log_list() });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let listed = listed.await;
            let _ = this.update(cx, |screen, cx| {
                screen.loading = None;
                match listed {
                    Ok(entries) => {
                        screen.entries = entries;
                        screen.failed = false;
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "connection log could not be read");
                        screen.failed = true;
                    }
                }
                if std::mem::take(&mut screen.stale) {
                    screen.reload(cx);
                }
                cx.notify();
            });
        }));
    }

    // -- Commands --------------------------------------------------------------------

    pub fn toggle_sort(&mut self, cx: &mut Context<Self>) {
        self.newest_first = !self.newest_first;
        cx.notify();
    }

    pub fn toggle_saved_only(&mut self, cx: &mut Context<Self>) {
        self.saved_only = !self.saved_only;
        cx.notify();
    }

    pub fn select(&mut self, id: Option<Uuid>, cx: &mut Context<Self>) {
        self.selected = id;
        cx.notify();
    }

    /// The bookmark of an entry (11.1).
    pub fn toggle_saved(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.record_id == id) else {
            return;
        };
        let saved = !entry.saved;
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.connection_log_set_saved(id, saved) {
            Ok(()) => {
                entry.saved = saved;
                cx.notify();
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "log entry bookmark not saved");
                crate::shell::show_toast(strings::SAVE_FAILED, window, cx);
            }
        }
    }

    // -- Rendering -------------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let cell = |text: &'static str| {
            div()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let sort = ControlButton::new("log-sort", ControlKind::Ghost)
            .label(strings::DATE)
            .trailing_icon(if self.newest_first {
                Lucide::ArrowDown
            } else {
                Lucide::ArrowUp
            })
            .tooltip(if self.newest_first {
                strings::SORT_NEWEST
            } else {
                strings::SORT_OLDEST
            })
            .height(px(28.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_sort(cx)));
        let saved = ControlButton::new("log-saved-filter", ControlKind::Ghost)
            .icon(if self.saved_only {
                Lucide::BookmarkCheck
            } else {
                Lucide::Bookmark
            })
            .tooltip(if self.saved_only {
                strings::SAVED_ALL
            } else {
                strings::SAVED_ONLY
            })
            .height(px(28.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_saved_only(cx)));
        h_flex()
            .id("log-header")
            .test_support()
            .w_full()
            .h(metrics::HEIGHT_ROW)
            .px(metrics::SPACE_2)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            // The button's own padding: its label lines up with the dates.
            .child(
                h_flex()
                    .flex_none()
                    .w(DATE_COLUMN)
                    .child(div().ml(-metrics::SPACE_3).child(sort)),
            )
            .child(cell(strings::USER).flex_none().w(USER_COLUMN))
            .child(cell(strings::HOST).flex_1().min_w_0())
            .child(
                h_flex()
                    .flex_none()
                    .w(SAVED_COLUMN)
                    .justify_center()
                    .child(saved),
            )
    }

    fn render_row(&self, entry: &ConnectionLogView, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tokens = crate::theme::tokens(cx);
        let id = entry.record_id;
        let selected = self.selected == Some(id);
        let hover = theme.secondary_hover;
        let (date, times) = date_lines(entry);
        let secondary = |text: String| {
            div()
                .truncate()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let protocol = match &entry.username {
            Some(user) => format!("{} · {user}", entry.protocol.to_uppercase()),
            None => entry.protocol.to_uppercase(),
        };
        let os_icon = crate::hosts::layout::os_icon(
            entry
                .os
                .as_ref()
                .map(|os| crate::session_ui::bridge::os_family(os.family)),
        );
        h_flex()
            .id(row_id(id))
            .test_support()
            .w_full()
            .min_h(px(52.))
            .px(metrics::SPACE_2)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .cursor_pointer()
            .map(|this| {
                if selected {
                    this.bg(tokens.accent_subtle.hsla())
                } else {
                    this.hover(move |style| style.bg(hover))
                }
            })
            .child(
                v_flex()
                    .flex_none()
                    .w(DATE_COLUMN)
                    .child(div().text_size(metrics::FONT_SIZE_BODY).child(date))
                    .child(secondary(times)),
            )
            .child(
                v_flex().flex_none().w(USER_COLUMN).child(
                    div().truncate().text_size(metrics::FONT_SIZE_BODY).child(
                        if entry.this_device {
                            strings::THIS_DEVICE
                        } else {
                            strings::OTHER_DEVICE
                        },
                    ),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(
                        Icon::new(os_icon)
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .child(entry.title.clone()),
                            )
                            .child(secondary(protocol)),
                    ),
            )
            .child(
                h_flex().flex_none().w(SAVED_COLUMN).justify_center().child(
                    ControlButton::new(saved_id(id), ControlKind::Ghost)
                        .icon(if entry.saved {
                            Lucide::BookmarkCheck
                        } else {
                            Lucide::Bookmark
                        })
                        .selected(entry.saved)
                        .tooltip(if entry.saved {
                            strings::UNSAVE
                        } else {
                            strings::SAVE
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.toggle_saved(id, window, cx)
                        })),
                ),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let next = (this.selected != Some(id)).then_some(id);
                this.select(next, cx)
            }))
            .into_any_element()
    }

    fn render_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let entry = self.selected_entry()?.clone();
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let header = DetailsHeader::new(strings::PANEL_TITLE).on_collapse(move |_, _, cx| {
            let _ = screen.update(cx, |screen, cx| screen.select(None, cx));
        });
        let row = |id: &'static str, label: &'static str, value: String| {
            v_flex()
                .id(id)
                .test_support()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(label, cx))
                .child(div().text_size(metrics::FONT_SIZE_BODY).child(value))
        };
        let (date, _) = date_time(&entry.started_at);
        let (_, start) = date_time(&entry.started_at);
        let started = format!("{date} {start} UTC");
        let ended = entry.ended_at.as_deref().map_or_else(
            || {
                if entry.is_active() {
                    strings::NOW.to_owned()
                } else {
                    strings::UNKNOWN_END.to_owned()
                }
            },
            |ended| {
                let (date, time) = date_time(ended);
                format!("{date} {time} UTC")
            },
        );
        let block = widgets::block("log-panel-block", None, cx)
            .child(row(
                "log-panel-host",
                strings::FIELD_HOST,
                entry.title.clone(),
            ))
            .child(row(
                "log-panel-address",
                strings::FIELD_ADDRESS,
                format!("{}:{}", entry.address, entry.port),
            ))
            .when_some(entry.username.clone(), |this, user| {
                this.child(row("log-panel-user", strings::FIELD_USER, user))
            })
            .child(row(
                "log-panel-protocol",
                strings::FIELD_PROTOCOL,
                entry.protocol.to_uppercase(),
            ))
            .child(row("log-panel-started", strings::FIELD_STARTED, started))
            .child(row("log-panel-ended", strings::FIELD_ENDED, ended))
            .child(row("log-panel-end", strings::FIELD_END, end_text(&entry)))
            .child(row(
                "log-panel-device",
                strings::FIELD_DEVICE,
                if entry.this_device {
                    strings::THIS_DEVICE
                } else {
                    strings::OTHER_DEVICE
                }
                .to_owned(),
            ));
        Some(
            div()
                .id("log-panel")
                .test_support()
                .w(metrics::WIDTH_DETAILS)
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.colors.list)
                .child(
                    v_flex().size_full().child(header).child(
                        v_flex()
                            .id("log-panel-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(metrics::SPACE_4)
                            .pb(metrics::SPACE_4)
                            .gap(metrics::SPACE_3)
                            .child(block)
                            .child(widgets::caption(strings::NOT_RECORDED, cx)),
                    ),
                )
                .into_any_element(),
        )
    }
}

impl Render for LogsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: Vec<ConnectionLogView> = self.shown().into_iter().cloned().collect();
        let rows: Vec<AnyElement> = shown
            .iter()
            .map(|entry| self.render_row(entry, cx))
            .collect();
        let theme = cx.theme();
        let empty = (rows.is_empty() && self.loading.is_none()).then(|| {
            let (title, body) = if self.failed {
                (strings::LOAD_FAILED, None)
            } else if self.saved_only && !self.entries.is_empty() {
                (strings::NO_SAVED, None)
            } else {
                (strings::EMPTY_TITLE, Some(strings::EMPTY_BODY))
            };
            v_flex()
                .id("log-empty")
                .test_support()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(metrics::SPACE_2)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_HEADING)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .when_some(body, |this, body| {
                    this.child(div().text_color(theme.muted_foreground).child(body))
                })
        });
        let main = v_flex()
            .id("log-main")
            .flex_1()
            .min_w_0()
            .h_full()
            .px(CONTENT_PADDING)
            .pt(metrics::SPACE_4)
            .child(self.render_header(cx))
            .children(empty)
            .when(!rows.is_empty(), |this| {
                this.child(
                    v_flex()
                        .id("log-rows")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .children(rows),
                )
            });
        h_flex()
            .id("logs-screen")
            .test_support()
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .child(main)
            .children(self.render_panel(cx))
    }
}
