//! Cell colors from the terminal theme (design-system 4.1-4.2, 7.2).
//!
//! The 16 ANSI colors, the foreground, the background and the cursor come
//! from the theme; the 256-color palette above 15 is the standard xterm
//! 6x6x6 cube and gray ramp, computed (not stored); truecolor (`SGR 38;2`)
//! is used as sent. Colors that the remote side changed with OSC 4/10/11/12
//! (alacritty's `Colors` overrides) win over the theme.
//!
//! The theme itself belongs to each client: it hands its colors over as
//! plain RGB ([`PaletteColors`]); the desktop reads its terminal themes in
//! `apps/desktop/src/terminal/colors.rs`.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

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
    selection: Translucent,
    selection_foreground: Option<Rgb>,
    search_match: Rgb,
    search_match_active: Rgb,
}

/// A color with an alpha byte (the selection, drawn over the cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Translucent {
    pub rgb: Rgb,
    pub alpha: u8,
}

/// The colors of a terminal theme, as a client hands them over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaletteColors {
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection: Translucent,
    pub selection_foreground: Option<Rgb>,
    pub search_match: Rgb,
    pub search_match_active: Rgb,
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
    pub fn new(colors: PaletteColors) -> Self {
        Self {
            ansi: colors.ansi,
            foreground: colors.foreground,
            background: colors.background,
            cursor: colors.cursor,
            cursor_text: colors.cursor_text,
            selection: colors.selection,
            selection_foreground: colors.selection_foreground,
            search_match: colors.search_match,
            search_match_active: colors.search_match_active,
        }
    }

    /// The background of search matches (design-system 7.2 `searchMatch`).
    pub fn search_match(&self) -> Rgb {
        self.search_match
    }

    /// The background of the current search match (`searchMatchActive`).
    pub fn search_match_active(&self) -> Rgb {
        self.search_match_active
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
    pub fn selection(&self) -> Translucent {
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

    /// Whether the text of a cell is drawn in a truecolor (`SGR 38;2`, or
    /// `48;2` for inverse text) rather than in a palette color: the
    /// "Palette only" minimum contrast leaves such text alone.
    pub fn text_is_truecolor(fg: Color, bg: Color, flags: Flags) -> bool {
        let text = if flags.contains(Flags::INVERSE) {
            bg
        } else {
            fg
        };
        matches!(text, Color::Spec(_))
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

/// A palette of grays and plain primaries for the crate's tests (no theme
/// here; the desktop tests its themes' palettes).
#[cfg(test)]
pub fn test_palette() -> Palette {
    let gray = |level: u8| Rgb {
        r: level,
        g: level,
        b: level,
    };
    let mut ansi = [gray(0); 16];
    for (index, color) in ansi.iter_mut().enumerate() {
        let on = |bit: usize| if index & bit != 0 { 205 } else { 0 };
        *color = Rgb {
            r: on(1),
            g: on(2),
            b: on(4),
        };
    }
    Palette::new(PaletteColors {
        ansi,
        foreground: gray(229),
        background: gray(16),
        cursor: gray(229),
        cursor_text: gray(16),
        selection: Translucent {
            rgb: gray(128),
            alpha: 96,
        },
        selection_foreground: None,
        search_match: gray(96),
        search_match_active: gray(160),
    })
}
