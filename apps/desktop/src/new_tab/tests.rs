//! Headless tests of the new tab page, the Ctrl+K overlay and the pane
//! picker (GPUI `TestAppContext`, UI integration: real views in a test
//! window, keys and clicks through the window). Each test runs on its own
//! temporary profile (`TestProfile`: in-memory secret store and clock,
//! never the OS keychain or the permanent profile). Connections are only
//! written to the connection log; no host is contacted.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, EventEmitter,
    FocusHandle, InputEvent as _, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Render, Styled as _,
    TestAppContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::service::dto::RecordRef;
use shellpp_core::service::host_api::ConnectionLogStart;
use uuid::Uuid;

use super::model::Item;
use super::*;
use crate::state::AppState;
use crate::testing::TestProfile;

/// The root of a test window: a focusable frame that shows one view.
struct Frame {
    focus: FocusHandle,
    child: Option<AnyView>,
}

impl Render for Frame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("frame")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .children(self.child.clone())
    }
}

struct Ui {
    state: Entity<AppState>,
    /// Owns the profile directory (the clock moves the log forward).
    profile: TestProfile,
    handle: AnyWindowHandle,
    frame: Entity<Frame>,
    events: Rc<RefCell<Vec<NewTabEvent>>>,
}

fn map(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1000.), px(700.)),
        ))),
        ..Default::default()
    }
}

