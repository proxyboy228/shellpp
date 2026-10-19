//! Motion of docs/design-system.md, section 6: 120 ms (`--duration-fast`:
//! hover, press, toggles) and 180 ms (`--duration-normal`: menus, panels,
//! modal windows), ease-out. Under the system's reduced-motion preference
//! (Windows: "Animation effects" off; read by gpui-kit at `init`) every
//! duration is 0 ms, so nothing waits for an animation to end.

use std::time::Duration;

use gpui::App;

use super::metrics::{DURATION_FAST, DURATION_NORMAL};

/// `--duration-fast`, or 0 ms under reduced motion.
pub fn fast(cx: &App) -> Duration {
    reduced(DURATION_FAST, cx)
}

/// `--duration-normal`, or 0 ms under reduced motion.
pub fn normal(cx: &App) -> Duration {
    reduced(DURATION_NORMAL, cx)
}

fn reduced(duration: Duration, cx: &App) -> Duration {
    if cx.reduce_motion() {
        Duration::ZERO
    } else {
        duration
    }
}

/// `--easing-standard`: ease-out (CSS `cubic-bezier(0, 0, 0.58, 1)`
/// approximated by a quadratic ease-out).
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    1. - (1. - t) * (1. - t)
}

/// Sets gpui-kit's motion tokens (used by kit menus, popovers, switches) to
/// the design-system durations.
pub(super) fn install(cx: &mut App) {
    gpui_kit::component::Theme::update(cx, |theme| {
        theme.motion.duration_fast = DURATION_FAST;
        theme.motion.duration_normal = DURATION_NORMAL;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_out_starts_fast_and_ends_at_one() {
        assert_eq!(ease_out(0.), 0.);
        assert_eq!(ease_out(1.), 1.);
        assert!(ease_out(0.5) > 0.5);
    }

    #[gpui_kit::test]
    fn reduced_motion_makes_durations_zero(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(fast(cx), DURATION_FAST);
            assert_eq!(normal(cx), DURATION_NORMAL);
            cx.set_reduce_motion(true);
            assert_eq!(fast(cx), Duration::ZERO);
            assert_eq!(normal(cx), Duration::ZERO);
        });
    }
}
