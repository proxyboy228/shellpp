//! The terminal font (design-system 5, decision 30): JetBrains Mono 2.304
//! (OFL-1.1) Regular, Bold, Italic and Bold Italic, embedded in the binary
//! and registered with GPUI's text system at startup. The OFL text ships
//! with the fonts (`assets/fonts/OFL.txt`, also embedded as
//! [`JETBRAINS_MONO_LICENSE`]).
//!
//! Characters the font lacks (CJK, emoji) come from the system fallback;
//! the system emoji font is named first for emoji.

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{App, Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Global, font};

/// The family name of the embedded font (`terminal.font_family` default).
pub const JETBRAINS_MONO: &str = "JetBrains Mono";

pub const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
pub const JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf");
pub const JETBRAINS_MONO_ITALIC: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Italic.ttf");
pub const JETBRAINS_MONO_BOLD_ITALIC: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-BoldItalic.ttf");
/// The SIL Open Font License 1.1 of JetBrains Mono.
pub const JETBRAINS_MONO_LICENSE: &str = include_str!("../../assets/fonts/OFL.txt");

/// The system emoji fonts, tried first for characters the terminal font lacks.
#[cfg(target_os = "windows")]
const EMOJI_FONTS: &[&str] = &["Segoe UI Emoji", "Segoe UI Symbol"];
#[cfg(target_os = "macos")]
const EMOJI_FONTS: &[&str] = &["Apple Color Emoji"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const EMOJI_FONTS: &[&str] = &["Noto Color Emoji"];

/// Marks the embedded fonts as registered.
struct FontsRegistered;

impl Global for FontsRegistered {}

/// Registers the embedded fonts once per app.
pub fn register(cx: &mut App) {
    if cx.has_global::<FontsRegistered>() {
        return;
    }
    let fonts = vec![
        Cow::Borrowed(JETBRAINS_MONO_REGULAR),
        Cow::Borrowed(JETBRAINS_MONO_BOLD),
        Cow::Borrowed(JETBRAINS_MONO_ITALIC),
        Cow::Borrowed(JETBRAINS_MONO_BOLD_ITALIC),
    ];
    if cx.text_system().add_fonts(fonts).is_err() {
        tracing::warn!("the embedded terminal font did not load; installed fonts are used");
    }
    cx.set_global(FontsRegistered);
}

/// The terminal font in `family`: no ligatures (each character stays in
/// its cell), the system emoji font as the first fallback.
pub fn terminal_font(family: &str, bold: bool, italic: bool) -> Font {
    let mut font = font(family.to_owned());
    font.features = FontFeatures(Arc::new(vec![
        ("calt".to_owned(), 0),
        ("liga".to_owned(), 0),
    ]));
    font.fallbacks = Some(FontFallbacks::from_fonts(
        EMOJI_FONTS.iter().map(|name| (*name).to_owned()).collect(),
    ));
    font.weight = if bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    };
    font.style = if italic {
        FontStyle::Italic
    } else {
        FontStyle::Normal
    };
    font
}

/// The four faces of the terminal font, built once per family (a `Font`
/// carries its features and fallbacks; text runs clone these).
#[derive(Clone, Debug, PartialEq)]
pub struct FontSet {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
}

impl FontSet {
    pub fn new(family: &str) -> Self {
        Self {
            regular: terminal_font(family, false, false),
            bold: terminal_font(family, true, false),
            italic: terminal_font(family, false, true),
            bold_italic: terminal_font(family, true, true),
        }
    }

    /// The family name.
    pub fn family(&self) -> &str {
        &self.regular.family
    }

    /// The face for a cell's attributes.
    pub fn get(&self, bold: bool, italic: bool) -> &Font {
        match (bold, italic) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_fonts_are_jetbrains_mono_2_304_with_the_ofl() {
        for data in [
            JETBRAINS_MONO_REGULAR,
            JETBRAINS_MONO_BOLD,
            JETBRAINS_MONO_ITALIC,
            JETBRAINS_MONO_BOLD_ITALIC,
        ] {
            // TrueType outlines.
            assert_eq!(&data[..4], [0, 1, 0, 0]);
            // The name table holds "Version 2.304" in UTF-16BE.
            let version: Vec<u8> = "Version 2.304"
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect();
            assert!(
                data.windows(version.len()).any(|window| window == version),
                "font version"
            );
        }
        assert!(JETBRAINS_MONO_LICENSE.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(JETBRAINS_MONO_LICENSE.contains("The JetBrains Mono Project Authors"));
    }

    #[test]
    fn the_terminal_font_has_no_ligatures_and_an_emoji_fallback() {
        let font = terminal_font(JETBRAINS_MONO, true, true);
        assert_eq!(font.weight, FontWeight::BOLD);
        assert_eq!(font.style, FontStyle::Italic);
        assert_eq!(font.features.is_calt_enabled(), Some(false));
        let fallbacks = font.fallbacks.expect("fallbacks");
        assert_eq!(fallbacks.fallback_list()[0], EMOJI_FONTS[0]);
    }
}
