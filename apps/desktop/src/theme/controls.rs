//! Button colors of docs/design-system.md (sections 2 and 6) for gpui-kit
//! buttons, so views never pick colors for kit buttons themselves. Use the
//! [`ButtonColors`] methods on a `Button`:
//!
//! - [`ButtonColors::accent_colors`]: `--accent` / `--accent-hover` /
//!   `--accent-pressed`, text `--text-on-accent`: the one primary action on
//!   screen (the kit's Primary variant, mapped to our accent in 7.4);
//! - [`ButtonColors::neutral_colors`]: `--bg-raised`, hover
//!   `--bg-raised-hover`, pressed `--bg-pressed`, text `--text-primary`
//!   (New host, Create host, ...; the kit's Secondary variant);
//! - [`ButtonColors::ghost_colors`]: no fill, icon/text `--text-secondary`,
//!   hover `--bg-raised-hover`, pressed `--bg-pressed` (icon buttons of bars).
//!
//! Trap (gpui-kit 0.7.3): a `ButtonCustomVariant` paints its normal
//! background at 20 % (`color.mix_oklab(transparent, 0.2)`); only hover and
//! pressed use the exact colors. A custom variant is therefore right only
//! for a transparent button ([`ghost`]); filled buttons use the kit's
//! Primary / Secondary variants, whose colors come from `graphite.json`.
//!
//! Disabled buttons are the whole component at `--opacity-disabled` without
//! hover (section 6): [`ButtonColors::disabled_as_designed`].
//!
//! Trap (gpui-kit 0.7.3): the kit's own disabled style repaints a filled
//! button (Primary and Danger fill at 15 %, text `muted_foreground` at 50 %),
//! so `.disabled(true)` plus `.opacity(0.45)` faded it twice (an accent fill
//! at about 7 %). The helper keeps the button inert through the kit but
//! gives the disabled state the variant's normal colors: the kit replays
//! the caller's own style inside its disabled state, so explicit colors win.

use gpui::{App, Entity, Hsla, IntoElement, Styled, canvas};
use gpui_kit::base::input::{DiagnosticColors, InputBaseState, InputEditorStyle, InputModeKind};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants};
use gpui_kit::component::{ActiveTheme as _, Disableable};

use super::metrics;

/// The filled buttons of design-system 6 (the kit variant a view picked).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filled {
    /// [`ButtonColors::accent_colors`]: `--accent`, text `--text-on-accent`.
    Accent,
    /// [`ButtonColors::neutral_colors`]: `--bg-raised`, text `--text-primary`.
    Neutral,
    /// The kit's Danger variant: `--danger`, text `--text-on-accent`.
    Danger,
}

impl Filled {
    /// The fill and text colors of the variant's normal state (the same
    /// theme fields the kit paints it with).
    pub fn colors(self, cx: &App) -> (Hsla, Hsla) {
        let theme = cx.theme();
        match self {
            Self::Accent => (theme.button_primary, theme.button_primary_foreground),
            Self::Neutral => (theme.button_secondary, theme.button_secondary_foreground),
            Self::Danger => (theme.button_danger, theme.button_danger_foreground),
        }
    }
}

/// Design-system colors for a kit button.
pub trait ButtonColors: ButtonVariants + Sized {
    /// The accent (primary) button.
    fn accent_colors(self) -> Self {
        self.with_variant(ButtonVariant::Primary)
    }

    /// The neutral button.
    fn neutral_colors(self) -> Self {
        self.with_variant(ButtonVariant::Secondary)
    }

    /// The ghost (icon) button of bars.
    fn ghost_colors(self, cx: &App) -> Self {
        self.custom(ghost(cx))
    }

    /// Disabled per design-system 6 when `disabled`: no clicks, no hover or
    /// pressed state, the variant's normal fill and text (`kind`), and the
    /// whole component once at `--opacity-disabled`. Nothing changes while
    /// enabled.
    fn disabled_as_designed(self, disabled: bool, kind: Filled, cx: &App) -> Self
    where
        Self: Disableable + Styled,
    {
        if !disabled {
            return self;
        }
        let (fill, text) = kind.colors(cx);
        self.disabled(true)
            .bg(fill)
            .border_color(fill)
            .text_color(text)
            .opacity(metrics::OPACITY_DISABLED)
    }
}

impl ButtonColors for Button {}

/// The ghost (icon) button of bars as a custom variant (transparent, so the
/// kit's 20 % normal fill does not matter).
pub fn ghost(cx: &App) -> ButtonCustomVariant {
    let theme = cx.theme();
    ButtonCustomVariant::new(cx)
        .color(theme.transparent)
        .foreground(theme.muted_foreground)
        .hover(theme.secondary_hover)
        .active(theme.secondary_active)
}

