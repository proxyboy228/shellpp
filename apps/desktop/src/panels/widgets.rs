//! Building blocks shared by the host and group panels (the header is the
//! shell's [`crate::shell::details::DetailsHeader`]): blocks,
//! labelled fields, text inputs with one focus ring, select-like triggers,
//! segmented choices, color swatches, chips and text buttons.
//!
//! Colors come only from the theme (`cx.theme()`, design-system 7.4) and from
//! [`crate::theme::tokens`] / [`crate::theme::palette`]; sizes from
//! [`crate::theme::metrics`]. Every interactive element gets a stable id
//! registered for tests (`.test_support()`); helpers return `impl` types
//! because that wrapper changes the element type.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, Focusable as _, IntoElement, SharedString, Stateful,
    StyledText, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::base::FocusableExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use shellpp_core::service::dto::TagColor;

use crate::theme::{self, metrics, palette};

/// A color swatch in the Color field.
pub(crate) const SWATCH_SIZE: gpui::Pixels = px(28.);
/// Height of the inline dropdown lists (pickers).
pub(crate) const PICKER_MAX_HEIGHT: gpui::Pixels = px(240.);

/// An element with a stable id, registered for tests (the wrapper type
/// differs with the `test-support` feature, so helpers name it this way).
pub(crate) type Hit = gpui_kit::base::ObservedElement<Stateful<Div>>;

/// A `div` with a stable id that tests can find and click.
pub(crate) fn hit(id: impl Into<ElementId>) -> Hit {
    div().id(id).test_support()
}

/// An icon-only button (32 px square, hover `--bg-raised-hover`).
pub(crate) fn icon_button(id: impl Into<ElementId>, icon: IconName, cx: &App) -> Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    hit(id)
        .flex_none()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .child(Icon::new(icon).small())
}

/// A panel block (`--bg-raised`, radius 10, 16 px padding, 12 px gaps)
/// with an optional title.
pub(crate) fn block(id: &'static str, title: Option<&'static str>, cx: &App) -> Hit {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .flex_none()
        .p(metrics::SPACE_4)
        .gap(metrics::SPACE_3)
        .rounded(metrics::RADIUS_CARD)
        .bg(theme.secondary)
        .when_some(title, |this, title| {
            this.child(
                div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(text_with_paths(title, cx)),
            )
        })
}

/// A key fingerprint (`SHA256:…`) as one line: a narrow column
/// cuts it in the middle, so both ends stay readable, and it never breaks
/// at a `/` (run 3 polish, D64). The caller sets its font, size and color
/// (monospaced in a panel, the list's font in a list row).
pub(crate) fn fingerprint_line(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_middle()
        .child(text.into())
}

/// A field caption (12 px, `--text-secondary`); file paths in it are in
/// the monospaced font ([`text_with_paths`]).
pub(crate) fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().muted_foreground)
        .child(text_with_paths(text, cx))
}

/// The byte ranges of the file paths in `text` (`~/.ssh/known_hosts`): a
/// run of non-space characters from `~/`, without the punctuation that ends
/// the sentence after it.
pub(crate) fn path_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find("~/") {
        let start = from + found;
        let rest = &text[start..];
        let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let path = rest[..len].trim_end_matches(['.', ',', ';', ':', ')', '"', '\'']);
        ranges.push(start..start + path.len());
        from = start + len;
    }
    ranges
}

/// Text whose file paths are in the monospaced font: names of files and
/// paths appear only in descriptions and tooltips, in that font
/// (design-system 6 [decision]).
pub(crate) fn text_with_paths(text: impl Into<SharedString>, cx: &App) -> StyledText {
    let text = text.into();
    let mono = cx.theme().mono_font_family.clone();
    let overrides: Vec<_> = path_ranges(&text)
        .into_iter()
        .map(|range| (range, mono.clone()))
        .collect();
    StyledText::new(text).with_font_family_overrides(overrides)
}

