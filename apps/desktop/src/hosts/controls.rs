//! Controls of the Hosts screen drawn exactly per docs/design-system.md,
//! section 6 (36 px, radius 6, hover / pressed surfaces, disabled = the whole
//! control at opacity 0.45 without hover, one 2 px `--focus-ring` with a
//! 2 px offset for keyboard focus). gpui-kit's own button derives its
//! colors and sizes from its theme defaults, so these wrappers paint from
//! the tokens instead (design-system 7.4, last rule).

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement,
    Interactivity, IntoElement, MouseButton, ParentElement, Rems, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, TextRun, Window, div,
    prelude::FluentBuilder,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{ActiveTheme as _, Disableable, Icon, Selectable};

use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// Icon size inside controls.
pub const ICON_SIZE: Rems = metrics::rpx(16.);
/// What a 36 px control button adds around its label: the 12 px padding on
/// both sides, the 16 px icon and the 8 px gap.
pub const LABEL_BUTTON_EXTRA: f32 = 48.;

/// The width of `text` in the window's current font with 2 % and a pixel to
/// spare (it plans layouts, so it errs on the wide side), in px at 100 %
/// (design units, D96): the layouts plan at every zoom alike.
pub fn text_width(window: &Window, text: &str, size: Rems, weight: FontWeight) -> f32 {
    if text.is_empty() {
        return 0.;
    }
    let mut font = window.text_style().font();
    font.weight = weight;
    let run = TextRun {
        len: text.len(),
        font,
        ..Default::default()
    };
    let line = window.text_system().shape_line(
        SharedString::from(text.to_owned()),
        size.to_pixels(window.rem_size()),
        &[run],
        None,
    );
    (line.width().as_f32() / metrics::zoom_of(window) * 1.02).ceil() + 1.
}

/// The common width of buttons of one group that stand side by side, each
/// with an icon and one of `labels` (design-system 6 [decision]): as wide
/// as the widest of them. Give it to each button as its minimum width.
pub fn group_width(window: &Window, labels: &[&str]) -> Rems {
    let widest = labels
        .iter()
        .map(|label| text_width(window, label, metrics::FONT_SIZE_BODY, FontWeight::MEDIUM))
        .fold(0., f32::max);
    metrics::rpx(widest + LABEL_BUTTON_EXTRA)
}

/// How a control is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    /// `--bg-raised`, hover `--bg-raised-hover`, pressed `--bg-pressed`,
    /// text `--text-primary` (New host, Terminal, Serial; Create host while
    /// a details panel is open).
    Neutral,
    /// The one primary action of the screen: `--accent`, hover
    /// `--accent-hover`, pressed `--accent-pressed`, text `--text-on-accent`.
    Accent,
    /// No fill until hovered; `--text-secondary` icon (view toggle, sort).
    Ghost,
    /// A confirmed dangerous action: `--danger`, hover `--danger-hover`,
    /// pressed `--danger-pressed`, text `--text-on-danger`.
    Danger,
}

/// The colors of one control state set.
struct Paint {
    bg: Hsla,
    hover: Hsla,
    pressed: Hsla,
    fg: Hsla,
}

fn paint(kind: ControlKind, active: bool, cx: &App) -> Paint {
    let theme = cx.theme();
    match kind {
        ControlKind::Neutral => Paint {
            bg: if active {
                theme.secondary_active
            } else {
                theme.secondary
            },
            hover: theme.secondary_hover,
            pressed: theme.secondary_active,
            fg: theme.foreground,
        },
        ControlKind::Accent => Paint {
            bg: theme.primary,
            hover: theme.primary_hover,
            pressed: theme.primary_active,
            fg: theme.primary_foreground,
        },
        ControlKind::Danger => Paint {
            bg: theme.danger,
            hover: theme.danger_hover,
            pressed: theme.danger_active,
            fg: theme.danger_foreground,
        },
        ControlKind::Ghost => Paint {
            bg: if active {
                theme::tokens(cx).accent_subtle.hsla()
            } else {
                theme.transparent
            },
            hover: if active {
                theme::tokens(cx).accent_subtle.hsla()
            } else {
                theme.secondary_hover
            },
            pressed: theme.secondary_active,
            fg: if active {
                theme.foreground
            } else {
                theme.muted_foreground
            },
        },
    }
}

/// The keyboard focus ring of design-system 6: one 2 px `--focus-ring`
/// line, 2 px outside the element's border box. `radius` is the element's
/// own corner radius; `border` its border width.
pub fn focus_ring(radius: Rems, border: Rems, cx: &App) -> impl IntoElement {
    let outset = metrics::FOCUS_RING_OFFSET + metrics::FOCUS_RING_WIDTH + border;
    div()
        .absolute()
        .top(-outset)
        .left(-outset)
        .right(-outset)
        .bottom(-outset)
        .border(metrics::FOCUS_RING_WIDTH)
        .border_color(cx.theme().ring)
        .rounded(radius + metrics::FOCUS_RING_OFFSET + metrics::FOCUS_RING_WIDTH)
}

/// A 36 px button with a label and/or an icon.
#[derive(IntoElement)]
pub struct ControlButton {
    id: ElementId,
    base: Stateful<Div>,
    kind: ControlKind,
    label: Option<SharedString>,
    icon: Option<Icon>,
    trailing_icon: Option<Icon>,
    tooltip: Option<SharedString>,
    /// The tooltip is a file path, shown in the monospaced font.
    tooltip_is_path: bool,
    disabled: bool,
    selected: bool,
    height: Rems,
    rounded: (bool, bool),
    on_click: Option<ClickHandler>,
    /// A focus handle the owner keeps (a dialog's default button).
    focus_handle: Option<FocusHandle>,
}

