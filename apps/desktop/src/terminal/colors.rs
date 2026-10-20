//! Cell colors from the terminal theme (design-system 4.1-4.2, 7.2).
//!
//! The 16 ANSI colors, the foreground, the background and the cursor come
//! from the theme; the 256-color palette above 15 is the standard xterm
//! 6x6x6 cube and gray ramp, computed (not stored); truecolor (`SGR 38;2`)
//! is used as sent. Colors that the remote side changed with OSC 4/10/11/12
//! (alacritty's `Colors` overrides) win over the theme.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

use crate::theme::palette::Token;
use crate::theme::terminal::TerminalTheme;

/// Brightness factor of dim text (SGR 2) and of the dim named colors.
const DIM_FACTOR: f32 = 0.66;

/// The resolved colors of one terminal theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    ansi: [Rgb; 16],
    foreground: Rgb,
    background: Rgb,
    cursor: Rgb,
    cursor_text: Rgb,
    selection: Token,
    selection_foreground: Option<Rgb>,
}

/// An opaque [`Token`] as an `Rgb` (the alpha byte is dropped).
pub fn token_rgb(token: Token) -> Rgb {
    Rgb {
        r: ((token.rgb >> 16) & 0xFF) as u8,
        g: ((token.rgb >> 8) & 0xFF) as u8,
        b: (token.rgb & 0xFF) as u8,
    }
}

/// An `Rgb` as an opaque theme [`Token`]; `.hsla()` gives the GPUI color
/// (the conversion lives with the theme, design-system 7.1).
pub fn rgb_token(color: Rgb) -> Token {
    Token {
        rgb: (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b),
        alpha: u8::MAX,
    }
}

fn dim(color: Rgb) -> Rgb {
    color * DIM_FACTOR
}

/// Index 16-255 of the xterm 256-color palette: a 6x6x6 cube
/// (levels 0, 95, 135, 175, 215, 255) and 24 grays (8, 18, ..., 238).
pub fn xterm_256(index: u8) -> Option<Rgb> {
    match index {
        16..=231 => {
            let cube = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + 40 * value };
            Some(Rgb {
                r: level(cube / 36),
                g: level((cube / 6) % 6),
                b: level(cube % 6),
            })
        }
        232..=255 => {
            let gray = 8 + 10 * (index - 232);
            Some(Rgb {
                r: gray,
                g: gray,
                b: gray,
            })
        }
        _ => None,
    }
}

impl Palette {
    pub fn from_theme(theme: &TerminalTheme) -> Self {
        Self {
            ansi: theme.ansi.0.map(token_rgb),
            foreground: token_rgb(theme.foreground),
            background: token_rgb(theme.background),
            cursor: token_rgb(theme.cursor),
            cursor_text: token_rgb(theme.cursor_text),
            selection: theme.selection,
            selection_foreground: theme.selection_foreground.map(token_rgb),
        }
    }

    pub fn foreground(&self) -> Rgb {
        self.foreground
    }

    pub fn background(&self) -> Rgb {
        self.background
    }

    pub fn cursor(&self) -> Rgb {
        self.cursor
    }

    pub fn cursor_text(&self) -> Rgb {
        self.cursor_text
    }

    /// The selection color, translucent, drawn over the cell backgrounds.
    pub fn selection(&self) -> Token {
        self.selection
    }

    /// The text color of selected cells when the theme sets one.
    pub fn selection_foreground(&self) -> Option<Rgb> {
        self.selection_foreground
    }

    /// Color `index` of the 256-color palette (0-15 from the theme).
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[usize::from(index)],
            _ => xterm_256(index).unwrap_or(self.foreground),
        }
    }

    /// A named color of the theme.
    pub fn named(&self, name: NamedColor) -> Rgb {
        let index = name as usize;
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            _ if index < 16 => self.ansi[index],
            // DimBlack ..= DimWhite follow the eight normal colors.
            _ => dim(self.ansi[(index - NamedColor::DimBlack as usize) % 8]),
        }
    }

    /// The color behind entry `index` of alacritty's color table (0-255,
    /// then the named colors from 256): the answer to an OSC 4/10/11 query.
    pub fn table_entry(&self, index: usize, overrides: &Colors) -> Rgb {
        if let Some(color) = (index < COUNT).then(|| overrides[index]).flatten() {
            return color;
        }
        match u8::try_from(index) {
            Ok(index) => self.indexed(index),
            Err(_) => match index {
                256 => self.foreground,
                257 => self.background,
                258 => self.cursor,
                _ => self.foreground,
            },
        }
    }

    /// A cell color with the remote overrides applied.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(index) => {
                overrides[usize::from(index)].unwrap_or_else(|| self.indexed(index))
            }
            Color::Named(name) => overrides[name].unwrap_or_else(|| self.named(name)),
        }
    }

    /// The foreground and background of a cell. The background is `None`
    /// when it is the default background (nothing to paint).
    ///
    /// - Bold text in one of the eight normal colors uses the bright
    ///   variant when `bold_is_bright` (UI 12.4 "Bright colors for bold text").
    /// - Dim (SGR 2) uses the dim variant.
    /// - Inverse (SGR 7) swaps the two; hidden (SGR 8) paints the text in
    ///   the background color.
    pub fn cell_colors(
        &self,
        fg: Color,
        bg: Color,
        flags: Flags,
        bold_is_bright: bool,
        overrides: &Colors,
    ) -> (Rgb, Option<Rgb>) {
        let fg = match fg {
            Color::Named(name) if flags.contains(Flags::DIM) => Color::Named(name.to_dim()),
            Color::Named(name)
                if bold_is_bright && flags.contains(Flags::BOLD) && (name as usize) < 8 =>
            {
                Color::Named(name.to_bright())
            }
            Color::Indexed(index) if bold_is_bright && flags.contains(Flags::BOLD) && index < 8 => {
                Color::Indexed(index + 8)
            }
            other => other,
        };
        let mut fg_rgb = self.resolve(fg, overrides);
        if flags.contains(Flags::DIM) && !matches!(fg, Color::Named(_)) {
            fg_rgb = dim(fg_rgb);
        }
        let default_bg = matches!(bg, Color::Named(NamedColor::Background));
        let bg_rgb = self.resolve(bg, overrides);
        let (fg_rgb, bg_rgb) = if flags.contains(Flags::INVERSE) {
            (bg_rgb, Some(fg_rgb))
        } else {
            (fg_rgb, (!default_bg).then_some(bg_rgb))
        };
        if flags.contains(Flags::HIDDEN) {
            return (bg_rgb.unwrap_or(self.background), bg_rgb);
        }
        (fg_rgb, bg_rgb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::terminal::builtin;

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