/// The reason under an invalid field (12 px, `--danger`).
pub(crate) fn error_text(text: &'static str, cx: &App) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().danger)
        .child(text)
}

/// A labelled field: caption, control, and the reason when it is invalid.
pub(crate) fn field(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    error: Option<&'static str>,
    cx: &App,
) -> Div {
    v_flex()
        .w_full()
        .gap(metrics::SPACE_1)
        .child(caption(label, cx))
        .child(control)
        .when_some(error, |this, error| this.child(error_text(error, cx)))
}

/// Draws the one keyboard focus ring of design-system 6 around `control`:
/// `--focus-ring`, 2 px wide, 2 px outside the control's edge.
pub(crate) fn focus_ring(control: impl IntoElement, focused: bool, cx: &App) -> AnyElement {
    let ring = cx.theme().ring;
    let outset = metrics::FOCUS_RING_WIDTH + metrics::FOCUS_RING_OFFSET;
    div()
        .relative()
        .w_full()
        .child(control)
        .when(focused, |this| {
            this.child(
                div()
                    .absolute()
                    .top(-outset)
                    .left(-outset)
                    .right(-outset)
                    .bottom(-outset)
                    .rounded(metrics::RADIUS_CONTROL + outset)
                    .border(metrics::FOCUS_RING_WIDTH)
                    .border_color(ring),
            )
        })
        .into_any_element()
}

/// A single-line text input: 36 px, `--bg-input`, 1 px `--border-strong`
/// (`--danger` when invalid), radius 6, one focus ring.
pub(crate) fn text_input(
    state: &Entity<InputState>,
    id: impl Into<ElementId>,
    invalid: bool,
    window: &Window,
    cx: &App,
) -> AnyElement {
    text_input_with(Input::new(state), state, id, invalid, window, cx)
}

/// [`text_input`] with a prepared [`Input`] (prefix, suffix, masking).
pub(crate) fn text_input_with(
    input: Input,
    state: &Entity<InputState>,
    id: impl Into<ElementId>,
    invalid: bool,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let focused = state.read(cx).focus_handle(cx).is_focused(window);
    let danger = cx.theme().danger;
    // `Styled::h`, not the kit's inherent `Input::h` (multi-line only): the
    // kit's single-line height is `h_8` = 2 rem = 28 px with our 14 px rem,
    // and its text `text_sm` = 12.25 px.
    let input = gpui::Styled::h(input.id(id), metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_3)
        .text_size(metrics::FONT_SIZE_BODY)
        .bg(theme::tokens(cx).bg_input.hsla())
        .rounded(metrics::RADIUS_CONTROL)
        .focus_ring(false)
        .when(invalid, |this| this.border_color(danger));
    focus_ring(
        div()
            .relative()
            .w_full()
            .child(theme::controls::muted_placeholder(state, cx))
            .child(input),
        focused,
        cx,
    )
}

/// The face of a select-like control (36 px, `--bg-input`, `--border-strong`):
/// an optional leading element, the value, a chevron.
pub(crate) fn select_trigger(
    id: impl Into<ElementId>,
    leading: Option<AnyElement>,
    value: impl Into<SharedString>,
    open: bool,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let border = if open { theme.ring } else { theme.input };
    hit(id)
        .w_full()
        .h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_3)
        .flex()
        .items_center()
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(border)
        .bg(tokens.bg_input.hsla())
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.foreground)
        .cursor_pointer()
        .children(leading)
        .child(div().flex_1().min_w_0().truncate().child(value.into()))
        .child(
            Icon::new(if open {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .small()
            .text_color(theme.muted_foreground),
        )
}

/// One option of a segmented choice. The selected option takes the
/// `--accent-subtle` fill with `--text-primary` (like an active filter chip).
pub(crate) fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    hit(id)
        .flex_1()
        .min_w_0()
        .h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_2)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .text_size(metrics::FONT_SIZE_DENSE)
        .cursor_pointer()
        .truncate()
        .map(|this| {
            if selected {
                this.bg(tokens.accent_subtle.hsla())
                    .border_color(theme.primary)
                    .text_color(theme.foreground)
            } else {
                this.border_color(theme.input)
                    .text_color(theme.muted_foreground)
                    .hover(move |style| style.bg(hover))
            }
        })
        .child(label.into())
}

