//! Minimum contrast of terminal text (UI 12.4 "Adjust indistinguishable
//! colors", owner's stage 2b step 0.4): when the WCAG 2.x contrast of a
//! cell's text and background is below 4.5:1, the text color moves toward
//! white or black until the pair reaches 4.5:1.
//!
//! - The direction: away from the background first (lighter text toward
//!   white, darker text toward black); when that end cannot reach 4.5:1
//!   (white on a light gray), the other one. One of the two always can:
//!   the better of black and white has at least 4.58:1 on any color.
//! - The amount: the smallest step along the straight line from the text
//!   color to that end (in sRGB, channels rounded) that reaches 4.5:1, so
//!   the hue stays as close to the original as the rule allows.
//! - What: [`MinContrast::Palette`] (the default) adjusts text whose color
//!   comes from the theme's palette (the 16 ANSI colors, the 256-color
//!   table, the default foreground and background, also when a program
//!   redefined them with OSC 4/10/11) and never truecolor (`SGR 38;2`);
//!   [`MinContrast::Always`] adjusts truecolor text too;
//!   [`MinContrast::Off`] adjusts nothing.
//! - Never: hidden text (SGR 8; it must stay invisible), the selection
//!   foreground of a theme and the text under the block cursor (they keep
//!   their own colors).
//!
//! Ratios are compared with 4.5 without rounding (design-system 1). Results
//! are cached per (text, background) pair ([`ContrastCache`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use alacritty_terminal::vte::ansi::Rgb;

/// The contrast every adjusted pair reaches (WCAG AA for text).
pub const MIN_RATIO: f64 = 4.5;

/// `terminal.min_contrast` (docs/data-model.md, section 16.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MinContrast {
    /// Colors are drawn as the program and the theme set them.
    Off,
    /// Text in palette colors is adjusted; truecolor text is not (default).
    #[default]
    Palette,
    /// All text is adjusted, truecolor included.
    Always,
}

impl MinContrast {
    /// The choices in menu order.
    pub const ALL: [MinContrast; 3] = [MinContrast::Off, MinContrast::Palette, MinContrast::Always];

    /// The setting value.
    pub fn key(self) -> &'static str {
        match self {
            MinContrast::Off => "off",
            MinContrast::Palette => "palette",
            MinContrast::Always => "always",
        }
    }

    /// The choice of a setting value; `None` for anything else.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }

    /// Whether text of this kind is adjusted: `truecolor` is text whose
    /// color the program sent as `SGR 38;2` (or `48;2` for inverse text).
    pub fn applies(self, truecolor: bool) -> bool {
        match self {
            MinContrast::Off => false,
            MinContrast::Palette => !truecolor,
            MinContrast::Always => true,
        }
    }
}

const WHITE: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};
const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

/// The sRGB transfer function of the 256 channel values.
fn linear_table() -> &'static [f64; 256] {
    static TABLE: OnceLock<[f64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.; 256];
        for (value, linear) in table.iter_mut().enumerate() {
            let value = value as f64 / 255.;
            *linear = if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            };
        }
        table
    })
}

/// The WCAG 2.x relative luminance.
pub fn luminance(color: Rgb) -> f64 {
    let table = linear_table();
    0.2126 * table[usize::from(color.r)]
        + 0.7152 * table[usize::from(color.g)]
        + 0.0722 * table[usize::from(color.b)]
}

/// The WCAG 2.x contrast ratio of two opaque colors (1 to 21).
pub fn ratio(a: Rgb, b: Rgb) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// `from` moved by `t` (0 to 1) toward `to`, channels rounded.
fn mix(from: Rgb, to: Rgb, t: f64) -> Rgb {
    let channel = |from: u8, to: u8| {
        let from = f64::from(from);
        (from + (f64::from(to) - from) * t).round().clamp(0., 255.) as u8
    };
    Rgb {
        r: channel(from.r, to.r),
        g: channel(from.g, to.g),
        b: channel(from.b, to.b),
    }
}

