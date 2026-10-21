//! Headless tests of the Logs screen (UI 11) over a temporary profile;
//! log entries are written through the core as the session layer does.

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, ElementId, TestAppContext, Window, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::service::dto::RecordRef;
use shellpp_core::service::host_api::{ConnectionEndReason, ConnectionLogStart};

use super::*;
use crate::testing::TestProfile;

struct Screen {
    state: Entity<AppState>,
    screen: Entity<LogsScreen>,
    handle: AnyWindowHandle,
}

impl Screen {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            cx.set_reduce_motion(true);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| LogsScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let this = Self {
            state,
            screen,
            handle,
        };
        this.frame(cx, |_, _| {});
        this
    }

    fn frame<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn start(&self, cx: &mut TestAppContext, host: Option<RecordRef>, address: &str) -> Uuid {
        self.core(cx)
            .connection_log_start(ConnectionLogStart {
                host,
                host_label: None,
                address: address.to_owned(),
                port: 22,
                username: Some("deploy".into()),
                os: None,
            })
            .unwrap()
    }

    fn reload(&self, cx: &mut TestAppContext) {
        self.screen.update(cx, |screen, cx| screen.reload(cx));
        self.frame(cx, |_, _| {});
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn entry(started: &str, ended: Option<&str>, reason: Option<&str>) -> ConnectionLogView {
    ConnectionLogView {
        record_id: Uuid::nil(),
        started_at: started.into(),
        ended_at: ended.map(Into::into),
        end_reason: reason.map(Into::into),
        exit_status: None,
        protocol: "ssh".into(),
        host: None,
        title: "web".into(),
        address: "192.0.2.1".into(),
        port: 22,
        username: None,
        os: None,
        saved: false,
        this_device: true,
    }
}

/// D68: the Date column in local time (here UTC+3), the day boundary by
/// local dates, the UTC times in its tooltip.
#[test]
fn the_date_column_reads_its_times() {
    crate::dates::set_fixed_display(Some(crate::dates::FixedDisplay {
        offset_minutes: 180,
    }));
    let ended = entry(
        "2026-10-21T18:04:05.000Z",
        Some("2026-10-21T19:10:00.000Z"),
        Some("closed"),
    );
    assert_eq!(
        date_lines(&ended),
        ("21.10.2026".to_owned(), "21:04 – 22:10".to_owned())
    );
    assert_eq!(
        date_tooltip(&ended).as_deref(),
        Some("2026-10-21 18:04:05 UTC – 2026-10-21 19:10:00 UTC")
    );
    assert_eq!(end_text(&ended), strings::END_CLOSED);
    // Over local midnight, and over a month's end.
    let next_day = entry(
        "2026-10-31T20:50:00.000Z",
        Some("2026-10-31T21:20:00.000Z"),
        Some("dropped"),
    );
    assert_eq!(date_lines(&next_day).1, "23:50 – 00:20 (+1d)");
    assert_eq!(end_text(&next_day), strings::END_DROPPED);
    // UTC midnight is no local one here.
    let same_day = entry(
        "2026-10-31T23:50:00.000Z",
        Some("2026-11-01T00:20:00.000Z"),
        Some("closed"),
    );
    assert_eq!(
        date_lines(&same_day),
        ("01.11.2026".to_owned(), "02:50 – 03:20".to_owned())
    );
    let active = entry("2026-10-21T18:04:05.000Z", None, None);
    assert_eq!(date_lines(&active).1, "21:04 – now");
    assert_eq!(
        date_tooltip(&active).as_deref(),
        Some("2026-10-21 18:04:05 UTC")
    );
    assert_eq!(end_text(&active), strings::ACTIVE);
    let unknown = entry("2026-10-21T18:04:05.000Z", None, Some("unknown"));
    assert_eq!(date_lines(&unknown).1, "21:04 – unknown");
    crate::dates::set_fixed_display(None);
    assert_eq!(end_text(&unknown), strings::END_UNKNOWN);
}

#[gpui_kit::test]
fn the_log_lists_sorts_saves_and_shows_details(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    assert!(ui.visible(cx, "log-empty"));
    let host = ui.state.update(cx, |state, cx| {
        let data: Map<String, Value> = json!({"address": "web.example.com", "label": "Web"})
            .as_object()
            .cloned()
            .unwrap();
        state.save_host(None, data, cx).unwrap()
    });
    let web = ui.start(
        cx,
        Some(RecordRef {
            vault_id: host.vault_id,
            record_id: host.record_id,
        }),
        "web.example.com",
    );
    ui.core(cx)
        .connection_log_end(web, ConnectionEndReason::ExitStatus, Some(2));
    let quick = ui.start(cx, None, "203.0.113.4");
    ui.reload(cx);
    let ids = |cx: &mut TestAppContext| {
        ui.screen.read_with(cx, |screen, _| {
            screen
                .shown()
                .iter()
                .map(|entry| entry.record_id)
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(ids(cx), [quick, web]);
    assert!(ui.visible(cx, row_id(web)));

    // The Date header turns the order.
    ui.click(cx, "log-sort");
    assert_eq!(ids(cx), [web, quick]);
    ui.click(cx, "log-sort");

    // The bookmark, and the Saved header's filter.
    ui.click(cx, saved_id(web));
    assert!(
        ui.core(cx)
            .connection_log_list()
            .unwrap()
            .iter()
            .any(|e| e.record_id == web && e.saved)
    );
    ui.click(cx, "log-saved-filter");
    assert_eq!(ids(cx), [web]);
    ui.click(cx, "log-saved-filter");
    assert_eq!(ids(cx).len(), 2);

    // A click shows the connection's details.
    ui.click(cx, row_id(web));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(web)
    );
    for id in [
        "log-panel",
        "log-panel-host",
        "log-panel-end",
        "log-panel-device",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    ui.click(cx, "panel-close");
    assert!(!ui.visible(cx, "log-panel"));
    ui.finish(cx);
}