impl Ui {
    /// A test window over a fresh unlocked profile holding `hosts`.
    fn open(cx: &mut TestAppContext, hosts: &[Value]) -> (Self, Vec<Uuid>) {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
        });
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let state = cx.new(|cx| AppState::new(opened, cx));
        let ids = state.update(cx, |state, cx| {
            hosts
                .iter()
                .map(|host| state.save_host(None, map(host), cx).unwrap().record_id)
                .collect::<Vec<_>>()
        });
        let (handle, frame) = cx
            .update(|cx| {
                gpui_kit::open_window(window_options(), cx, |_, cx| {
                    cx.new(|cx| Frame {
                        focus: cx.focus_handle(),
                        child: None,
                    })
                })
            })
            .unwrap();
        let ui = Self {
            state,
            profile,
            handle,
            frame,
            events: Rc::default(),
        };
        ui.frame(cx, |_, _| {});
        (ui, ids)
    }

    /// Shows `view` (built in the window) in the frame and records its events.
    fn mount<V: Render + EventEmitter<NewTabEvent>>(
        &self,
        cx: &mut TestAppContext,
        build: impl FnOnce(Entity<AppState>, &mut Window, &mut Context<V>) -> V,
    ) -> Entity<V> {
        let state = self.state.clone();
        let frame = self.frame.clone();
        let events = self.events.clone();
        let view = cx
            .update_window(self.handle, |_, window, cx| {
                let view = cx.new(|cx| build(state, window, cx));
                cx.subscribe(&view, move |_, event: &NewTabEvent, _| {
                    events.borrow_mut().push(event.clone())
                })
                .detach();
                frame.update(cx, |frame, cx| {
                    frame.child = Some(view.clone().into());
                    cx.notify();
                });
                view
            })
            .unwrap();
        // The recent connections arrive from a background read.
        self.frame(cx, |_, _| {});
        self.frame(cx, |_, _| {});
        view
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

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        self.frame(cx, |window, cx| window.press(key, cx));
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<gpui::ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn focused(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        self.frame(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// Replaces the text of the search field `id`.
    fn type_search(&self, cx: &mut TestAppContext, id: &'static str, text: &str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            if !text.is_empty() {
                window.input(text, cx);
            }
        });
    }

    fn take_events(&self) -> Vec<NewTabEvent> {
        std::mem::take(&mut self.events.borrow_mut())
    }

    /// Writes a connection to the log (UI 6), then moves the clock on so
    /// the next one is newer.
    fn log(&self, cx: &mut TestAppContext, host: Option<Uuid>, address: &str, port: u16) {
        let (core, vault_id) = self.state.read_with(cx, |state, _| {
            (state.core().cloned().unwrap(), state.vault_id().unwrap())
        });
        core.connection_log_start(ConnectionLogStart {
            host: host.map(|record_id| RecordRef {
                vault_id,
                record_id,
            }),
            host_label: None,
            address: address.to_owned(),
            port,
            username: Some("root".to_owned()),
            os: None,
        })
        .unwrap();
        self.profile
            .memory()
            .unwrap()
            .clock
            .advance(Duration::from_secs(5));
        cx.run_until_parked();
    }

    /// Focuses the frame (what the overlay must give the focus back to).
    fn focus_frame(&self, cx: &mut TestAppContext) {
        let focus = self.frame.read_with(cx, |frame, _| frame.focus.clone());
        self.frame(cx, |window, cx| window.focus(&focus, cx));
    }

    fn frame_focused(&self, cx: &mut TestAppContext) -> bool {
        let focus = self.frame.read_with(cx, |frame, _| frame.focus.clone());
        self.frame(cx, |window, _| focus.is_focused(window))
    }

    /// A left click with modifiers held (Alt + click).
    fn click_with(
        &self,
        cx: &mut TestAppContext,
        id: impl Into<gpui::ElementId>,
        modifiers: Modifiers,
    ) {
        let id = id.into();
        self.frame(cx, |window, cx| {
            window.render_frame(cx);
            let position = window.find(id).bounds().center();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
        drop(self.profile);
    }
}

/// The keys of the rows of a list, in order.
fn keys(items: &[Item]) -> Vec<String> {
    items
        .iter()
        .filter_map(Item::row)
        .map(|row| row.key.to_string())
        .collect()
}

fn hosts() -> Vec<Value> {
    vec![
        json!({"address": "web.example.lan", "label": "Web", "tags": ["prod"], "ssh": {"username": "deploy"}}),
        json!({"address": "db.example.lan", "label": "DB"}),
        json!({"address": "mail.example.lan", "label": "Mail"}),
    ]
}

/// The page opens with the focus in the search field and lists Recent
/// connections from the connection log: newest first, one row per target,
/// updated when a connection is logged. Restore is disabled without a
/// snapshot of the previous launch; "Create a workspace" stays disabled.
#[gpui_kit::test]
fn page_opens_focused_with_recent_connections(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &hosts());
    let [web, db, _mail] = ids[..] else {
        unreachable!()
    };
    ui.log(cx, Some(web), "web.example.lan", 22);
    ui.log(cx, None, "203.0.113.4", 2222);
    ui.log(cx, Some(db), "db.example.lan", 22);
    // Again: the same host and the same quick-connect target are one row
    // each, at their newest place.
    ui.log(cx, Some(web), "web.example.lan", 22);
    ui.log(cx, None, "203.0.113.4", 2222);

    let page = ui.mount(cx, NewTabPage::new);
    assert!(ui.focused(cx, "new-tab-search"), "focus on open");
    let quick = "recent-203.0.113.4-2222".to_owned();
    let web_row = format!("recent-host-{web}");
    let db_row = format!("recent-host-{db}");
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [quick.clone(), web_row.clone(), db_row.clone()]
    );
    assert!(ui.visible(cx, "new-tab-recent-header"));
    assert!(ui.visible(cx, format!("new-tab-row-{web_row}")));
    let row = page.read_with(cx, |page, _| page.rows()[1].clone());
    assert_eq!(row.title.as_ref(), "Web");
    assert_eq!(row.detail.as_deref(), Some("Personal"));

    // A new connection moves its row to the top.
    ui.log(cx, Some(db), "db.example.lan", 22);
    ui.frame(cx, |_, _| {});
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [db_row.clone(), quick.clone(), web_row.clone()]
    );

    // Restore: disabled without a snapshot, then enabled by the shell.
    assert!(!page.read_with(cx, |page, _| page.restore_available()));
    ui.frame(cx, |window, cx| window.click("new-tab-restore", cx));
    assert!(
        ui.take_events().is_empty(),
        "a disabled Restore does nothing"
    );
    page.update(cx, |page, cx| page.set_restore_available(true, cx));
    ui.frame(cx, |window, cx| window.click("new-tab-restore", cx));
    assert_eq!(ui.take_events(), [NewTabEvent::Restore]);
    // Workspaces arrive in stage 9.
    ui.frame(cx, |window, cx| window.click("new-tab-workspace", cx));
    assert!(ui.take_events().is_empty());

    // Arrows move the selection, Enter connects, Alt+Enter in a new window.
    ui.frame(cx, |window, cx| {
        page.update(cx, |page, cx| page.focus_search(window, cx));
    });
    assert_eq!(
        page.read_with(cx, |page, _| page
            .selected_row()
            .map(|row| row.key.to_string())),
        Some(db_row.clone())
    );
    ui.press(cx, "down");
    ui.press(cx, "down");
    ui.press(cx, "down");
    assert_eq!(
        page.read_with(cx, |page, _| page
            .selected_row()
            .map(|row| row.key.to_string())),
        Some(web_row.clone()),
        "the selection stops at the last row"
    );
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: web,
            new_window: false,
        }]
    );
    ui.press(cx, "up");
    ui.press(cx, "alt-enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::QuickConnect {
            target: crate::hosts::QuickTarget {
                username: Some("root".into()),
                host: "203.0.113.4".into(),
                port: Some(2222),
            },
            new_window: true,
        }]
    );
    // A click connects too; with Alt in a new window (UI 2.4).
    ui.frame(cx, |window, cx| {
        window.click(format!("new-tab-row-{db_row}"), cx)
    });
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: db,
            new_window: false,
        }]
    );
    ui.click_with(cx, format!("new-tab-row-{web_row}"), Modifiers::alt());
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: web,
            new_window: true,
        }]
    );
    ui.finish(cx);
}