/// `color` is on the far side of `background` toward `end` and has enough
/// contrast with it. Along a line toward `end` this holds from some point
/// on, so it can be bisected.
fn reaches(color: Rgb, background: Rgb, toward_white: bool) -> bool {
    let (text, back) = (luminance(color), luminance(background));
    let far_side = if toward_white {
        text > back
    } else {
        text < back
    };
    far_side && ratio(color, background) >= MIN_RATIO
}

/// The text color to draw `text` on `background` with: `text` itself when
/// the pair has at least 4.5:1, else the nearest color toward white or
/// black that has (see the module notes for the direction).
pub fn adjust(text: Rgb, background: Rgb) -> Rgb {
    if ratio(text, background) >= MIN_RATIO {
        return text;
    }
    let lighter = luminance(text) >= luminance(background);
    for toward_white in [lighter, !lighter] {
        let end = if toward_white { WHITE } else { BLACK };
        if !reaches(end, background, toward_white) {
            continue;
        }
        // `low` never reaches, `high` always does.
        let (mut low, mut high) = (0f64, 1f64);
        for _ in 0..24 {
            let middle = (low + high) / 2.;
            if reaches(mix(text, end, middle), background, toward_white) {
                high = middle;
            } else {
                low = middle;
            }
        }
        return mix(text, end, high);
    }
    // Not reachable (one end always has more than 4.5:1); keep the better end.
    if ratio(WHITE, background) >= ratio(BLACK, background) {
        WHITE
    } else {
        BLACK
    }
}

/// Most pairs kept; the cache starts over when it is full.
const CACHE_LIMIT: usize = 4096;

fn bits(color: Rgb) -> u64 {
    (u64::from(color.r) << 16) | (u64::from(color.g) << 8) | u64::from(color.b)
}

/// [`adjust`] with a cache per (text, background) pair. A screen uses few
/// pairs, so rows are re-colored without computing a ratio per cell.
#[derive(Default)]
pub struct ContrastCache {
    pairs: HashMap<u64, Rgb>,
    /// The last pair asked for (cells of one run repeat it).
    last: Option<(u64, Rgb)>,
}

impl std::fmt::Debug for ContrastCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContrastCache")
            .field("pairs", &self.pairs.len())
            .finish_non_exhaustive()
    }
}

impl ContrastCache {
    /// The adjusted text color of a pair.
    pub fn adjust(&mut self, text: Rgb, background: Rgb) -> Rgb {
        let key = (bits(text) << 24) | bits(background);
        if let Some((last, color)) = self.last
            && last == key
        {
            return color;
        }
        if self.pairs.len() >= CACHE_LIMIT {
            self.pairs.clear();
        }
        let color = *self
            .pairs
            .entry(key)
            .or_insert_with(|| adjust(text, background));
        self.last = Some((key, color));
        color
    }

    /// Pairs computed and kept (tests).
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::colors::token_rgb;
    use crate::theme::terminal::{TerminalTheme, builtin};

    // Colors come from the built-in themes (design-system 4) or are grays
    // given by one channel value: no color literals outside src/theme.
    fn gray(level: u8) -> Rgb {
        Rgb {
            r: level,
            g: level,
            b: level,
        }
    }

    fn themes() -> (TerminalTheme, TerminalTheme) {
        let themes = builtin();
        (themes[0].clone(), themes[1].clone())
    }

    fn ansi(theme: &TerminalTheme, index: usize) -> Rgb {
        token_rgb(theme.ansi.0[index])
    }

    #[test]
    fn the_formula_matches_known_ratios() {
        assert!((ratio(WHITE, BLACK) - 21.).abs() < 1e-9);
        assert!((ratio(BLACK, BLACK) - 1.).abs() < 1e-9);
        // Paper's foreground on its background (design-system 4.2).
        let (_, paper) = themes();
        let quoted = ratio(token_rgb(paper.foreground), token_rgb(paper.background));
        assert!(quoted > 13. && quoted < 15., "{quoted}");
    }

