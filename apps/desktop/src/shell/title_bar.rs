//! Pieces of the top bar (UI 1.1, design-system 6): 44 px, `--bg-base`, a
//! `--border-subtle` line at the bottom. Left to right: the app menu, the
//! persistent Vaults tab with the vault switcher arrow, the persistent SFTP
//! tab, the session tabs and "+"; on the right the notification center, the
//! terminal sidebar button (session tabs only) and the window buttons (drawn
//! by gpui-kit's `TitleBar`).
//!
//! Tabs: `--text-secondary` text, hover `--bg-raised-hover`, pressed
//! `--bg-pressed`; the active tab gets the `--accent-subtle` background,
//! `--text-primary` text and the 2 px `--accent` bar (audit decision 2),
//! never an accent fill.
//!
//! Windows: gpui-kit's `TitleBar` makes the whole bar a drag region
//! (`HTCAPTION`), including whatever sits on top of it. Every interactive
//! child occludes it (`.occlude()`), or real clicks start a window drag.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, IconName, Sizable as _, TitleBar, h_flex,
};

use super::sessions::SessionTab;
use crate::i18n;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, controls, metrics};

/// Height of a tab and of the icon buttons in the bar.
const TAB_HEIGHT: gpui::Pixels = metrics::HEIGHT_CONTROL;
/// Width limits of a session tab.
const SESSION_TAB_MIN_WIDTH: gpui::Pixels = px(96.);
const SESSION_TAB_MAX_WIDTH: gpui::Pixels = px(220.);
/// The icon size in tabs and bar buttons.
const ICON_SIZE: gpui::Pixels = px(16.);

/// A title bar without tabs, for the lock screens (UI 13.2: only the window
/// buttons stay available) and the startup error.
pub fn bare_title_bar(cx: &App) -> TitleBar {
    let theme = cx.theme();
    TitleBar::new()
        .h(metrics::HEIGHT_TOP_BAR)
        // macOS keeps the kit's room for the system buttons on the left.
        .when(!cfg!(target_os = "macos"), |bar| bar.pl(metrics::SPACE_1))
        .bg(theme.title_bar)
        .border_color(theme.title_bar_border)
}

/// The 2 px `--accent` bar under the active tab.
fn active_bar(cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .bottom_0()
        .left(metrics::SPACE_3)
        .right(metrics::SPACE_3)
        .h(metrics::SIZE_TAB_INDICATOR)
        .rounded(metrics::SIZE_TAB_INDICATOR)
        .bg(cx.theme().primary)
}

/// The shared look of a tab: 36 px, radius 6, states of design-system 6.
fn tab_base(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    h_flex()
        .id(id)
        .test_support()
        .occlude()
        .relative()
        .flex_none()
        .h(TAB_HEIGHT)
        .px(metrics::SPACE_3)
        .gap(metrics::SPACE_2)
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .cursor_pointer()
        .track_focus(focus)
        .tab_index(0)
        .text_size(metrics::FONT_SIZE_BODY)
        .focus_ring_visible(theme.title_bar, cx)
        .when(active, |tab| {
            tab.bg(tokens.accent_subtle.hsla())
                .text_color(theme.tab_active_foreground)
                .child(active_bar(cx))
        })
        .when(!active, |tab| {
            tab.text_color(theme.tab_foreground)
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
        })
}

/// A persistent tab (Vaults, SFTP): icon and label.
pub fn top_tab(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    icon: Lucide,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    tab_base(id, focus, active, cx)
        .child(Icon::new(icon).size(ICON_SIZE))
        .child(label.into())
}

/// A session tab: the OS icon (the close button replaces it on the active
/// tab and under the pointer), the name, the session type icon on the right.
/// `close` is the tab's close button ([`session_tab_close`] with the
/// caller's click handler); the caller attaches the tab's own click handler.
pub fn session_tab(
    tab: &SessionTab,
    focus: &FocusHandle,
    active: bool,
    close: impl IntoElement,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    let theme = cx.theme();
    let group: SharedString = format!("{}-group", tab.element_id()).into();
    let icon_slot = div()
        .relative()
        .flex_none()
        .size(ICON_SIZE)
        .child(
            div()
                .absolute()
                .inset_0()
                .when(active, |icon| icon.invisible())
                .group_hover(group.clone(), |style| style.invisible())
                .child(Icon::new(tab.icon()).size(ICON_SIZE)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .when(!active, |close| close.invisible())
                .group_hover(group.clone(), |style| style.visible())
                .child(close),
        );
    tab_base(tab.element_id(), focus, active, cx)
        .group(group)
        .flex_shrink(1.)
        .min_w(SESSION_TAB_MIN_WIDTH)
        .max_w(SESSION_TAB_MAX_WIDTH)
        .px(metrics::SPACE_2)
        .child(icon_slot)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(tab.title().clone()),
        )
        .children(tab.kind().type_icon().map(|icon| {
            div()
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(Icon::new(icon).size(px(14.)))
        }))
}

/// The close button of a session tab (16 px, inside the icon slot).
pub fn session_tab_close(
    tab: &SessionTab,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
    let theme = cx.theme();
    let hover = theme.secondary_active;
    div()
        .id(tab.close_element_id())
        .test_support()
        .size(ICON_SIZE)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(IconName::Close).xsmall())
}

/// A ghost icon button of the bar (36 px square, radius 6, hover
/// `--bg-raised-hover`, pressed `--bg-pressed`, icon `--text-secondary`).
pub fn icon_button(id: impl Into<ElementId>, icon: impl Into<Icon>, cx: &App) -> Button {
    let theme = cx.theme();
    Button::new(id)
        .custom(controls::ghost(cx))
        .icon(icon.into().size(ICON_SIZE))
        .size(TAB_HEIGHT)
        .rounded(metrics::RADIUS_CONTROL)
        .focus_ring(false)
        .focus_ring_visible(theme.title_bar, cx)
}

/// The vault switcher arrow next to the Vaults tab (narrow button).
pub fn vault_switcher_button(cx: &App) -> Button {
    let theme = cx.theme();
    Button::new("vault-switcher")
        .custom(controls::ghost(cx))
        .icon(Icon::new(IconName::ChevronDown).size(px(14.)))
        .w(px(20.))
        .h(TAB_HEIGHT)
        .rounded(metrics::RADIUS_CONTROL)
        .tooltip(i18n::top_bar::SWITCH_VAULT)
        .focus_ring(false)
        .focus_ring_visible(theme.title_bar, cx)
}

/// The bell of the notification center with its unread counter.
pub fn notifications_button(unread: usize, cx: &App) -> Button {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let counter: Option<AnyElement> = (unread > 0).then(|| {
        div()
            .id("notification-count")
            .test_support()
            .absolute()
            .top(px(4.))
            .right(px(4.))
            .min_w(px(16.))
            .h(px(16.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            // Inverse colors: a neutral, always legible counter (the accent
            // is reserved for the primary action, the danger color for
            // dangerous actions).
            .bg(theme.foreground)
            .text_color(tokens.bg_base.hsla())
            .text_size(metrics::FONT_SIZE_CAPTION)
            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
            .child(i18n::notifications::counter(unread))
            .into_any_element()
    });
    icon_button("top-bar-notifications", IconName::Bell, cx)
        .relative()
        .accessibility_label(i18n::notifications::bell_label(unread))
        .children(counter)
}