/// A row of segments.
pub(crate) fn segments(children: impl IntoIterator<Item = AnyElement>) -> Div {
    h_flex().w_full().gap(metrics::SPACE_1).children(children)
}

/// The id of a color swatch.
pub(crate) fn swatch_id(prefix: &str, color: TagColor) -> SharedString {
    format!("{prefix}-color-{}", color_name(color)).into()
}

/// The data-model name of a tile color.
pub(crate) fn color_name(color: TagColor) -> &'static str {
    match color {
        TagColor::Graphite => "graphite",
        TagColor::Jade => "jade",
        TagColor::Amber => "amber",
        TagColor::Coral => "coral",
        TagColor::Plum => "plum",
        TagColor::Moss => "moss",
        TagColor::Ocean => "ocean",
        TagColor::Rose => "rose",
    }
}

/// One color swatch (`--tag-*`, radius 8); the chosen one carries a white
/// check (`--text-on-tag`).
pub(crate) fn swatch(prefix: &str, color: TagColor, selected: bool, cx: &App) -> Hit {
    let fill = palette::tag_color(color).hsla();
    let ring = cx.theme().ring;
    hit(swatch_id(prefix, color))
        .flex_none()
        .size(SWATCH_SIZE)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_TILE)
        .bg(fill)
        .cursor_pointer()
        .when(selected, |this| {
            this.border_2().border_color(ring).child(
                Icon::new(IconName::Check)
                    .small()
                    .text_color(palette::TEXT_ON_TAG.hsla()),
            )
        })
}

/// A neutral tag chip (design-system 3.4): no fill, 1 px `--border-strong`,
/// `--text-secondary` text and a remove button.
pub(crate) fn chip(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    remove: impl IntoElement,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .test_support()
        .flex_none()
        .h(px(24.))
        .pl(metrics::SPACE_2)
        .pr(metrics::SPACE_1)
        .gap(metrics::SPACE_1)
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(theme.muted_foreground)
        .child(text.into())
        .child(remove)
}

/// The small remove button inside a chip or a row.
pub(crate) fn remove_button(id: impl Into<ElementId>, cx: &App) -> Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    hit(id)
        .flex_none()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(IconName::Close).xsmall())
}

/// A text button in `--accent-text` (links, "Share this host"). A disabled
/// one is shown at opacity 0.45 and does not react (design-system 6).
pub(crate) fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let hover = theme.link_hover;
    hit(id)
        .flex_none()
        .h(px(28.))
        .flex()
        .items_center()
        .gap(metrics::SPACE_1)
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.link)
        .map(|this| {
            if enabled {
                this.cursor_pointer()
                    .hover(move |style| style.text_color(hover))
            } else {
                this.opacity(metrics::OPACITY_DISABLED)
            }
        })
        .child(label.into())
}

/// A neutral full-width button inside a block (Add Telnet, Add variable):
/// `--bg-raised` on the block is invisible, so it uses a 1 px
/// `--border-strong` outline and the hover of buttons.
pub(crate) fn outline_button(
    id: impl Into<ElementId>,
    icon: Option<IconName>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    hit(id)
        .h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_3)
        .flex()
        .items_center()
        .justify_center()
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .when_some(icon, |this, icon| this.child(Icon::new(icon).small()))
        .child(label.into())
}

/// An inline dropdown list under a trigger (`--bg-overlay`, radius 12,
/// `--border-strong`), scrolling past [`PICKER_MAX_HEIGHT`].
pub(crate) fn dropdown_list(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .w_full()
        .p(metrics::SPACE_1)
        .gap(px(2.))
        .rounded(metrics::RADIUS_OVERLAY)
        .border_1()
        .border_color(theme.input)
        .bg(theme.popover)
}