    #[test]
    fn pairs_below_the_minimum_reach_it_and_others_stay() {
        let (graphite, paper) = themes();
        let pairs = [
            // Black text on ANSI blue / red / magenta / cyan (htop, mc in Paper).
            (ansi(&paper, 0), ansi(&paper, 4)),
            (ansi(&paper, 0), ansi(&paper, 1)),
            (ansi(&paper, 0), ansi(&paper, 5)),
            (ansi(&paper, 0), ansi(&paper, 6)),
            // Gray on gray, both directions.
            (gray(119), gray(136)),
            (gray(136), gray(119)),
            // Equal colors.
            (ansi(&graphite, 4), ansi(&graphite, 4)),
            // Dark text on a dark background (Graphite black on default).
            (ansi(&graphite, 0), token_rgb(graphite.background)),
            // Light text on white.
            (ansi(&graphite, 15), WHITE),
            // Middle grays: one end always reaches.
            (gray(118), gray(119)),
            (BLACK, gray(89)),
        ];
        for (text, background) in pairs {
            let adjusted = adjust(text, background);
            let value = ratio(adjusted, background);
            assert!(
                value >= MIN_RATIO,
                "{text:?} on {background:?} -> {adjusted:?}: {value}"
            );
        }
        // Enough contrast already: unchanged.
        for theme in [&graphite, &paper] {
            let (text, background) = (token_rgb(theme.foreground), token_rgb(theme.background));
            assert_eq!(adjust(text, background), text);
        }
    }

    #[test]
    fn text_moves_away_from_the_background_by_the_smallest_step() {
        // Lighter text on a dark background brightens.
        let background = gray(32);
        let adjusted = adjust(gray(64), background);
        assert!(luminance(adjusted) > luminance(gray(64)));
        // The step is minimal: one unit less toward white is not enough.
        let back_one = Rgb {
            r: adjusted.r - 1,
            g: adjusted.g - 1,
            b: adjusted.b - 1,
        };
        assert!(ratio(back_one, background) < MIN_RATIO);
        // Darker text on a light background darkens.
        let adjusted = adjust(gray(176), gray(224));
        assert!(luminance(adjusted) < luminance(gray(176)));
        // Light gray text slightly lighter than a light background cannot
        // reach 4.5 toward white: it goes to the dark side instead.
        let adjusted = adjust(gray(208), gray(200));
        assert!(luminance(adjusted) < luminance(gray(200)));
        assert!(ratio(adjusted, gray(200)) >= MIN_RATIO);
        // Paper's black on its blue: the darker blue cannot be passed by
        // going toward black, so the text turns light.
        let (_, paper) = themes();
        let blue = ansi(&paper, 4);
        let adjusted = adjust(ansi(&paper, 0), blue);
        assert!(
            luminance(adjusted) > luminance(blue),
            "black on blue turns light"
        );
    }

    #[test]
    fn modes_choose_what_is_adjusted() {
        assert!(!MinContrast::Off.applies(false));
        assert!(!MinContrast::Off.applies(true));
        assert!(MinContrast::Palette.applies(false));
        assert!(!MinContrast::Palette.applies(true));
        assert!(MinContrast::Always.applies(false));
        assert!(MinContrast::Always.applies(true));
        assert_eq!(MinContrast::default(), MinContrast::Palette);
        for mode in MinContrast::ALL {
            assert_eq!(MinContrast::from_key(mode.key()), Some(mode));
        }
        assert_eq!(MinContrast::from_key("sometimes"), None);
    }

    #[test]
    fn the_cache_keeps_one_result_per_pair() {
        let (_, paper) = themes();
        let mut cache = ContrastCache::default();
        let (text, background) = (ansi(&paper, 0), ansi(&paper, 4));
        let first = cache.adjust(text, background);
        assert_eq!(cache.adjust(text, background), first);
        assert_eq!(cache.len(), 1);
        cache.adjust(ansi(&paper, 0), ansi(&paper, 1));
        assert_eq!(cache.len(), 2);
        assert_eq!(first, adjust(text, background));
    }
}