/// Typing replaces Recent connections with results: open tabs first, then
/// hosts; Enter on a tab brings it to the front; an address that no host
/// matches offers a quick connect.
#[gpui_kit::test]
fn typing_lists_open_tabs_first_then_hosts(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &hosts());
    let [web, db, _mail] = ids[..] else {
        unreachable!()
    };
    ui.log(cx, Some(db), "db.example.lan", 22);
    let other = cx
        .update(|cx| {
            gpui_kit::open_window(window_options(), cx, |_, cx| {
                cx.new(|cx| Frame {
                    focus: cx.focus_handle(),
                    child: None,
                })
            })
        })
        .unwrap()
        .0;
    let page = ui.mount(cx, NewTabPage::new);
    let handle = ui.handle;
    page.update(cx, |page, cx| {
        page.set_open_tabs(
            vec![
                OpenTab::new(handle, 7, "Local terminal").icon(gpui_kit::assets::IconName::Monitor),
                OpenTab::new(other, 2, "Web").host(Some(web)),
            ],
            cx,
        )
    });

    ui.type_search(cx, "new-tab-search", "web");
    let tab_row = format!("tab-{}-2", other.window_id().as_u64());
    let host_row = format!("host-{web}");
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [tab_row.clone(), host_row.clone()]
    );
    assert_eq!(
        page.read_with(cx, |page, _| page.items()[0].clone()),
        Item::Heading(strings::OPEN_TABS)
    );
    assert!(
        !ui.visible(cx, "new-tab-recent-header"),
        "results replace Recent connections"
    );
    // The host's tag finds its tab as well.
    ui.type_search(cx, "new-tab-search", "prod");
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [tab_row.clone(), host_row.clone()]
    );
    // Enter on the tab (selected first) brings that window's tab forward.
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::ActivateTab {
            window: other,
            tab: 2,
        }]
    );
    ui.press(cx, "down");
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: web,
            new_window: false,
        }]
    );

    // An address of no saved host: "Connect to ..." without saving.
    ui.type_search(cx, "new-tab-search", "ssh admin@192.0.2.30 -p 2200");
    assert_eq!(page.read_with(cx, |page, _| keys(page.items())), ["quick"]);
    assert!(ui.visible(cx, "new-tab-row-quick"));
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::QuickConnect {
            target: crate::hosts::QuickTarget {
                username: Some("admin".into()),
                host: "192.0.2.30".into(),
                port: Some(2200),
            },
            new_window: false,
        }]
    );
    // The address of a saved host connects to the host (UI 2.1).
    ui.type_search(cx, "new-tab-search", "db.example.lan");
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [format!("host-{db}")]
    );
    // Nothing found.
    ui.type_search(cx, "new-tab-search", "no such thing");
    assert!(ui.visible(cx, "new-tab-empty"));
    ui.press(cx, "enter");
    assert!(ui.take_events().is_empty());
    // Clearing the text brings Recent connections back.
    ui.type_search(cx, "new-tab-search", "");
    assert!(ui.visible(cx, "new-tab-recent-header"));
    assert_eq!(
        page.read_with(cx, |page, _| keys(page.items())),
        [format!("recent-host-{db}")]
    );
    let _ = cx.update_window(other, |_, window, _| window.remove_window());
    ui.finish(cx);
}

