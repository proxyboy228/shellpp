//! Pieces of the top bar (UI 1.1, design-system 6): 44 px, `--bg-base`, a
//! `--border-subtle` line at the bottom. Left to right: the app menu, the
//! persistent Vaults tab with the vault switcher arrow, the persistent SFTP
//! tab, the session tabs and "+"; on the right the notification center, the
//! terminal sidebar button (session tabs only) and the window buttons (drawn
//! by gpui-kit's `TitleBar`).
//!
//! Tabs: 32 px (`--height-tab`) inside the 44 px bar, 10 px inside
//! (`--padding-tab`), 4 px apart; `--text-secondary` text, hover
//! `--bg-raised-hover`, pressed `--bg-pressed`; the active tab gets the
//! `--accent-subtle` background, `--text-primary` text and the 2 px
//! `--accent` bar (audit decision 2), never an accent fill. The Vaults tab
//! and its vault switcher arrow are one pill: the active state spans both,
//! and the arrow is a click zone of its own with its own hover
//! (design-system 6 [decision]).
//!
//! Windows: gpui-kit's `TitleBar` makes the whole bar a drag region
//! (`HTCAPTION`), including whatever sits on top of it. Every interactive
//! child occludes it (`.occlude()`), or real clicks start a window drag.
//! The session tab strip occludes it as a whole, and its tabs block the
//! mouse except the wheel (`.block_mouse_except_scroll()`), so the wheel
//! over a tab scrolls the strip.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, IconName, Sizable as _, TitleBar, h_flex,
};

use super::sessions::SessionTab;
use crate::i18n;
use crate::panes::PaneStatus;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, controls, metrics};

/// Height of a tab and of the icon buttons in the bar.
const TAB_HEIGHT: gpui::Pixels = metrics::HEIGHT_TAB;
/// Width of the vault switcher arrow inside the Vaults pill.
const SWITCHER_WIDTH: gpui::Pixels = px(20.);
/// Width limits of a session tab (below the minimum the strip scrolls).
pub(super) const SESSION_TAB_MIN_WIDTH: gpui::Pixels = px(96.);
const SESSION_TAB_MAX_WIDTH: gpui::Pixels = px(220.);
/// The icon size in tabs and bar buttons.
const ICON_SIZE: gpui::Pixels = px(16.);