/// Paints the placeholder of a kit input (`Input`, `Textarea`) in
/// `--text-muted` (design-system 2.3; 7.4 maps the kit's `muted_foreground`
/// to `--text-secondary`).
///
/// Trap (gpui-kit 0.7.3): the kit `Input` projects `muted_foreground` into
/// the state's editor style on every render, and the text element paints
/// the placeholder with it during prepaint. This zero-size element, placed
/// in the same parent BEFORE the input, prepaints first and swaps in the same
/// style with `--text-muted` (layout runs for every child before any
/// prepaint, so the kit's projection is already done by then).
pub fn muted_placeholder<M: InputModeKind>(
    state: &Entity<InputBaseState<M>>,
    cx: &App,
) -> impl IntoElement + use<M> {
    let theme = cx.theme();
    let tokens = super::tokens(cx);
    let highlight = theme.highlight_theme.clone();
    let style = InputEditorStyle {
        foreground: theme.foreground,
        muted_foreground: tokens.text_muted.hsla(),
        background: tokens.bg_input.hsla(),
        border: theme.border,
        selection: theme.selection,
        caret: theme.caret,
        diagnostics: DiagnosticColors {
            error: theme.danger,
            warning: theme.warning,
            info: theme.info,
            hint: theme.muted_foreground,
        },
        editor_invisible: highlight.style.editor_invisible,
        editor_active_line: highlight.style.editor_active_line,
        editor_gutter_background: highlight.style.editor_gutter_background,
        highlight_styles: highlight,
        fold_icon_renderer: None,
    };
    let state = state.clone();
    canvas(
        move |_, _, cx| state.update(cx, |state, _| state.set_editor_style(style)),
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
}

/// Do not use for a filled button: the kit paints a custom variant's fill
/// at 20 %. Use [`ButtonColors::neutral_colors`].
#[deprecated(
    note = "the kit paints a custom variant's fill at 20 %; use `ButtonColors::neutral_colors`"
)]
pub fn neutral(cx: &App) -> ButtonCustomVariant {
    let theme = cx.theme();
    ButtonCustomVariant::new(cx)
        .color(theme.secondary)
        .foreground(theme.secondary_foreground)
        .hover(theme.secondary_hover)
        .active(theme.secondary_active)
}

/// Do not use for a filled button: the kit paints a custom variant's fill
/// at 20 %. Use [`ButtonColors::accent_colors`].
#[deprecated(
    note = "the kit paints a custom variant's fill at 20 %; use `ButtonColors::accent_colors`"
)]
pub fn accent(cx: &App) -> ButtonCustomVariant {
    let theme = cx.theme();
    ButtonCustomVariant::new(cx)
        .color(theme.primary)
        .foreground(theme.primary_foreground)
        .hover(theme.primary_hover)
        .active(theme.primary_active)
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Render, TestAppContext,
        Window, WindowBounds, WindowOptions, div, px, size,
    };
    use gpui_kit::test::TestWindowExt as _;

    use super::*;
    use crate::theme::{Appearance, install};

    struct Buttons;

    impl Render for Buttons {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // Away from the pointer at the window origin (no hover state).
            div()
                .size_full()
                .flex()
                .flex_wrap()
                .p(px(40.))
                .gap(px(8.))
                .child(
                    Button::new("accent")
                        .label("Accent")
                        .accent_colors()
                        .w(px(120.)),
                )
                .child(
                    Button::new("neutral")
                        .label("Neutral")
                        .neutral_colors()
                        .w(px(120.)),
                )
                .child(
                    Button::new("accent-disabled")
                        .label("Accent")
                        .accent_colors()
                        .w(px(120.))
                        .disabled_as_designed(true, Filled::Accent, cx),
                )
                .child(
                    Button::new("neutral-disabled")
                        .label("Neutral")
                        .neutral_colors()
                        .w(px(120.))
                        .disabled_as_designed(true, Filled::Neutral, cx),
                )
        }
    }

    /// The fills are the exact tokens (the custom-variant trap above), and a
    /// disabled button is its normal fill at `--opacity-disabled` (0.45),
    /// not the kit's own faded disabled style under that opacity.
    #[gpui_kit::test]
    fn filled_buttons_paint_the_exact_tokens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            install(Appearance::Dark, cx);
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(400.), px(200.)),
            ))),
            ..Default::default()
        };
        let window = cx
            .update(|cx| gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Buttons)))
            .unwrap()
            .0;
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let theme = cx.theme();
            let quads = window.painted_quads();
            let faded = metrics::OPACITY_DISABLED;
            for (id, color, alpha) in [
                ("accent", theme.primary, 1.),
                ("neutral", theme.secondary, 1.),
                ("accent-disabled", theme.primary, faded),
                ("neutral-disabled", theme.secondary, faded),
            ] {
                let bounds = window.find(id).bounds();
                let scale = window.scale_factor();
                let found = quads.iter().any(|quad| {
                    let inside = (quad.bounds.origin.x.0 / scale - bounds.origin.x.as_f32()).abs()
                        < 1.
                        && (quad.bounds.origin.y.0 / scale - bounds.origin.y.as_f32()).abs() < 1.
                        && (quad.bounds.size.width.0 / scale - bounds.size.width.as_f32()).abs()
                            < 1.;
                    inside
                        && quad.background.as_solid().is_some_and(|fill| {
                            let (a, b) = (fill.to_rgb(), color.to_rgb());
                            (a.r - b.r).abs() < 1e-3
                                && (a.g - b.g).abs() < 1e-3
                                && (a.b - b.b).abs() < 1e-3
                                && (a.a - alpha).abs() < 1e-3
                        })
                });
                assert!(found, "{id}: fill of its token at alpha {alpha}");
            }
        })
        .unwrap();
    }
}
