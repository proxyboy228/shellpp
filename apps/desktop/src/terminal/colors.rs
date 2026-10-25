//! Cell colors: the model's palette (`shellpp-terminal`, run 13) from the
//! desktop's terminal themes (design-system 4.1-4.2, 7.2). A theme's tokens
//! become plain RGB for the model, and the model's RGB becomes theme tokens
//! again for drawing (`.hsla()`; the conversion lives with the theme,
//! design-system 7.1).

use alacritty_terminal::vte::ansi::Rgb;
pub use shellpp_terminal::colors::*;

use crate::theme::palette::Token;
use crate::theme::terminal::TerminalTheme;

/// An opaque [`Token`] as an `Rgb` (the alpha byte is dropped).
pub fn token_rgb(token: Token) -> Rgb {
    Rgb {
        r: ((token.rgb >> 16) & 0xFF) as u8,
        g: ((token.rgb >> 8) & 0xFF) as u8,
        b: (token.rgb & 0xFF) as u8,
    }
}

/// An `Rgb` as an opaque theme [`Token`]; `.hsla()` gives the GPUI color.
pub fn rgb_token(color: Rgb) -> Token {
    Token {
        rgb: (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b),
        alpha: u8::MAX,
    }
}

/// A translucent model color (the selection) as a theme [`Token`].
pub fn translucent_token(color: Translucent) -> Token {
    Token {
        alpha: color.alpha,
        ..rgb_token(color.rgb)
    }
}

/// The palette of a terminal theme.
pub trait FromTheme {
    fn from_theme(theme: &TerminalTheme) -> Self;
}

impl FromTheme for Palette {
    fn from_theme(theme: &TerminalTheme) -> Self {
        Palette::new(PaletteColors {
            ansi: theme.ansi.0.map(token_rgb),
            foreground: token_rgb(theme.foreground),
            background: token_rgb(theme.background),
            cursor: token_rgb(theme.cursor),
            cursor_text: token_rgb(theme.cursor_text),
            selection: Translucent {
                rgb: token_rgb(theme.selection),
                alpha: theme.selection.alpha,
            },
            selection_foreground: theme.selection_foreground.map(token_rgb),
            search_match: token_rgb(theme.search_match),
            search_match_active: token_rgb(theme.search_match_active),
        })
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::cell::Flags;
    use alacritty_terminal::term::color::Colors;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    use super::*;
    use crate::theme::terminal::builtin;

    /// A theme's selection keeps its color and alpha through the model's
    /// palette and back (review of run 13).
    #[test]
    fn the_selection_keeps_its_theme_color_and_alpha() {
        for theme in builtin() {
            let palette = Palette::from_theme(&theme);
            assert_eq!(translucent_token(palette.selection()), theme.selection);
        }
    }

    #[test]
    fn the_256_color_cube_and_grays() {
        let level = |r, g, b| Rgb { r, g, b };
        assert_eq!(xterm_256(16), Some(level(0, 0, 0)));
        assert_eq!(xterm_256(196), Some(level(255, 0, 0)));
        assert_eq!(xterm_256(231), Some(level(255, 255, 255)));
        assert_eq!(xterm_256(232), Some(level(8, 8, 8)));
        assert_eq!(xterm_256(255), Some(level(238, 238, 238)));
        assert_eq!(xterm_256(15), None);
    }

    #[test]
    fn theme_colors_overrides_and_attributes() {
        let themes = builtin();
        let palette = Palette::from_theme(&themes[0]);
        let overrides = Colors::default();
        let red = token_rgb(themes[0].ansi.0[1]);
        let bright_red = token_rgb(themes[0].ansi.0[9]);
        assert_eq!(palette.indexed(1), red);
        assert_eq!(palette.named(NamedColor::Red), red);
        let spec = Rgb { r: 1, g: 2, b: 3 };
        assert_eq!(palette.resolve(Color::Spec(spec), &overrides), spec);

        // Bold red is bright red only with the setting.
        let bold = Flags::BOLD;
        let fg = Color::Named(NamedColor::Red);
        let bg = Color::Named(NamedColor::Background);
        assert_eq!(
            palette.cell_colors(fg, bg, bold, true, &overrides),
            (bright_red, None)
        );
        assert_eq!(
            palette.cell_colors(fg, bg, bold, false, &overrides),
            (red, None)
        );
        assert_eq!(
            palette
                .cell_colors(Color::Indexed(1), bg, bold, true, &overrides)
                .0,
            bright_red
        );

        // Inverse: the default background becomes the text color.
        let (text, background) = palette.cell_colors(fg, bg, Flags::INVERSE, true, &overrides);
        assert_eq!(text, palette.background());
        assert_eq!(background, Some(red));

        // An OSC 4 override wins over the theme.
        let mut changed = Colors::default();
        changed[1] = Some(spec);
        assert_eq!(palette.resolve(Color::Indexed(1), &changed), spec);
        assert_eq!(palette.table_entry(1, &changed), spec);
        assert_eq!(palette.table_entry(257, &overrides), palette.background());
    }
}

#[cfg(test)]
mod contrast_tests {
    use alacritty_terminal::vte::ansi::Rgb;
    use shellpp_terminal::contrast::*;

    use super::token_rgb;
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