/// A title bar without tabs, for the lock screens (UI 13.2: only the window
/// buttons stay available) and the startup error; the shell's top bar adds
/// its tabs to it. Its close button (drawn by the kit on Linux) asks first
/// when sessions are active and saves the tabs for Restore, as the system
/// close does ([`super::request_close_window`]).
pub fn bare_title_bar(cx: &App) -> TitleBar {
    let theme = cx.theme();
    TitleBar::new()
        .on_close_window(|_, window, cx| super::request_close_window(window, cx))
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

/// The shared look of a tab: 32 px, radius 6, states of design-system 6.
fn tab_base<I: Into<ElementId>>(
    id: I,
    focus: &FocusHandle,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + use<I> {
    let tokens = theme::tokens(cx);
    tab_zone(id, focus, active, cx)
        .rounded(metrics::RADIUS_CONTROL)
        .when(active, |tab| {
            tab.bg(tokens.accent_subtle.hsla()).child(active_bar(cx))
        })
}

/// A tab's click zone without the active fill: text colors, hover and
/// pressed while inactive, focus. A plain tab adds the fill and the bar
/// ([`tab_base`]); the Vaults pill paints them around its two zones.
fn tab_zone<I: Into<ElementId>>(
    id: I,
    focus: &FocusHandle,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + use<I> {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    h_flex()
        .id(id)
        .test_support()
        .occlude()
        .relative()
        .flex_none()
        .h(TAB_HEIGHT)
        .px(metrics::PADDING_TAB)
        .gap(metrics::SPACE_2)
        .items_center()
        .cursor_pointer()
        .track_focus(focus)
        .tab_index(0)
        .text_size(metrics::FONT_SIZE_BODY)
        .focus_ring_visible(theme.title_bar, cx)
        .when(active, |tab| tab.text_color(theme.tab_active_foreground))
        .when(!active, |tab| {
            tab.text_color(theme.tab_foreground)
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
        })
}

/// A persistent tab (Vaults, SFTP): icon and label.
pub fn top_tab<I: Into<ElementId>, L: Into<SharedString>>(
    id: I,
    focus: &FocusHandle,
    icon: Lucide,
    label: L,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + use<I, L> {
    tab_base(id, focus, active, cx)
        .child(Icon::new(icon).size(ICON_SIZE))
        .child(label.into())
}

/// The Vaults tab's click zone (icon and label): the left part of the
/// Vaults pill ([`vaults_pill`]). Both zones keep radius 6 (the kit button
/// of the arrow rounds all four corners), so their hovers are two rounded
/// zones inside the pill.
pub fn vaults_tab<I: Into<ElementId>, L: Into<SharedString>>(
    id: I,
    focus: &FocusHandle,
    icon: Lucide,
    label: L,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + use<I, L> {
    tab_zone(id, focus, active, cx)
        .rounded(metrics::RADIUS_CONTROL)
        // The arrow sits right after the label: less padding between them.
        .pr(metrics::SPACE_1)
        .child(Icon::new(icon).size(ICON_SIZE))
        .child(label.into())
}

/// The Vaults pill (UI 1.1, design-system 6 [decision]): the tab zone and
/// the vault switcher arrow in one 32 px pill. While Vaults is active, the
/// `--accent-subtle` fill and the `--accent` bar span both zones.
pub fn vaults_pill(
    tab: impl IntoElement,
    switcher: impl IntoElement,
    active: bool,
    cx: &App,
) -> impl IntoElement {
    let tokens = theme::tokens(cx);
    h_flex()
        .id("vaults-pill")
        .test_support()
        .relative()
        .flex_none()
        .h(TAB_HEIGHT)
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .when(active, |pill| pill.bg(tokens.accent_subtle.hsla()))
        .child(tab)
        .child(switcher)
        .when(active, |pill| pill.child(active_bar(cx)))
}

/// A session tab (UI 1.1): the OS icon (the close button replaces it on the
/// active tab and under the pointer), the name, the 8 px state dot right
/// after it (design-system 3.3; none while the tab has no session), a key
/// in `--warning` while a pane waits for the answer to a signature question
/// (`asks_signature`, D68) and the session type icon on the right. `rename` replaces the name with the
/// inline Rename field (UI 4.4). `close` is the tab's close button
/// ([`session_tab_close`] with the caller's click handler); the caller
/// attaches the tab's own click handler.
pub fn session_tab<C: IntoElement>(
    tab: &SessionTab,
    state: Option<PaneStatus>,
    asks_signature: bool,
    rename: Option<AnyElement>,
    focus: &FocusHandle,
    active: bool,
    close: C,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + use<C> {
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
    let title = match rename {
        Some(field) => div().flex_1().min_w_0().child(field).into_any_element(),
        None => div()
            .flex_shrink(1.)
            .min_w_0()
            .truncate()
            .child(tab.title().clone())
            .into_any_element(),
    };
    tab_base(tab.element_id(), focus, active, cx)
        // Inside the occluding strip: the wheel reaches the strip's scroll.
        .block_mouse_except_scroll()
        .group(group)
        .flex_shrink(1.)
        .min_w(SESSION_TAB_MIN_WIDTH)
        .max_w(SESSION_TAB_MAX_WIDTH)
        .child(icon_slot)
        .child(title)
        .children(state.map(|state| state_dot(tab, state, cx)))
        .when(asks_signature, |this| {
            this.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-signature",
                        tab.element_id()
                    )))
                    .test_support()
                    .flex_none()
                    .text_color(theme.warning)
                    .child(Icon::new(Lucide::KeyRound).size(px(14.)))
                    .tooltip(|window, cx| {
                        Tooltip::new(crate::session_ui::strings::SIGN_WAITING).build(window, cx)
                    }),
            )
        })
        // The type icon keeps to the right edge; the dot stays by the name.
        .child(div().flex_1())
        .children(tab.kind().type_icon().map(|icon| {
            div()
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(Icon::new(icon).size(px(14.)))
        }))
}

/// The state dot of a session tab (design-system 3.3): 8 px, connecting
/// `--warning`, connected `--success`, drop `--danger`, ended with a
/// non-zero code `--text-muted`.
fn state_dot(tab: &SessionTab, state: PaneStatus, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let color = match state {
        PaneStatus::Connected => theme.success,
        PaneStatus::Connecting => theme.warning,
        PaneStatus::Dropped => theme.danger,
        // Inactive (UI 1.1): `--text-muted`.
        PaneStatus::Ended => crate::theme::tokens(cx).text_muted.hsla(),
    };
    div()
        .id(tab.dot_element_id())
        .test_support()
        .flex_none()
        .size(metrics::SIZE_STATUS_DOT)
        .rounded_full()
        .bg(color)
        .into_any_element()
}

/// The close button of a session tab (16 px, inside the icon slot).
pub fn session_tab_close(
    tab: &SessionTab,
    cx: &App,
) -> impl StatefulInteractiveElement + ParentElement + IntoElement + use<> {
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

/// A ghost icon button of the bar (32 px square, radius 6, hover
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

/// A bar button of a later stage, shown disabled (D30; design-system 6): no
/// clicks, no hover, the whole button once at `--opacity-disabled`. The
/// icon keeps its normal color (the kit's own disabled style would fade it
/// a second time).
pub fn disabled(button: Button, cx: &App) -> Button {
    use gpui_kit::component::Disableable as _;
    button
        .disabled(true)
        .text_color(cx.theme().muted_foreground)
        .opacity(metrics::OPACITY_DISABLED)
}

/// The vault switcher arrow: the right zone of the Vaults pill, with its
/// own hover even inside the active pill.
pub fn vault_switcher_button(cx: &App) -> Button {
    let theme = cx.theme();
    Button::new("vault-switcher")
        .custom(controls::ghost(cx))
        .icon(Icon::new(IconName::ChevronDown).size(px(14.)))
        .w(SWITCHER_WIDTH)
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
