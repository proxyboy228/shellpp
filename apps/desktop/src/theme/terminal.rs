//! Terminal themes (docs/design-system.md, sections 4 and 7.2): the theme
//! format and the built-in themes stored as JSON in `src/theme/terminal/`,
//! one file per theme. Imported themes (later) use the same format.
//!
//! Version 1 accepts exactly the fields of 7.2: an unknown field, another
//! `version`, a name outside 1-64 characters or a color that is not
//! `#RRGGBB` (`#RRGGBBAA` for `selection`) is rejected.

use serde::Deserialize;

use super::palette::{Token, opaque, translucent};

/// The built-in theme files, default first.
pub const BUILTIN_FILES: [&str; 2] = [
    include_str!("terminal/graphite.json"),
    include_str!("terminal/paper.json"),
];

/// Why a theme file was rejected.
#[derive(Debug, thiserror::Error)]
pub enum TerminalThemeError {
    #[error("the theme is not valid JSON or has unknown fields: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported theme version {0}")]
    Version(u32),
    #[error("the theme name must have 1-64 characters")]
    Name,
    #[error("{field} is not a valid color")]
    Color { field: &'static str },
}

/// The 16 ANSI colors in color index order 0-15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnsiColors(pub [Token; 16]);

impl AnsiColors {
    /// The key names of `ansi`, in color index order.
    pub const KEYS: [&'static str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "brightBlack",
        "brightRed",
        "brightGreen",
        "brightYellow",
        "brightBlue",
        "brightMagenta",
        "brightCyan",
        "brightWhite",
    ];

    /// The color of `key` (one of [`Self::KEYS`]).
    pub fn get(&self, key: &str) -> Option<Token> {
        Self::KEYS
            .iter()
            .position(|name| *name == key)
            .map(|index| self.0[index])
    }
}

/// A parsed terminal theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalTheme {
    pub name: String,
    pub background: Token,
    pub foreground: Token,
    pub cursor: Token,
    pub cursor_text: Token,
    /// Drawn over `background`.
    pub selection: Token,
    /// When absent, selected text keeps its colors.
    pub selection_foreground: Option<Token>,
    pub search_match: Token,
    pub search_match_active: Token,
    pub ansi: AnsiColors,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ThemeFile {
    name: String,
    version: u32,
    background: String,
    foreground: String,
    cursor: String,
    cursor_text: String,
    selection: String,
    #[serde(default)]
    selection_foreground: Option<String>,
    search_match: String,
    search_match_active: String,
    ansi: AnsiFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AnsiFile {
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

impl TerminalTheme {
    /// Parses and validates a theme file (design-system 7.2, version 1).
    pub fn parse(json: &str) -> Result<Self, TerminalThemeError> {
        let file: ThemeFile = serde_json::from_str(json)?;
        if file.version != 1 {
            return Err(TerminalThemeError::Version(file.version));
        }
        let length = file.name.chars().count();
        if !(1..=64).contains(&length) {
            return Err(TerminalThemeError::Name);
        }
        let color = |value: &str, field: &'static str| {
            parse_color(value, false).ok_or(TerminalThemeError::Color { field })
        };
        let a = &file.ansi;
        let ansi = [
            color(&a.black, "black")?,
            color(&a.red, "red")?,
            color(&a.green, "green")?,
            color(&a.yellow, "yellow")?,
            color(&a.blue, "blue")?,
            color(&a.magenta, "magenta")?,
            color(&a.cyan, "cyan")?,
            color(&a.white, "white")?,
            color(&a.bright_black, "brightBlack")?,
            color(&a.bright_red, "brightRed")?,
            color(&a.bright_green, "brightGreen")?,
            color(&a.bright_yellow, "brightYellow")?,
            color(&a.bright_blue, "brightBlue")?,
            color(&a.bright_magenta, "brightMagenta")?,
            color(&a.bright_cyan, "brightCyan")?,
            color(&a.bright_white, "brightWhite")?,
        ];
        Ok(Self {
            background: color(&file.background, "background")?,
            foreground: color(&file.foreground, "foreground")?,
            cursor: color(&file.cursor, "cursor")?,
            cursor_text: color(&file.cursor_text, "cursorText")?,
            selection: parse_color(&file.selection, true)
                .ok_or(TerminalThemeError::Color { field: "selection" })?,
            selection_foreground: file
                .selection_foreground
                .as_deref()
                .map(|value| color(value, "selectionForeground"))
                .transpose()?,
            search_match: color(&file.search_match, "searchMatch")?,
            search_match_active: color(&file.search_match_active, "searchMatchActive")?,
            ansi: AnsiColors(ansi),
            name: file.name,
        })
    }

    /// The color of a role key of the theme file (`background`,
    /// `cursorText`, `brightRed`, ...). `None` for an unknown key or an unset
    /// `selectionForeground`.
    pub fn role(&self, key: &str) -> Option<Token> {
        match key {
            "background" => Some(self.background),
            "foreground" => Some(self.foreground),
            "cursor" => Some(self.cursor),
            "cursorText" => Some(self.cursor_text),
            "selection" => Some(self.selection),
            "selectionForeground" => self.selection_foreground,
            "searchMatch" => Some(self.search_match),
            "searchMatchActive" => Some(self.search_match_active),
            other => self.ansi.get(other),
        }
    }
}

/// The built-in themes: Graphite (dark, default) and Paper (light).
pub fn builtin() -> Vec<TerminalTheme> {
    BUILTIN_FILES
        .iter()
        .map(|file| TerminalTheme::parse(file).expect("built-in terminal theme"))
        .collect()
}

/// `#RRGGBB`, or `#RRGGBBAA` when `alpha` is allowed. Hex digits may be
/// upper or lower case.
fn parse_color(value: &str, alpha: bool) -> Option<Token> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => u32::from_str_radix(hex, 16).ok().map(opaque),
        8 if alpha => {
            let value = u32::from_str_radix(hex, 16).ok()?;
            Some(translucent(value >> 8, (value & 0xFF) as u8))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_themes_parse() {
        let themes = builtin();
        assert_eq!(themes[0].name, "Graphite");
        assert_eq!(themes[1].name, "Paper");
        assert_eq!(themes[0].selection, translucent(0x2EC4A6, 0x4D));
        assert_eq!(themes[0].selection_foreground, None);
        assert_eq!(themes[1].ansi.get("brightRed"), Some(opaque(0xC6483F)));
    }

    #[test]
    fn version_1_rejects_unknown_fields_and_versions() {
        let graphite = BUILTIN_FILES[0];
        let unknown = graphite.replacen("\"version\": 1,", "\"version\": 1, \"extra\": 1,", 1);
        assert!(matches!(
            TerminalTheme::parse(&unknown),
            Err(TerminalThemeError::Json(_))
        ));
        let version = graphite.replacen("\"version\": 1", "\"version\": 2", 1);
        assert!(matches!(
            TerminalTheme::parse(&version),
            Err(TerminalThemeError::Version(2))
        ));
        let empty_name = graphite.replacen("\"Graphite\"", "\"\"", 1);
        assert!(matches!(
            TerminalTheme::parse(&empty_name),
            Err(TerminalThemeError::Name)
        ));
        let bad_color = graphite.replacen("#DCDCD5", "#DCDCD", 1);
        assert!(matches!(
            TerminalTheme::parse(&bad_color),
            Err(TerminalThemeError::Color {
                field: "foreground"
            })
        ));
        let lower = graphite.replacen("#DCDCD5", "#dcdcd5", 1);
        assert_eq!(
            TerminalTheme::parse(&lower).unwrap().foreground,
            opaque(0xDCDCD5)
        );
    }
}
