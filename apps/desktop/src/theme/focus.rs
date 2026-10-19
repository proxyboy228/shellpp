//! The keyboard focus ring of docs/design-system.md, section 6: one 2 px
//! `--focus-ring` outline with a 2 px offset, shown only for keyboard focus.
//!
//! gpui-kit's own ring is different (a tinted border plus a translucent 3 px
//! band), so our elements turn it off (`FocusableExt::focus_ring(false)` on
//! kit buttons) and use [`FocusRingExt::focus_ring_visible`] instead. The
//! ring is drawn as two box shadows: the outline (offset + width) and, on
//! top of it, the offset band in the color of the surface around the element.

use gpui::{App, BoxShadow, Hsla, InteractiveElement, Styled as _, px};
use gpui_kit::component::ActiveTheme as _;

use super::metrics::{FOCUS_RING_OFFSET, FOCUS_RING_WIDTH};

/// The shadows that draw the focus ring around an element standing on
/// `surface` (the color right around the element).
pub fn ring_shadows(surface: Hsla, cx: &App) -> Vec<BoxShadow> {
    let ring = cx.theme().ring;
    vec![
        BoxShadow::new(px(0.), px(0.), ring)
            .blur_radius(px(0.))
            .spread_radius(FOCUS_RING_OFFSET + FOCUS_RING_WIDTH),
        BoxShadow::new(px(0.), px(0.), surface)
            .blur_radius(px(0.))
            .spread_radius(FOCUS_RING_OFFSET),
    ]
}

/// Adds the design-system focus ring to a focusable element.
pub trait FocusRingExt: InteractiveElement + Sized {
    /// Shows the ring while the element has keyboard focus
    /// (`focus_visible`). `surface` is the color around the element.
    fn focus_ring_visible(self, surface: Hsla, cx: &App) -> Self {
        let shadows = ring_shadows(surface, cx);
        self.focus_visible(move |style| style.shadow(shadows))
    }
}

impl<T: InteractiveElement + Sized> FocusRingExt for T {}
