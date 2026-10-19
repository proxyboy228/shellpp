//! The details panel frame (UI 1.3): the shell owns the container on the
//! right (380 px, `--width-details`; resizable from 320 to 560 px by
//! dragging its left edge; it narrows the main area and never overlaps it;
//! background `--bg-surface`, audit decision 7), and every object panel
//! renders [`DetailsHeader`] at its top: the title ("Host details" / "New
//! host"), the "…" menu with the object's actions and the collapse button.
//!
//! Element ids: `right-panel` (container), `panel-resize` (drag handle),
//! `panel-title`, `panel-menu` ("…"), `panel-close` (collapse).

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Context, IntoElement, ParentElement as _, Pixels, Render, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu};
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, IconName, h_flex};

use crate::i18n;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{controls, metrics};

/// The width of the panel for a drag that ends at `pointer_x`, given the
/// right edge of the panel: clamped to 320-560 px.
pub fn width_for_drag(panel_right: Pixels, pointer_x: Pixels) -> Pixels {
    clamp_width(panel_right - pointer_x)
}

/// Clamps a panel width to `--width-details-min` .. `--width-details-max`.
pub fn clamp_width(width: Pixels) -> Pixels {
    width
        .max(metrics::WIDTH_DETAILS_MIN)
        .min(metrics::WIDTH_DETAILS_MAX)
}

/// The value dragged by the resize handle.
#[derive(Clone, Copy, Debug)]
pub struct ResizeDetails;

/// The (invisible) drag preview of the resize handle.
#[derive(Debug)]
pub struct ResizePreview;

impl Render for ResizePreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The header of a details panel: title, "…" menu, collapse button.
#[derive(IntoElement)]
pub struct DetailsHeader {
    title: SharedString,
    menu: Option<MenuBuilder>,
    on_collapse: Option<ClickHandler>,
}

impl DetailsHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            menu: None,
            on_collapse: None,
        }
    }

    /// The "…" menu (UI 3.3 for a host). Without it the button is hidden.
    pub fn menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Rc::new(builder));
        self
    }

    /// The collapse button: closes the panel (UI 1.3; a draft is discarded).
    pub fn on_collapse(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_collapse = Some(Rc::new(handler));
        self
    }
}

impl std::fmt::Debug for DetailsHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetailsHeader")
            .field("title", &self.title)
            .field("menu", &self.menu.is_some())
            .finish_non_exhaustive()
    }
}

impl RenderOnce for DetailsHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let surface = theme.colors.list;
        let menu = self.menu.map(|builder| {
            Button::new("panel-menu")
                .custom(controls::ghost(cx))
                .icon(IconName::Ellipsis)
                .size(metrics::HEIGHT_CONTROL)
                .rounded(metrics::RADIUS_CONTROL)
                .tooltip(i18n::details::MORE_ACTIONS)
                .focus_ring(false)
                .focus_ring_visible(surface, cx)
                .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, window, cx| {
                    builder(menu, window, cx)
                })
        });
        let collapse = Button::new("panel-close")
            .custom(controls::ghost(cx))
            .icon(IconName::PanelRightClose)
            .size(metrics::HEIGHT_CONTROL)
            .rounded(metrics::RADIUS_CONTROL)
            .tooltip(i18n::details::COLLAPSE)
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .when_some(self.on_collapse, |button, handler| {
                button.on_click(move |event, window, cx| handler(event, window, cx))
            });
        h_flex()
            .id("panel-header")
            .test_support()
            .flex_none()
            .h(metrics::HEIGHT_CONTROL + metrics::SPACE_4)
            .pl(metrics::SPACE_4)
            .pr(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .items_center()
            .child(
                div()
                    .id("panel-title")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(self.title),
            )
            .children(menu)
            .child(collapse)
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    #[test]
    fn drag_width_is_clamped_to_the_design_system_limits() {
        let right = px(1280.);
        assert_eq!(width_for_drag(right, px(900.)), px(380.));
        assert_eq!(width_for_drag(right, px(1100.)), metrics::WIDTH_DETAILS_MIN);
        assert_eq!(width_for_drag(right, px(100.)), metrics::WIDTH_DETAILS_MAX);
    }
}