/// A click handler of a control.
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

impl std::fmt::Debug for ControlButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlButton")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl ControlButton {
    pub fn new(id: impl Into<ElementId>, kind: ControlKind) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            kind,
            label: None,
            icon: None,
            trailing_icon: None,
            tooltip: None,
            tooltip_is_path: false,
            disabled: false,
            selected: false,
            height: metrics::HEIGHT_CONTROL,
            rounded: (true, true),
            on_click: None,
            focus_handle: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// An icon after the label (a dropdown caret).
    pub fn trailing_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.trailing_icon = Some(icon.into());
        self
    }

    /// Accessible name of an icon-only button.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// A file path as the tooltip, in the monospaced font: the label says
    /// what the button does in plain words, the tooltip names the file
    /// (design-system 6 [decision]).
    pub fn path_tooltip(mut self, path: impl Into<SharedString>) -> Self {
        self.tooltip = Some(path.into());
        self.tooltip_is_path = true;
        self
    }

    /// A control height other than 36 px (the Connect button inside the
    /// search field).
    pub fn height(mut self, height: Rems) -> Self {
        self.height = height;
        self
    }

    /// Which outer corners are rounded (`left`, `right`): the halves of a
    /// split button keep their inner corners square.
    pub fn rounded_sides(mut self, left: bool, right: bool) -> Self {
        self.rounded = (left, right);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// The focus handle of the owner, which can give this button the
    /// focus (the default choice of a dialog); otherwise the button keeps
    /// one of its own.
    pub fn focus_handle(mut self, handle: FocusHandle) -> Self {
        self.focus_handle = Some(handle);
        self
    }
}

impl Disableable for ControlButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Selectable for ControlButton {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Styled for ControlButton {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for ControlButton {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl DropdownMenu for ControlButton {}

impl RenderOnce for ControlButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = paint(self.kind, self.selected, cx);
        let radius = metrics::RADIUS_CONTROL;
        let (round_left, round_right) = self.rounded;
        let icon_only = self.label.is_none();
        let focus_handle = self.focus_handle.clone().unwrap_or_else(|| {
            window
                .use_keyed_state(self.id.clone(), cx, |_, cx| {
                    cx.focus_handle().tab_stop(true)
                })
                .read(cx)
                .clone()
        });
        let focused = !self.disabled && focus_handle.is_focused(window);
        let fg = colors.fg;
        let label_color = if self.kind == ControlKind::Ghost && self.label.is_some() {
            cx.theme().foreground
        } else {
            fg
        };
        let disabled = self.disabled;
        let on_click = self.on_click;
        let hover_bg = colors.hover;
        let pressed_bg = colors.pressed;

        self.base
            .test_support()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_2)
            .h(self.height)
            .when(icon_only, |this| this.w(self.height))
            .when(!icon_only, |this| this.px(metrics::SPACE_3))
            .when(round_left, |this| this.rounded_l(radius))
            .when(round_right, |this| this.rounded_r(radius))
            .bg(colors.bg)
            .text_color(label_color)
            .text_size(metrics::FONT_SIZE_BODY)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .cursor_default()
            .when(disabled, |this| this.opacity(metrics::OPACITY_DISABLED))
            .when(!disabled, |this| {
                this.track_focus(&focus_handle)
                    .hover(move |style| style.bg(hover_bg))
                    .active(move |style| style.bg(pressed_bg))
                    // Focus comes from the keyboard only; a click does not
                    // leave a ring behind.
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .when_some(on_click, |this, on_click| {
                        this.on_click(move |event, window, cx| on_click(event, window, cx))
                    })
            })
            .when_some(self.tooltip, |this, tooltip| {
                let is_path = self.tooltip_is_path;
                this.tooltip(move |window, cx| {
                    use gpui_kit::component::tooltip::Tooltip;
                    if is_path {
                        let path = tooltip.clone();
                        Tooltip::element(move |_, cx| {
                            div()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(path.clone())
                        })
                        .build(window, cx)
                    } else {
                        Tooltip::new(tooltip.clone()).build(window, cx)
                    }
                })
            })
            .when_some(self.icon, |this, icon| {
                this.child(icon.size(ICON_SIZE).text_color(fg))
            })
            .when_some(self.label, |this, label| this.child(label))
            .when_some(self.trailing_icon, |this, icon| {
                this.child(icon.size(ICON_SIZE).text_color(fg))
            })
            .when(focused, |this| {
                this.child(focus_ring(radius, Rems::ZERO, cx))
            })
    }
}

/// A tag filter chip (design-system 3.4): no fill, a 1 px `--border-strong`
/// border and `--text-secondary` text; the active chip gets the
/// `--accent-subtle` fill and `--text-primary` text.
pub fn tag_chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> impl StatefulInteractiveElement + IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    div()
        .id(id)
        .test_support()
        .keyboard_stop(theme.colors.list, cx)
        .flex()
        .flex_none()
        .items_center()
        .h(metrics::rpx(28.))
        .px(metrics::SPACE_3)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .whitespace_nowrap()
        .cursor_default()
        .map(|this| {
            if active {
                this.bg(tokens.accent_subtle.hsla())
                    .border_color(tokens.accent_subtle.hsla())
                    .text_color(theme.foreground)
            } else {
                this.border_color(tokens.border_strong.hsla())
                    .text_color(theme.muted_foreground)
                    .hover(move |style| style.bg(hover))
            }
        })
        .child(label.into())
}
