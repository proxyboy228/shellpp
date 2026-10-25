//! The terminal palettes: the desktop's built-in themes, read from its theme
//! files (design-system 4.1, 4.2, 7.2), as plain RGB for the model.
//! Graphite (dark) and Paper (light) follow the system's mode unless
//! Settings → Font & colours or the host picks another (run 15).

use alacritty_terminal::vte::ansi::Rgb;
use serde::Deserialize;
use shellpp_terminal::colors::{Palette, PaletteColors, Translucent};

const GRAPHITE: &str = include_str!("../../../desktop/src/theme/terminal/graphite.json");
const PAPER: &str = include_str!("../../../desktop/src/theme/terminal/paper.json");

/// Every built-in theme by its slug (`shellpp_core::model::fields::BUILTIN_THEMES`).
const BUILT_IN: &[(&str, &str)] = &[
    ("graphite", GRAPHITE),
    ("paper", PAPER),
    (
        "dracula",
        include_str!("../../../desktop/src/theme/terminal/dracula.json"),
    ),
    (
        "nord",
        include_str!("../../../desktop/src/theme/terminal/nord.json"),
    ),
    (
        "solarized-dark",
        include_str!("../../../desktop/src/theme/terminal/solarized-dark.json"),
    ),
    (
        "solarized-light",
        include_str!("../../../desktop/src/theme/terminal/solarized-light.json"),
    ),
    (
        "gruvbox-dark",
        include_str!("../../../desktop/src/theme/terminal/gruvbox-dark.json"),
    ),
    (
        "tokyo-night",
        include_str!("../../../desktop/src/theme/terminal/tokyo-night.json"),
    ),
    (
        "catppuccin-mocha",
        include_str!("../../../desktop/src/theme/terminal/catppuccin-mocha.json"),
    ),
    (
        "catppuccin-latte",
        include_str!("../../../desktop/src/theme/terminal/catppuccin-latte.json"),
    ),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThemeFile {
    background: String,
    foreground: String,
    cursor: String,
    cursor_text: String,
    selection: String,
    #[serde(default)]
    selection_foreground: Option<String>,
    search_match: String,
    search_match_active: String,
    ansi: Ansi,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Ansi {
    black: String,
    red: String,
    green: String,
    yellow: String,
    blue: String,
    magenta: String,
    cyan: String,
    white: String,
    bright_black: String,
    bright_red: String,
    bright_green: String,
    bright_yellow: String,
    bright_blue: String,
    bright_magenta: String,
    bright_cyan: String,
    bright_white: String,
}

/// `#RRGGBB` or `#RRGGBBAA` as the color and its alpha byte.
fn hex(text: &str) -> Option<(Rgb, u8)> {
    let digits = text.strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    let alpha = match digits.len() {
        6 => u8::MAX,
        8 => byte(6)?,
        _ => return None,
    };
    Some((
        Rgb {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        },
        alpha,
    ))
}

fn palette_of(json: &str) -> Option<Palette> {
    let theme: ThemeFile = serde_json::from_str(json).ok()?;
    let rgb = |text: &str| hex(text).map(|(color, _)| color);
    let a = &theme.ansi;
    let ansi = [
        &a.black,
        &a.red,
        &a.green,
        &a.yellow,
        &a.blue,
        &a.magenta,
        &a.cyan,
        &a.white,
        &a.bright_black,
        &a.bright_red,
        &a.bright_green,
        &a.bright_yellow,
        &a.bright_blue,
        &a.bright_magenta,
        &a.bright_cyan,
        &a.bright_white,
    ]
    .map(|text| rgb(text));
    let (selection, alpha) = hex(&theme.selection)?;
    Some(Palette::new(PaletteColors {
        ansi: [
            ansi[0]?, ansi[1]?, ansi[2]?, ansi[3]?, ansi[4]?, ansi[5]?, ansi[6]?, ansi[7]?,
            ansi[8]?, ansi[9]?, ansi[10]?, ansi[11]?, ansi[12]?, ansi[13]?, ansi[14]?, ansi[15]?,
        ],
        foreground: rgb(&theme.foreground)?,
        background: rgb(&theme.background)?,
        cursor: rgb(&theme.cursor)?,
        cursor_text: rgb(&theme.cursor_text)?,
        selection: Translucent {
            rgb: selection,
            alpha,
        },
        selection_foreground: match &theme.selection_foreground {
            Some(text) => Some(rgb(text)?),
            None => None,
        },
        search_match: rgb(&theme.search_match)?,
        search_match_active: rgb(&theme.search_match_active)?,
    }))
}

/// The palette of the built-in theme `slug`, else of the dark (Graphite) or
/// light (Paper) one.
pub fn palette_for(slug: Option<&str>, dark: bool) -> Palette {
    slug.and_then(|slug| BUILT_IN.iter().find(|(name, _)| *name == slug))
        .and_then(|(_, json)| palette_of(json))
        .unwrap_or_else(|| palette(dark))
}

/// The palette of the dark (Graphite) or light (Paper) built-in theme.
pub fn palette(dark: bool) -> Palette {
    let json = if dark { GRAPHITE } else { PAPER };
    // The files are the desktop's built-in themes, checked by its tests and
    // by `the_built_in_themes_read` below.
    palette_of(json).unwrap_or_else(|| panic!("a built-in theme does not read"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_themes_read() {
        for (slug, json) in BUILT_IN {
            assert!(palette_of(json).is_some(), "{slug}");
        }
        assert_eq!(
            BUILT_IN.len(),
            shellpp_core::model::fields::BUILTIN_THEMES.len()
        );
        assert_ne!(
            palette_for(Some("dracula"), true).background(),
            palette(true).background()
        );
        assert_eq!(
            palette_for(Some("no-such-theme"), false).background(),
            palette(false).background()
        );
        assert!(palette_of(GRAPHITE).is_some());
        assert!(palette_of(PAPER).is_some());
        assert_ne!(palette(true).background(), palette(false).background());
    }

    #[test]
    fn hex_colors_read_with_and_without_alpha() {
        assert_eq!(
            hex("#0A0B0C"),
            Some((
                Rgb {
                    r: 10,
                    g: 11,
                    b: 12
                },
                255
            ))
        );
        assert_eq!(
            hex("#0A0B0C4D"),
            Some((
                Rgb {
                    r: 10,
                    g: 11,
                    b: 12
                },
                0x4D
            ))
        );
        assert_eq!(hex("0A0B0C"), None);
        assert_eq!(hex("#0A0B"), None);
        assert_eq!(hex("#GG0000"), None);
    }
}
