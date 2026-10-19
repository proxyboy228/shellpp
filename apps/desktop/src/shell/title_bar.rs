//! The top bar (UI 1.1): app menu, the persistent Vaults and SFTP tabs,
//! session tabs (later) and "+", then notifications and the window buttons
//! (drawn by gpui-kit's `TitleBar`).
//!
//! Windows: gpui-kit's `TitleBar` makes the whole bar a drag region
//! (`HTCAPTION`), including whatever sits on top of it. Every interactive
//! child must be wrapped in `.occlude()` or real clicks start a window drag.
//!
//! TODO(shell): the vault switcher arrow on the Vaults tab, session tabs with
//! OS icons and close buttons, the notification center popover with its
//! unread counter, the full app menu of UI 1.4.

use gpui::prelude::*;
use gpui::{
    App, ElementId, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, TitleBar, h_flex};

use crate::theme::{self, metrics};

/// A title bar without tabs, for the lock screens (UI 13.2: only the window
/// buttons stay available).
pub fn bare_title_bar(cx: &App) -> TitleBar {
    let theme = cx.theme();
    TitleBar::new()
        .h(metrics::HEIGHT_TOP_BAR)
        .bg(theme.title_bar)
        .border_color(theme.title_bar_border)
}

/// One persistent tab of the top bar: an icon and a label. The active tab
/// gets the `--accent-subtle` background, never an accent fill.
pub fn top_tab(
    id: impl Into<ElementId>,
    icon: Lucide,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    h_flex()
        .id(id)
        .test_support()
        .h(metrics::HEIGHT_CONTROL - metrics::SPACE_1)
        .px(metrics::SPACE_3)
        .gap(metrics::SPACE_2)
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .cursor_pointer()
        .text_size(metrics::FONT_SIZE_BODY)
        .when(active, |tab| {
            tab.bg(tokens.accent_subtle.hsla())
                .text_color(theme.foreground)
        })
        .when(!active, |tab| {
            tab.text_color(theme.muted_foreground)
                .hover(move |style| style.bg(hover))
        })
        .child(Icon::new(icon).small())
        .child(label.into())
}

/// A square icon button of the top bar.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    div()
        .id(id)
        .test_support()
        .size(metrics::HEIGHT_CONTROL - metrics::SPACE_1)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .cursor_pointer()
        .text_color(theme.muted_foreground)
        .hover(move |style| style.bg(hover))
        .child(icon.into().small())
}

/// The bell of the notification center (UI 1.5). TODO(shell): popover and counter.
pub fn notifications_button(
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    icon_button("top-bar-notifications", IconName::Bell, cx)
}