/// Ctrl+K's overlay: the focus goes to its search; Esc closes it and gives
/// the focus back; a click outside the panel closes it too, a click inside
/// does not; choosing a row emits its event.
#[gpui_kit::test]
fn overlay_searches_and_closes_with_escape(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &hosts());
    let web = ids[0];
    ui.log(cx, Some(web), "web.example.lan", 22);
    ui.focus_frame(cx);
    let handle = ui.handle;
    let overlay = ui.mount(cx, move |state, window, cx| {
        SearchOverlay::new(state, vec![OpenTab::new(handle, 4, "Web")], window, cx)
    });
    assert!(ui.focused(cx, "search-overlay-search"));
    // Empty: Recent connections under their heading.
    assert_eq!(
        overlay.read_with(cx, |overlay, _| overlay.items()[0].clone()),
        Item::Heading(strings::RECENT_CONNECTIONS)
    );
    assert_eq!(
        overlay.read_with(cx, |overlay, _| keys(overlay.items())),
        [format!("recent-host-{web}")]
    );
    ui.type_search(cx, "search-overlay-search", "web");
    assert_eq!(
        overlay.read_with(cx, |overlay, _| keys(overlay.items())),
        [
            format!("tab-{}-4", handle.window_id().as_u64()),
            format!("host-{web}")
        ]
    );
    ui.press(cx, "escape");
    assert_eq!(ui.take_events(), [NewTabEvent::Dismissed]);
    assert!(ui.frame_focused(cx), "the focus is back where it was");

    // A second overlay: a click inside the panel keeps it, outside closes it.
    ui.focus_frame(cx);
    let overlay = ui.mount(cx, move |state, window, cx| {
        SearchOverlay::new(state, Vec::new(), window, cx)
    });
    ui.frame(cx, |window, cx| window.click("search-overlay-search", cx));
    assert!(ui.take_events().is_empty());
    let height = ui.frame(cx, |window, _| {
        window.find("search-overlay").bounds().size.height
    });
    ui.frame(cx, |window, cx| {
        window.click_at("search-overlay", point(px(20.), height - px(20.)), cx)
    });
    assert_eq!(ui.take_events(), [NewTabEvent::Dismissed]);
    assert!(ui.frame_focused(cx));

    // Choosing: Enter on the first row.
    let _ = overlay;
    ui.mount(cx, move |state, window, cx| {
        SearchOverlay::new(state, Vec::new(), window, cx)
    });
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: web,
            new_window: false,
        }]
    );
    ui.finish(cx);
}

/// The picker of a new pane starts with the split pane's host, so Enter
/// opens another session to it; the host is not listed again under Recent
/// connections; typing searches hosts (never tabs); Esc asks to close the
/// pane.
#[gpui_kit::test]
fn pane_picker_offers_the_split_host_first(cx: &mut TestAppContext) {
    let (ui, ids) = Ui::open(cx, &hosts());
    let [web, db, _mail] = ids[..] else {
        unreachable!()
    };
    ui.log(cx, Some(db), "db.example.lan", 22);
    ui.log(cx, Some(web), "web.example.lan", 22);
    let picker = ui.mount(cx, move |state, window, cx| {
        CompactPicker::new(state, Some(PickerOrigin::Host(web)), window, cx)
    });
    assert!(ui.focused(cx, "pane-picker-search"));
    assert_eq!(
        picker.read_with(cx, |picker, _| keys(picker.items())),
        [format!("origin-{web}"), format!("recent-host-{db}")]
    );
    assert!(ui.visible(cx, format!("pane-picker-row-origin-{web}")));
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenHost {
            host_id: web,
            new_window: false,
        }]
    );

    ui.type_search(cx, "pane-picker-search", "mail");
    assert_eq!(
        picker.read_with(cx, |picker, _| keys(picker.items())),
        [format!("host-{}", ids[2])]
    );
    ui.press(cx, "escape");
    assert_eq!(ui.take_events(), [NewTabEvent::Dismissed]);

    // A local terminal's pane: a new local terminal with the same shell.
    let local = ui.mount(cx, |state, window, cx| {
        CompactPicker::new(
            state,
            Some(PickerOrigin::Local {
                shell_id: Some("pwsh".into()),
                title: "PowerShell 7".into(),
            }),
            window,
            cx,
        )
    });
    assert_eq!(
        local.read_with(cx, |picker, _| picker.rows()[0].title.to_string()),
        "PowerShell 7"
    );
    ui.press(cx, "enter");
    assert_eq!(
        ui.take_events(),
        [NewTabEvent::OpenLocal {
            shell_id: Some("pwsh".into()),
            new_window: false,
        }]
    );
    ui.finish(cx);
}
