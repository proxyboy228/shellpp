//! Right-hand panels (UI 1.3, 3): the host panel and the group panel.
//!
//! Saving rule (UI 1.3): a new object is a draft until its required field is
//! valid, then it is created; after that every change is saved automatically
//! after [`crate::theme::metrics::AUTOSAVE_DEBOUNCE`]; there is no save
//! button. [`autosave::Autosave`] holds that debounce.

pub mod autosave;
pub mod group_panel;
pub mod host_panel;

use gpui::prelude::*;
use gpui::{App, IntoElement, SharedString, div};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};

use crate::theme::metrics;

pub use group_panel::GroupPanel;
pub use host_panel::HostPanel;

/// What a panel asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    /// Close the panel (its pending edits were saved).
    Close,
    /// Show a toast at the bottom of the main area.
    Toast(SharedString),
}

/// The panel header: title and the collapse button (`panel-close`).
pub(crate) fn header(title: &'static str, close: Button, cx: &App) -> impl IntoElement {
    h_flex()
        .h(metrics::HEIGHT_TOP_BAR)
        .px(metrics::SPACE_4)
        .justify_between()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_HEADING)
                .text_color(cx.theme().foreground)
                .child(title),
        )
        .child(close.ghost().small().icon(IconName::Close))
}

/// A labelled field block.
pub(crate) fn field(
    label: &'static str,
    input: impl IntoElement,
    error: Option<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap(metrics::SPACE_1)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(input)
        .when_some(error, |this, error| {
            this.child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.danger)
                    .child(error),
            )
        })
}
