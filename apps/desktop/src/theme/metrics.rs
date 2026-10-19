//! Sizes, spacing, radii, type sizes and motion of docs/design-system.md
//! (sections 5 and 6). Views use these names instead of bare numbers.

use std::time::Duration;

use gpui::{FontWeight, Pixels, px};

// Spacing: the 4 px grid (`--space-1` .. `--space-6`).
pub const SPACE_1: Pixels = px(4.);
pub const SPACE_2: Pixels = px(8.);
pub const SPACE_3: Pixels = px(12.);
pub const SPACE_4: Pixels = px(16.);
pub const SPACE_5: Pixels = px(24.);
pub const SPACE_6: Pixels = px(32.);

// Radii.
/// Buttons, fields, chips (`--radius-control`).
pub const RADIUS_CONTROL: Pixels = px(6.);
/// Icon tiles (`--radius-tile`).
pub const RADIUS_TILE: Pixels = px(8.);
/// Cards and panel blocks (`--radius-card`).
pub const RADIUS_CARD: Pixels = px(10.);
/// Menus and modal windows (`--radius-overlay`).
pub const RADIUS_OVERLAY: Pixels = px(12.);

// Heights.
/// The top bar (`--height-top-bar`).
pub const HEIGHT_TOP_BAR: Pixels = px(44.);
/// Buttons and fields (`--height-control`).
pub const HEIGHT_CONTROL: Pixels = px(36.);
/// Table rows (`--height-row`).
pub const HEIGHT_ROW: Pixels = px(44.);
/// Host cards (`--height-host-card`).
pub const HEIGHT_HOST_CARD: Pixels = px(64.);

// Widths.
/// The sidebar (`--width-sidebar`).
pub const WIDTH_SIDEBAR: Pixels = px(220.);
/// The details panel (`--width-details`), resizable within the two limits below.
pub const WIDTH_DETAILS: Pixels = px(380.);
pub const WIDTH_DETAILS_MIN: Pixels = px(320.);
pub const WIDTH_DETAILS_MAX: Pixels = px(560.);

// Host card grid (owner's audit item 4): columns 240-360 px, 12 px apart.
pub const HOST_CARD_GAP: Pixels = SPACE_3;
pub const HOST_CARD_MIN_WIDTH: Pixels = px(240.);
pub const HOST_CARD_MAX_WIDTH: Pixels = px(360.);
/// The icon tile on a host card.
pub const HOST_TILE_SIZE: Pixels = px(36.);

// Indicators.
/// Status dot (`--size-status-dot`).
pub const SIZE_STATUS_DOT: Pixels = px(8.);
/// Active session tab bar (`--size-tab-indicator`).
pub const SIZE_TAB_INDICATOR: Pixels = px(2.);
/// Keyboard focus outline (`--focus-ring-width`, `--focus-ring-offset`).
pub const FOCUS_RING_WIDTH: Pixels = px(2.);
pub const FOCUS_RING_OFFSET: Pixels = px(2.);
/// Opacity of a disabled component (`--opacity-disabled`).
pub const OPACITY_DISABLED: f32 = 0.45;

// Typography (`--font-size-*`).
pub const FONT_SIZE_CAPTION: Pixels = px(12.);
pub const FONT_SIZE_DENSE: Pixels = px(13.);
pub const FONT_SIZE_BODY: Pixels = px(14.);
pub const FONT_SIZE_HEADING: Pixels = px(16.);
pub const FONT_SIZE_TITLE: Pixels = px(20.);
pub const FONT_SIZE_MONO: Pixels = px(14.);
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
