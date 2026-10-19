//! The tag filter of the action bar (UI 2.2, owner's step 1.1, lead
//! decision 5): a button with the tag icon opens a dropdown with every tag
//! and its host count, a search field and a checkbox per tag (several tags
//! can be on), plus "Clear". The tags that are on show as chips next to the
//! button; the chips that do not fit collapse into a "+N" chip, which opens
//! the dropdown. The button shows the active state while any tag is on.
//!
//! [`TagFilterMenu`] is the dropdown's content (its own entity: the search
//! field keeps its text and focus); the Hosts screen owns the filter and
//! feeds the menu its options. The chip fitting ([`chips_that_fit`]) is pure
//! and unit-tested; the chip and button widths it works with are measured
//! with the window's text system (`hosts::view`).

use gpui::{
    App, AppContext as _, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, Global,
    InteractiveElement as _, IntoElement, KeyBinding, NoAction, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};

use super::controls::{ControlButton, ControlKind, ICON_SIZE};
use super::strings;
use crate::state::hosts_view::TagCount;
use crate::theme::{self, metrics};

/// Width of the dropdown.
pub const MENU_WIDTH: f32 = 260.;
/// The tallest the tag list grows before it scrolls.
const LIST_MAX_HEIGHT: f32 = 280.;
/// Height of a chip (inside the 36 px action bar).
pub const CHIP_HEIGHT: f32 = 28.;
/// Size of the remove icon of a chip.
const CHIP_ICON: f32 = 12.;
/// Height of a tag row of the dropdown.
const ROW_HEIGHT: f32 = 32.;
/// Key context of the dropdown.
const KEY_CONTEXT: &str = "TagFilterMenu";

/// Inside the dropdown, Enter and Space belong to its search field and its
/// checkboxes: Enter turns the first match on, Space types a space (tags may
/// hold spaces) or toggles a focused checkbox. Without these bindings the
/// popover around the dropdown takes both keys as its Confirm and closes.
pub fn bind_keys(cx: &mut App) {
    struct Bound;
    impl Global for Bound {}
    if cx.has_global::<Bound>() {
        return;
    }
    cx.set_global(Bound);
    cx.bind_keys([
        KeyBinding::new("enter", NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("space", NoAction, Some(KEY_CONTEXT)),
    ]);
}

/// Width of a chip of a selected tag around its text: padding, the 1 px
/// border, the gap and the remove icon.
pub fn selected_chip_width(text: f32) -> f32 {
    text + 2. * metrics::SPACE_3.as_f32() + 2. + metrics::SPACE_1.as_f32() + CHIP_ICON
}

/// Width of the "+N" chip around its text.
pub fn more_chip_width(text: f32) -> f32 {
    text + 2. * metrics::SPACE_3.as_f32() + 2.
}

/// Room the chips take after the tag button: `lead` before the first chip,
/// `between` between chips; with `shown < widths.len()` the "+N" chip
/// (`more(hidden)`) follows.
pub fn chips_room(
    widths: &[f32],
    shown: usize,
    lead: f32,
    between: f32,
    more: impl Fn(usize) -> f32,
) -> f32 {
    if widths.is_empty() {
        return 0.;
    }
    let shown = shown.min(widths.len());
    let mut room = lead + widths[..shown].iter().sum::<f32>();
    room += between * shown.saturating_sub(1) as f32;
    if shown < widths.len() {
        if shown > 0 {
            room += between;
        }
        room += more(widths.len() - shown);
    }
    room
}

/// How many chips (in order) fit in `room`; the rest collapse into "+N".
/// At least the "+N" chip is always shown when a tag is on.
pub fn chips_that_fit(
    widths: &[f32],
    room: f32,
    lead: f32,
    between: f32,
    more: impl Fn(usize) -> f32,
) -> usize {
    (0..=widths.len())
        .rev()
        .find(|&shown| chips_room(widths, shown, lead, between, &more) <= room + 0.5)
        .unwrap_or(0)
}

/// Stable element id of the checkbox of a tag in the dropdown.
pub fn tag_option_id(tag: &str) -> SharedString {
    format!("tag-option-{}", tag.to_lowercase()).into()
}

/// What the dropdown asks the Hosts screen to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagFilterEvent {
    /// Turn a tag (lower case) on or off.
    Toggle(String),
    /// Turn every tag off.
    Clear,
}

