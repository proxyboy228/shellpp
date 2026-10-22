//! Sizes, spacing, radii, type sizes and motion of docs/design-system.md
//! (sections 5 and 6). Views use these names instead of bare numbers.
//!
//! Sizes are in rems (D96): 1 rem is 14 px at 100 % and follows the
//! interface zoom (View > Zoom In, UI 1.4), so the whole window scales; the
//! comments give the size at 100 %.

use std::rc::Rc;
use std::time::Duration;

use gpui::{App, FontWeight, Pixels, Rems, Size, Window, px};
use gpui_kit::component::Theme;

/// One rem at 100 % (the kit's base font size, `font.size` of the theme).
pub const REM_BASE: f32 = 14.;

/// A size of the design system given in px at 100 %, as rems.
pub const fn rpx(value: f32) -> Rems {
    Rems(value / REM_BASE)
}

/// `size` (rems) in pixels at 100 %: what tests and the screenshots of the
/// default zoom compare against. Rounded to 1/100 px: the sizes are whole
/// pixels at 100 %, and `n / 14 * 14` in `f32` is not always `n`.
pub fn at_100(size: Rems) -> Pixels {
    px(design_px(size))
}

/// `size` in px at 100 % (design units), for layout arithmetic that works
/// at every zoom alike (the host grid, the action bar).
pub fn design_px(size: Rems) -> f32 {
    (size.0 * REM_BASE * 100.).round() / 100.
}

/// A width or height in design units (px at 100 %) as rems: what a layout
/// planned in design units draws.
pub fn rems_of(design: Pixels) -> Rems {
    Rems(design.as_f32() / REM_BASE)
}

/// The interface zoom (D96): the kit's base font size, which every window
/// takes as its rem, over the one at 100 %.
pub fn zoom(cx: &App) -> f32 {
    Theme::global(cx).font_size.as_f32() / REM_BASE
}

/// The interface zoom of `window`: its rem over the rem at 100 % (the same
/// as [`zoom`] once the window has drawn a frame).
pub fn zoom_of(window: &Window) -> f32 {
    window.rem_size().as_f32() / REM_BASE
}

/// `size` in pixels at the zoom: for the few APIs that take pixels only
/// (the kit's button radius, shadows, virtual list rows).
pub fn to_px(size: Rems, cx: &App) -> Pixels {
    px(size.0 * REM_BASE * zoom(cx))
}

/// Design units (px at 100 %) in pixels at the zoom.
pub fn scaled(design: f32, cx: &App) -> Pixels {
    px(design * zoom(cx))
}

/// Pixels on screen (a pointer position, a measured width) in design units.
pub fn unscaled(on_screen: Pixels, cx: &App) -> Pixels {
    on_screen / zoom(cx)
}

/// The item sizes of a virtual list, planned in design units, in pixels at
/// the zoom (the list lays its items out in pixels).
pub fn scaled_sizes(sizes: &Rc<Vec<Size<Pixels>>>, cx: &App) -> Rc<Vec<Size<Pixels>>> {
    let zoom = zoom(cx);
    if zoom == 1. {
        return sizes.clone();
    }
    Rc::new(
        sizes
            .iter()
            .map(|item| gpui::size(item.width * zoom, item.height * zoom))
            .collect(),
    )
}

// Spacing: the 4 px grid (`--space-1` .. `--space-6`).
pub const SPACE_1: Rems = rpx(4.);
pub const SPACE_2: Rems = rpx(8.);
pub const SPACE_3: Rems = rpx(12.);
pub const SPACE_4: Rems = rpx(16.);
pub const SPACE_5: Rems = rpx(24.);
pub const SPACE_6: Rems = rpx(32.);

// Radii.
/// Buttons, fields, chips (`--radius-control`).
pub const RADIUS_CONTROL: Rems = rpx(6.);
/// Icon tiles (`--radius-tile`).
pub const RADIUS_TILE: Rems = rpx(8.);
/// Cards and panel blocks (`--radius-card`).
pub const RADIUS_CARD: Rems = rpx(10.);
/// Menus and modal windows (`--radius-overlay`).
pub const RADIUS_OVERLAY: Rems = rpx(12.);

