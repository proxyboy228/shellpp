//! Shared helpers of the headless UI tests (GPUI `TestAppContext`, no real
//! window). Every test opens its own temporary profile (`TestProfile`),
//! never the permanent one; [`Ui::finish`] shuts the app state down, which
//! deletes the profile, and checks that it is gone.
//!
//! Elements are found by element id (UI 0), never by text.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ElementId, Entity, Hsla, InputEvent as _,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, TestAppContext,
    Window, WindowBounds, WindowOptions, px, size,
};
use serde_json::{Map, Value};
use shellpp_desktop::app::{self, RootView};
use shellpp_desktop::shell::Shell;
use shellpp_desktop::state::{AppState, HostItem};
use shellpp_desktop::testing::TestProfile;
use shellpp_desktop::theme::{self, Appearance};
use uuid::Uuid;

/// The autosave delay plus a margin (`metrics::AUTOSAVE_DEBOUNCE` = 500 ms).
pub const AFTER_DEBOUNCE: Duration = Duration::from_millis(700);
/// Less than the autosave delay.
pub const BEFORE_DEBOUNCE: Duration = Duration::from_millis(200);

/// gpui-kit, our theme and key bindings, as `app::run` installs them.
pub fn setup(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(Appearance::Dark, cx);
        app::root::bind_keys(cx);
    });
}

pub fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("a JSON object")
}

/// One window over an unlocked temporary profile.
pub struct Ui {
    pub state: Entity<AppState>,
    pub handle: AnyWindowHandle,
    pub root: Entity<RootView>,
    /// The temporary profile directory (deleted by [`Ui::finish`]).
    pub profile: PathBuf,
}

impl Ui {
    /// An unlocked temporary profile with `records` saved through the app
    /// state (each a host's data map, or a group's when it has a `name`
    /// and no `address`), and a `width` x `height` window on it.
    pub fn open_sized(
        cx: &mut TestAppContext,
        width: f32,
        height: f32,
        records: &[Value],
    ) -> (Self, Vec<Uuid>) {
        setup(cx);
        let profile = TestProfile::new();
        let path = profile.path().to_path_buf();
        assert_temporary(&path);
        let opened = profile.into_unlocked();
        let state = cx.new(|cx| AppState::new(opened, cx));
        let ids = state.update(cx, |state, cx| {
            records
                .iter()
                .map(|record| {
                    let data = map(record.clone());
                    if data.contains_key("address") {
                        state
                            .save_host(None, data, cx)
                            .expect("host saved")
                            .record_id
                    } else {
                        state
                            .save_group(None, data, cx)
                            .expect("group saved")
                            .record_id
                    }
                })
                .collect::<Vec<_>>()
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(height)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| app::open_window_with(state.clone(), options, cx))
            .expect("test window");
        cx.run_until_parked();
        (
            Self {
                state,
                handle,
                root,
                profile: path,
            },
            ids,
        )
    }

    /// Like [`Ui::open_sized`] with the default window size (1280 x 800).
    pub fn open(cx: &mut TestAppContext, records: &[Value]) -> (Self, Vec<Uuid>) {
        Self::open_sized(cx, 1280., 800., records)
    }

    /// Runs `f` on the window after a fresh frame.
    pub fn frame<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .expect("the test window is open")
    }

    /// Turns motion off (the system's reduced-motion preference, design
    /// system 6: every duration 0 ms), for tests that compare painted
    /// colors: a panel fading in is not at full opacity yet.
    pub fn reduce_motion(&self, cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
    }

    /// Lets subscriptions and effects run.
    pub fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
    }

    /// Advances the test clock (autosave, toasts) and settles.
    pub fn wait(&self, cx: &mut TestAppContext, duration: Duration) {
        cx.run_until_parked();
        cx.executor().advance_clock(duration);
        cx.run_until_parked();
    }

    /// The shell of the unlocked window.
    pub fn shell(&self, cx: &mut TestAppContext) -> Entity<Shell> {
        cx.update(|cx| self.root.read(cx).shell().cloned())
            .expect("the shell is shown")
    }

    /// A stored host.
    pub fn host(&self, cx: &mut TestAppContext, id: Uuid) -> Option<HostItem> {
        cx.update(|cx| self.state.read(cx).host(id).cloned())
    }

    /// All stored hosts.
    pub fn hosts(&self, cx: &mut TestAppContext) -> Vec<HostItem> {
        cx.update(|cx| self.state.read(cx).hosts().as_ref().clone())
    }

    /// The selected record ids.
    pub fn selected(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        cx.update(|cx| self.state.read(cx).selection().ids().to_vec())
    }

    /// Closes the app state: the core closes, the profile folder is
    /// deleted. Fails if the folder is still there.
    pub fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
        assert!(
            !self.profile.exists(),
            "the temporary profile was not removed"
        );
    }
}

/// The profile of a test is a fresh folder under the OS temporary folder and
/// never inside (or around) the permanent profile.
pub fn assert_temporary(path: &Path) {
    assert!(path.starts_with(std::env::temp_dir()), "{path:?}");
    if let Some(permanent) = permanent_profile() {
        assert!(!path.starts_with(&permanent));
        assert!(!permanent.starts_with(path));
    }
}

