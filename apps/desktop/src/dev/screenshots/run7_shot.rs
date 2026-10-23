//! The run 7 states of the screenshot run (plan, section 2): the top bar
//! with the update button of an update found (D100, item 4).
//!
//! The update state sets the update found of the run by hand (no update
//! source is asked); the reset clears it ([`reset`]), so the other states
//! keep a top bar without the button.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 1] = [ShotState::TopBarUpdate];

/// The version the update state shows.
const VERSION: &str = "0.2.0";

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

/// Clears what these states set (the reset of the demo window calls it).
pub fn reset(cx: &mut gpui::App) {
    cx.set_global(crate::updates::Found(None));
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    _state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    shell_of(cx, root, shot)?;
    match shot {
        ShotState::TopBarUpdate => {
            cx.update(|cx| {
                cx.set_global(crate::updates::Found(Some((
                    VERSION.to_owned(),
                    format!("https://github.com/proxyboy228/shellpp/releases/tag/v{VERSION}"),
                ))));
            });
            settle(cx, handle).await
        }
        _ => Err(failed(shot, "not a run 7 state")),
    }
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shown = |cx: &mut AsyncApp, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(id))
                .is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)
    };
    match shot {
        ShotState::TopBarUpdate if shown(cx, "top-bar-update")? => Ok(()),
        ShotState::TopBarUpdate => Err(failed(shot, "no update button in the top bar")),
        _ => Err(failed(shot, "not a run 7 state")),
    }
}
