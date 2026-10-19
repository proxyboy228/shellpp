//! The "In development" placeholder (UI 0; owner's audit items 9 and 10;
//! audit decision 5) for sections and tabs whose screen is not built yet:
//! the title (16 px, 600, `--text-primary`) and "In development" (14 px,
//! `--text-secondary`), centered.

use gpui::prelude::*;
use gpui::{App, ElementId, IntoElement, SharedString, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use crate::i18n;
use crate::theme::metrics;

/// A centered placeholder with the section `title` and "In development".
pub fn placeholder(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .test_support()
        .size_full()
        .items_center()
        .justify_center()
        .gap(metrics::SPACE_2)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_HEADING)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(theme.foreground)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.muted_foreground)
                .child(i18n::placeholder::TITLE),
        )
}