/// The permanent profile folder of this user (`%LOCALAPPDATA%\shellpp` on
/// Windows), as a normal launch would resolve it.
pub fn permanent_profile() -> Option<PathBuf> {
    shellpp_desktop::dirs::permanent(
        shellpp_desktop::dirs::OsFamily::current(),
        &shellpp_desktop::dirs::BaseDirs::from_os(),
    )
    .ok()
    .map(|dirs| dirs.data)
}

/// Whether the element is on screen.
pub fn visible(window: &Window, id: impl Into<ElementId>) -> bool {
    window
        .try_find(id)
        .is_some_and(|snapshot| snapshot.visible())
}

/// The element's bounds (logical pixels).
pub fn bounds(window: &Window, id: impl Into<ElementId>) -> Bounds<Pixels> {
    window.find(id).bounds()
}

/// Every element on screen whose own id starts with `prefix`, top to
/// bottom, then left to right: (id, snapshot).
pub fn elements_with_prefix(
    window: &Window,
    prefix: &str,
) -> Vec<(String, gpui_kit::base::test_support::ElementSnapshot)> {
    let mut found: Vec<_> = gpui_kit::base::test_support::snapshots(window)
        .into_iter()
        .filter(|snapshot| snapshot.visible())
        .filter_map(|snapshot| {
            let id = snapshot.path().last()?.to_string();
            id.starts_with(prefix).then_some((id, snapshot))
        })
        .collect();
    found.sort_by(|(_, a), (_, b)| {
        let (a, b) = (a.bounds().origin, b.bounds().origin);
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    found.dedup_by(|(a, _), (b, _)| a == b);
    found
}

/// The elements whose id is exactly `prefix` followed by a number
/// (`session-tab-3`, `toast-1`), not their parts (`toast-1-close`).
pub fn numbered(
    window: &Window,
    prefix: &str,
) -> Vec<(String, gpui_kit::base::test_support::ElementSnapshot)> {
    elements_with_prefix(window, prefix)
        .into_iter()
        .filter(|(id, _)| {
            let rest = &id[prefix.len()..];
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        })
        .collect()
}

/// Clicks the element `id` with `modifiers` held (Ctrl or Shift selection,
/// which gpui-kit's `click` cannot send).
pub fn click_with(
    window: &mut Window,
    id: impl Into<ElementId>,
    modifiers: Modifiers,
    cx: &mut App,
) {
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
    window.render_frame(cx);
}

pub fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Default::default()
    }
}

pub fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn same_color(a: Hsla, b: Hsla) -> bool {
    (a.h - b.h).abs() < 0.002
        && (a.s - b.s).abs() < 0.002
        && (a.l - b.l).abs() < 0.002
        && (a.a - b.a).abs() < 0.002
}

/// Whether the element `id` is filled with our accent (gpui-kit `primary`,
/// owner's audit item 2): some painted quad inside its bounds has exactly
/// that solid color. A disabled control (opacity 0.45) does not count.
pub fn accent_filled(window: &Window, id: impl Into<ElementId>, cx: &App) -> bool {
    let accent = gpui_kit::component::ActiveTheme::theme(cx).primary;
    let element = window.find(id).bounds();
    let scale = window.scale_factor();
    let quads = window.painted_quads();
    quads.iter().any(|quad| {
        let Some(color) = quad.background.as_solid() else {
            return false;
        };
        if !same_color(color, accent) {
            return false;
        }
        let origin_x = quad.bounds.origin.x.0 / scale;
        let origin_y = quad.bounds.origin.y.0 / scale;
        let width = quad.bounds.size.width.0 / scale;
        let height = quad.bounds.size.height.0 / scale;
        // A fill, not a 1-2 px line, inside the element.
        width >= 8.
            && height >= 8.
            && origin_x >= element.origin.x.as_f32() - 1.
            && origin_y >= element.origin.y.as_f32() - 1.
            && origin_x + width <= element.origin.x.as_f32() + element.size.width.as_f32() + 1.
            && origin_y + height <= element.origin.y.as_f32() + element.size.height.as_f32() + 1.
    })
}

/// The number of accent-filled areas of at least `min_width` x
/// `min_height` logical pixels on screen (buttons, not bars or carets).
pub fn accent_fills(window: &Window, min_width: f32, min_height: f32, cx: &App) -> usize {
    let accent = gpui_kit::component::ActiveTheme::theme(cx).primary;
    let scale = window.scale_factor();
    window
        .painted_quads()
        .iter()
        .filter(|quad| {
            quad.background
                .as_solid()
                .is_some_and(|color| same_color(color, accent))
                && quad.bounds.size.width.0 / scale >= min_width
                && quad.bounds.size.height.0 / scale >= min_height
        })
        .count()
}

/// One entry of [`tree_snapshot`]: relative path, length, modification time.
pub type TreeEntry = (PathBuf, u64, Option<std::time::SystemTime>);

/// A snapshot of a directory tree: relative path, length and modification
/// time of every entry (`None` when the directory does not exist).
pub fn tree_snapshot(root: &Path) -> Option<Vec<TreeEntry>> {
    if !root.exists() {
        return None;
    }
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            entries.push((relative, meta.len(), meta.modified().ok()));
            if meta.is_dir() {
                pending.push(path);
            }
        }
    }
    entries.sort();
    Some(entries)
}