/// The dropdown of the tag filter.
pub struct TagFilterMenu {
    search: Entity<InputState>,
    options: Vec<TagCount>,
    /// The tags that are on (lower case).
    selected: Vec<String>,
    _subscription: Subscription,
}

impl std::fmt::Debug for TagFilterMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TagFilterMenu")
            .field("options", &self.options.len())
            .field("selected", &self.selected.len())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<TagFilterEvent> for TagFilterMenu {}

impl Focusable for TagFilterMenu {
    /// The search field: it takes the focus when the dropdown opens.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

impl TagFilterMenu {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        bind_keys(cx);
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::TAG_SEARCH_PLACEHOLDER));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.notify(),
                // Enter turns the first match on or off.
                InputEvent::PressEnter { .. } => {
                    if let Some(first) = this.visible_options(cx).first() {
                        cx.emit(TagFilterEvent::Toggle(first.key()));
                    }
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );
        Self {
            search,
            options: Vec::new(),
            selected: Vec::new(),
            _subscription: subscription,
        }
    }

    /// Every tag with its host count, and the tags that are on.
    pub fn set_options(
        &mut self,
        options: Vec<TagCount>,
        selected: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.options != options || self.selected != selected {
            self.options = options;
            self.selected = selected;
            cx.notify();
        }
    }

    /// Empties the search field (when the dropdown opens again).
    pub fn reset_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
            cx.notify();
        }
    }

    /// The search text.
    pub fn search_text(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    /// The options that match the search (case-insensitive substring).
    pub fn visible_options(&self, cx: &App) -> Vec<TagCount> {
        let query = self.search_text(cx).trim().to_lowercase();
        self.options
            .iter()
            .filter(|option| query.is_empty() || option.key().contains(&query))
            .cloned()
            .collect()
    }

    fn is_selected(&self, option: &TagCount) -> bool {
        self.selected.contains(&option.key())
    }

    fn render_search(&self, window: &Window, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let focused = self
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        h_flex()
            .id("tag-filter-search-field")
            .test_support()
            .relative()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .flex_none()
            .gap(metrics::SPACE_2)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(tokens.bg_input.hsla())
            .border_1()
            .border_color(tokens.border_strong.hsla())
            .child(
                Icon::new(IconName::Search)
                    .with_size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(theme::controls::muted_placeholder(&self.search, cx))
                    .child(
                        Input::new(&self.search)
                            .id("tag-filter-search")
                            .appearance(false)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .when(focused, |this| {
                this.child(super::controls::focus_ring(
                    metrics::RADIUS_CONTROL,
                    px(1.),
                    cx,
                ))
            })
    }

    fn render_option(&self, option: &TagCount, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hover = theme.accent;
        let key = option.key();
        let checked = self.is_selected(option);
        div()
            .w_full()
            .rounded(metrics::RADIUS_CONTROL)
            .hover(move |style| style.bg(hover))
            .child(
                Checkbox::new(ElementId::Name(tag_option_id(&option.name)))
                    .checked(checked)
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .items_center()
                    .px(metrics::SPACE_2)
                    .text_size(metrics::FONT_SIZE_BODY)
                    .on_click(cx.listener(move |_, _: &bool, _, cx| {
                        cx.emit(TagFilterEvent::Toggle(key.clone()));
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap(metrics::SPACE_2)
                            .justify_between()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.foreground)
                                    .child(option.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(metrics::FONT_SIZE_CAPTION)
                                    .text_color(theme.muted_foreground)
                                    .child(option.hosts.to_string()),
                            ),
                    ),
            )
    }
}

impl Render for TagFilterMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.visible_options(cx);
        let empty_text = if self.options.is_empty() {
            Some(strings::TAG_NONE)
        } else if visible.is_empty() {
            Some(strings::TAG_NO_MATCH)
        } else {
            None
        };
        let rows: Vec<_> = visible
            .iter()
            .map(|option| self.render_option(option, cx).into_any_element())
            .collect();
        let theme = cx.theme();
        let clear = ControlButton::new("tag-filter-clear", ControlKind::Ghost)
            .label(strings::TAG_CLEAR)
            .disabled(self.selected.is_empty())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(TagFilterEvent::Clear)));
        v_flex()
            .id("tag-filter-menu")
            .test_support()
            .key_context(KEY_CONTEXT)
            .w(px(MENU_WIDTH))
            .gap(metrics::SPACE_2)
            .text_color(theme.foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(self.render_search(window, cx))
            .child(
                v_flex()
                    .id("tag-filter-options")
                    .test_support()
                    .max_h(px(LIST_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .children(rows)
                    .when_some(empty_text, |this, text| {
                        this.child(
                            div()
                                .id("tag-filter-empty")
                                .test_support()
                                .px(metrics::SPACE_2)
                                .py(metrics::SPACE_2)
                                .text_color(theme.muted_foreground)
                                .child(text),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(1.))
                    .w_full()
                    .bg(theme::tokens(cx).border_strong.hsla()),
            )
            .child(h_flex().w_full().justify_end().child(clear))
    }
}

/// The chip of a tag that is on (design-system 3.4: the active filter chip
/// has the `--accent-subtle` fill and `--text-primary` text; the remove icon
/// is `--text-secondary`). A click turns the tag off.
pub fn selected_chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> impl StatefulInteractiveElement + IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let label: SharedString = label.into();
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .gap(metrics::SPACE_1)
        .h(px(CHIP_HEIGHT))
        .px(metrics::SPACE_3)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .bg(tokens.accent_subtle.hsla())
        .border_color(tokens.accent_subtle.hsla())
        .text_color(theme.foreground)
        .text_size(metrics::FONT_SIZE_CAPTION)
        .whitespace_nowrap()
        .cursor_default()
        .tooltip({
            let text: SharedString = strings::remove_tag_filter(&label).into();
            move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
            }
        })
        .child(label)
        .child(
            Icon::new(IconName::Close)
                .with_size(px(CHIP_ICON))
                .text_color(theme.muted_foreground),
        )
}

/// The "+N" chip for the tags that are on but did not fit; a click opens
/// the dropdown. It stands for selected tags, so it has the active filter
/// style of their chips (UI 2.2, design-system 3.4).
pub fn more_chip(hidden: usize, cx: &App) -> impl StatefulInteractiveElement + IntoElement {
    super::controls::tag_chip("tag-chip-more", strings::more_tags(hidden), true, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_collapse_into_more_when_they_do_not_fit() {
        let more = |hidden: usize| if hidden < 10 { 40. } else { 48. };
        let widths = [60., 70., 80.];
        // Everything fits: 8 + 60 + 4 + 70 + 4 + 80 = 226.
        assert_eq!(chips_room(&widths, 3, 8., 4., more), 226.);
        assert_eq!(chips_that_fit(&widths, 226., 8., 4., more), 3);
        // One short: two chips and "+1" (8 + 60 + 4 + 70 + 4 + 40 = 186).
        assert_eq!(chips_that_fit(&widths, 225., 8., 4., more), 2);
        assert_eq!(chips_room(&widths, 2, 8., 4., more), 186.);
        // Only "+3" (8 + 40).
        assert_eq!(chips_that_fit(&widths, 100., 8., 4., more), 0);
        assert_eq!(chips_room(&widths, 0, 8., 4., more), 48.);
        // Not even "+3" fits: it is still the one chip shown.
        assert_eq!(chips_that_fit(&widths, 10., 8., 4., more), 0);
        // No tag on: no chips, no room.
        assert_eq!(chips_room(&[], 0, 8., 4., more), 0.);
        assert_eq!(chips_that_fit(&[], 0., 8., 4., more), 0);
    }

    #[test]
    fn chip_widths_add_padding_border_and_icon() {
        assert_eq!(more_chip_width(20.), 20. + 24. + 2.);
        assert_eq!(selected_chip_width(20.), 20. + 24. + 2. + 4. + 12.);
        assert_eq!(tag_option_id("Prod").as_ref(), "tag-option-prod");
    }
}
