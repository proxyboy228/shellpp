//! The interface zoom (UI 1.4 View > Zoom In, Zoom Out, Actual Size; D96).
//!
//! The design system is in rems (`metrics`), and every window takes the
//! kit's base font size (`font.size` of the theme) as its rem; the zoom sets
//! that size, so every window scales at once and the next launch starts at
//! the zoom of `appearance.ui_zoom`. The terminal's font is a setting of its
//! own (`terminal.font_size`) and keeps its size. On top of it the system
//! scales the window by the display's scale factor (150 % on Windows and
//! 125 % here draw at 187.5 %).

use std::rc::Rc;

use gpui::{App, px};
use gpui_kit::component::Theme;

use super::metrics;

/// The setting that keeps the zoom (data-model 16.1).
pub const SETTING: &str = "appearance.ui_zoom";

/// The steps of Zoom In and Zoom Out, smallest first; 1.0 is Actual Size.
pub const STEPS: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// The zoom in effect.
pub fn current(cx: &App) -> f32 {
    metrics::zoom(cx)
}

/// The step nearest to `zoom` (a stored value between steps, or outside
/// them, snaps to the nearest).
pub fn nearest(zoom: f32) -> f32 {
    STEPS
        .iter()
        .copied()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.)
}

/// The step after `zoom` (Zoom In), the one before (Zoom Out); the ends
/// stay where they are.
pub fn step(zoom: f32, up: bool) -> f32 {
    let index = STEPS
        .iter()
        .position(|step| *step == nearest(zoom))
        .unwrap_or(2);
    if up {
        STEPS[(index + 1).min(STEPS.len() - 1)]
    } else {
        STEPS[index.saturating_sub(1)]
    }
}

/// The kit's radii at 100 % (`radius` and `radius.lg` of the theme file):
/// `--radius-control` and `--radius-overlay`.
const KIT_RADIUS: f32 = 6.;
const KIT_RADIUS_LG: f32 = 12.;

/// Applies `zoom` (snapped to a step) to every window: the kit's base font
/// size of both themes, and its radii (they are pixels in the kit), so a
/// later change of appearance keeps them.
pub fn set(zoom: f32, cx: &mut App) {
    let zoom = nearest(zoom);
    let size = metrics::REM_BASE * zoom;
    let radius = (KIT_RADIUS * zoom).round();
    let radius_lg = (KIT_RADIUS_LG * zoom).round();
    let theme = Theme::global_mut(cx);
    for config in [&mut theme.dark_theme, &mut theme.light_theme] {
        let mut zoomed = (**config).clone();
        zoomed.font_size = Some(size);
        zoomed.mono_font_size = Some(size);
        zoomed.radius = Some(radius as usize);
        zoomed.radius_lg = Some(radius_lg as usize);
        *config = Rc::new(zoomed);
    }
    theme.font_size = px(size);
    theme.mono_font_size = px(size);
    theme.radius = px(radius);
    theme.radius_lg = px(radius_lg);
    cx.refresh_windows();
}

/// The zoom of the settings: the last launch's, applied before the first
/// window opens, and again when the setting changes (an import, a reset).
pub fn apply_stored(core: &shellpp_core::Core, cx: &mut App) {
    let stored = core
        .settings_get()
        .ok()
        .and_then(|values| values.get(SETTING).and_then(serde_json::Value::as_f64))
        .unwrap_or(1.);
    if nearest(stored as f32) != current(cx) {
        set(stored as f32, cx);
    }
}

/// Changes the zoom and keeps it in the settings; `None` is Actual Size.
pub fn change(core: Option<&shellpp_core::Core>, up: Option<bool>, cx: &mut App) {
    let next = match up {
        Some(up) => step(current(cx), up),
        None => 1.,
    };
    set(next, cx);
    if let Some(core) = core
        && let Err(error) = core.settings_set(shellpp_core::service::dto::SettingsSetInput {
            // Two decimals: 1.1, not the f32's 1.100000023841858.
            values: [(
                SETTING.to_owned(),
                serde_json::json!((f64::from(next) * 100.).round() / 100.),
            )]
            .into_iter()
            .collect(),
            reset: Vec::new(),
        })
    {
        tracing::warn!(code = error.code(), "the interface zoom was not kept");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_up_and_down_and_stop_at_the_ends() {
        assert_eq!(step(1., true), 1.1);
        assert_eq!(step(1.1, true), 1.25);
        assert_eq!(step(1.25, false), 1.1);
        assert_eq!(step(2., true), 2.);
        assert_eq!(step(0.8, false), 0.8);
        assert_eq!(step(0.8, true), 0.9);
        // A stored value between steps snaps to the nearest first.
        assert_eq!(nearest(1.3), 1.25);
        assert_eq!(nearest(0.5), 0.8);
        assert_eq!(nearest(9.), 2.);
        assert_eq!(step(1.3, true), 1.5);
    }
}