// Heights.
/// The top bar (`--height-top-bar`).
pub const HEIGHT_TOP_BAR: Rems = rpx(44.);
/// Tabs of the top bar and its icon buttons (`--height-tab`).
pub const HEIGHT_TAB: Rems = rpx(32.);
/// Inside a tab, left and right (`--padding-tab`).
pub const PADDING_TAB: Rems = rpx(10.);
/// Sidebar items (`--height-nav-item`), this far apart (`--gap-nav-item`),
/// with this much between the icon and the name (`--gap-nav-icon`).
pub const HEIGHT_NAV_ITEM: Rems = rpx(32.);
pub const GAP_NAV_ITEM: Rems = rpx(2.);
pub const GAP_NAV_ICON: Rems = rpx(10.);
/// Buttons and fields (`--height-control`).
pub const HEIGHT_CONTROL: Rems = rpx(36.);
/// Table rows (`--height-row`).
pub const HEIGHT_ROW: Rems = rpx(44.);
/// Host cards (`--height-host-card`).
pub const HEIGHT_HOST_CARD: Rems = rpx(64.);

// Widths.
/// The sidebar (`--width-sidebar`).
pub const WIDTH_SIDEBAR: Rems = rpx(196.);
/// The details panel (`--width-details`), resizable within the two limits below.
pub const WIDTH_DETAILS: Rems = rpx(380.);
pub const WIDTH_DETAILS_MIN: Rems = rpx(320.);
pub const WIDTH_DETAILS_MAX: Rems = rpx(560.);

// Host card grid (owner's audit item 4): columns 240-360 px, 12 px apart.
pub const HOST_CARD_GAP: Rems = SPACE_3;
pub const HOST_CARD_MIN_WIDTH: Rems = rpx(240.);
pub const HOST_CARD_MAX_WIDTH: Rems = rpx(360.);
/// The icon tile on a host card.
pub const HOST_TILE_SIZE: Rems = rpx(36.);

// Indicators.
/// Status dot (`--size-status-dot`).
pub const SIZE_STATUS_DOT: Rems = rpx(8.);
/// Active session tab bar (`--size-tab-indicator`).
pub const SIZE_TAB_INDICATOR: Rems = rpx(2.);
/// Keyboard focus outline (`--focus-ring-width`, `--focus-ring-offset`).
pub const FOCUS_RING_WIDTH: Rems = rpx(2.);
pub const FOCUS_RING_OFFSET: Rems = rpx(2.);
/// Opacity of a disabled component (`--opacity-disabled`).
pub const OPACITY_DISABLED: f32 = 0.45;

// Typography (`--font-size-*`).
pub const FONT_SIZE_CAPTION: Rems = rpx(12.);
pub const FONT_SIZE_DENSE: Rems = rpx(13.);
pub const FONT_SIZE_BODY: Rems = rpx(14.);
pub const FONT_SIZE_HEADING: Rems = rpx(16.);
pub const FONT_SIZE_TITLE: Rems = rpx(20.);
pub const FONT_SIZE_MONO: Rems = rpx(14.);
pub const LINE_HEIGHT_MONO: f32 = 1.2;

// Motion (`--duration-fast`, `--duration-normal`).
pub const DURATION_FAST: Duration = Duration::from_millis(120);
pub const DURATION_NORMAL: Duration = Duration::from_millis(180);

/// How long a toast stays (UI 1.5).
pub const TOAST_DURATION: Duration = Duration::from_secs(5);
/// Debounce of panel edits before they are saved (docs/architecture.md, 2.5).
pub const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500);

// Font weights (`--font-weight-*`).
pub const FONT_WEIGHT_REGULAR: FontWeight = FontWeight::NORMAL;
pub const FONT_WEIGHT_MEDIUM: FontWeight = FontWeight::MEDIUM;
pub const FONT_WEIGHT_SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;

/// The interface font on Windows (`--font-ui`): Segoe UI Variable, which
/// ships with Windows 11. Where it is missing the system UI font stays.
pub const FONT_UI_WINDOWS: &[&str] = &["Segoe UI Variable Text", "Segoe UI Variable"];
