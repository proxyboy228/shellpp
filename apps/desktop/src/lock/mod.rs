//! Lock screens (UI 13.1, 13.2), needed to open the encrypted store.
//!
//! MINIMAL versions: first run (master password, then the recovery key) and
//! unlock. TODO(lock): "Use recovery key" (13.2), the full first-run copy
//! and layout, show/hide polish, Settings > Security (13.4).
//!
//! Passwords travel as `SecretString` to the core. The text field itself
//! (gpui-kit `InputState`) keeps its text in a plain rope until it is cleared,
//! which these screens do right after reading it.

mod first_run;
mod unlock;

pub use first_run::{FirstRunEvent, FirstRunScreen};
pub use unlock::UnlockScreen;

use gpui::prelude::*;
use gpui::{App, IntoElement, div, px};
use gpui_kit::component::{ActiveTheme as _, v_flex};

use crate::shell::title_bar::bare_title_bar;
use crate::theme::metrics;

/// The frame of a lock screen: the bare title bar (window buttons only)
/// and a centered column.
pub(crate) fn frame(id: &'static str, content: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .id(id)
        .size_full()
        .bg(cx.theme().background)
        .child(bare_title_bar(cx))
        .child(
            div().flex_1().flex().items_center().justify_center().child(
                v_flex()
                    .w(px(420.))
                    .gap(metrics::SPACE_4)
                    .p(metrics::SPACE_6)
                    .rounded(metrics::RADIUS_OVERLAY)
                    .bg(cx.theme().secondary)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(content),
            ),
        )
}
