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
//! hover (section 6): set `.disabled(true)` and
//! `.opacity(metrics::OPACITY_DISABLED)` on the button.

use gpui::{App, Entity, IntoElement, Styled as _, canvas};
use gpui_kit::base::input::{DiagnosticColors, InputBaseState, InputEditorStyle, InputModeKind};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants};

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
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Away from the pointer at the window origin (no hover state).
            div()
                .size_full()
                .flex()
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
        }
    }

    /// The fills are the exact tokens (the custom-variant trap above).
    #[gpui_kit::test]
    fn filled_buttons_paint_the_exact_tokens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            install(Appearance::Dark, cx);
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(400.), px(100.)),
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
            for (id, color) in [("accent", theme.primary), ("neutral", theme.secondary)] {
                let bounds = window.find(id).bounds();
                let scale = window.scale_factor();
                let found = quads.iter().any(|quad| {
                    let inside = (quad.bounds.origin.x.0 / scale - bounds.origin.x.as_f32()).abs()
                        < 1.
                        && (quad.bounds.size.width.0 / scale - bounds.size.width.as_f32()).abs()
                            < 1.;
                    inside
                        && quad.background.as_solid().is_some_and(|fill| {
                            let (a, b) = (fill.to_rgb(), color.to_rgb());
                            (a.r - b.r).abs() < 1e-3
                                && (a.g - b.g).abs() < 1e-3
                                && (a.b - b.b).abs() < 1e-3
                                && (a.a - 1.).abs() < 1e-3
                        })
                });
                assert!(found, "{id}: opaque fill of its token");
            }
        })
        .unwrap();
    }
}