/// One row of a dropdown list (hover `--bg-overlay-hover`).
pub(crate) fn dropdown_item(
    id: impl Into<ElementId>,
    leading: Option<AnyElement>,
    text: impl Into<SharedString>,
    secondary: Option<SharedString>,
    checked: bool,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let hover = theme.accent;
    hit(id)
        .w_full()
        .min_h(px(32.))
        .px(metrics::SPACE_2)
        .flex()
        .items_center()
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.popover_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .children(leading)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(text.into()))
                .when_some(secondary, |this, secondary| {
                    this.child(
                        div()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(theme.muted_foreground)
                            .child(secondary),
                    )
                }),
        )
        .when(checked, |this| {
            this.child(Icon::new(IconName::Check).small().text_color(theme.primary))
        })
}

/// A small tile with a white icon on a tile color (the Address block's OS
/// icon, group icons in pickers).
pub(crate) fn tile(icon: impl Into<Icon>, color: TagColor, size: gpui::Pixels) -> Div {
    div()
        .flex_none()
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(color).hsla())
        .child(
            Icon::new(icon)
                .small()
                .text_color(palette::TEXT_ON_TAG.hsla()),
        )
}

/// A one-line row: caption on the left, control on the right.
pub(crate) fn inline_row(
    label: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    h_flex()
        .w_full()
        .min_h(metrics::HEIGHT_CONTROL)
        .gap(metrics::SPACE_3)
        .items_center()
        .justify_between()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(cx.theme().foreground)
                .child(label.into()),
        )
        .child(control)
}

/// A thin divider between rows (`--border-subtle`).
pub(crate) fn divider(cx: &App) -> Div {
    div().w_full().h(px(1.)).bg(cx.theme().border)
}

/// A framed area inside a block for an inline form (the "+" forms of the
/// SSH block): 1 px `--border-strong`, radius 6, 12 px padding, 8 px gaps.
pub(crate) fn inset(id: impl Into<ElementId>, cx: &App) -> Hit {
    hit(id)
        .w_full()
        .flex()
        .flex_col()
        .p(metrics::SPACE_3)
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(cx.theme().input)
}

/// A linked item (identity, key, certificate): a muted icon, the type as a
/// caption over the name, and a remove button on the right.
pub(crate) fn linked_row(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    kind: &'static str,
    name: impl Into<SharedString>,
    remove: impl IntoElement,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    hit(id)
        .w_full()
        .min_h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_2)
        .flex()
        .items_center()
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .child(
            Icon::new(icon)
                .small()
                .flex_none()
                .text_color(theme.muted_foreground),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .py(metrics::SPACE_1)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(kind),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.foreground)
                        .child(name.into()),
                ),
        )
        .child(remove)
}

/// A neutral compact button for inline forms (Create, Import, Attach):
/// 32 px, 1 px `--border-strong`, hover of buttons; disabled at 0.45.
pub(crate) fn form_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
    cx: &App,
) -> Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    hit(id)
        .flex_none()
        .h(px(32.))
        .px(metrics::SPACE_3)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.foreground)
        .map(|this| {
            if enabled {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            } else {
                this.opacity(metrics::OPACITY_DISABLED)
            }
        })
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::path_ranges;

    #[test]
    fn paths_are_found_without_the_punctuation_after_them() {
        let text = "Adds the key to ~/.ssh/authorized_keys on the hosts.";
        let ranges = path_ranges(text);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&text[ranges[0].clone()], "~/.ssh/authorized_keys");
        let text = "The ~/.ssh folder, then ~/.ssh/config.";
        let found: Vec<&str> = path_ranges(text)
            .into_iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(found, ["~/.ssh", "~/.ssh/config"]);
        assert!(path_ranges("No paths here").is_empty());
        let text = "~/.ssh";
        assert_eq!(
            path_ranges(text),
            vec![std::ops::Range { start: 0, end: 6 }]
        );
    }
}
