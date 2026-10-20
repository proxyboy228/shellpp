//! Terminal themes (docs/design-system.md, sections 4 and 7.2): the theme
//! format and the built-in themes stored as JSON in `src/theme/terminal/`,
//! one file per theme. Imported themes (later) use the same format.
//!
//! Version 1 accepts exactly the fields of 7.2: an unknown field, another
//! `version`, a name outside 1-64 characters or a color that is not
//! `#RRGGBB` (`#RRGGBBAA` for `selection`) is rejected.
//!
//! [`REGISTRY`] lists the built-in themes: our Graphite and Paper (4.1, 4.2)
//! and the bundled open color schemes (4.4) with their license and source.
//! Theme files are plain JSON (they double as examples of the import
//! format), so the source of each bundled scheme is recorded in a comment on
//! its registry entry below and in THIRD_PARTY_NOTICES.md ("Terminal
//! themes").

use std::sync::OnceLock;

use serde::Deserialize;

use super::palette::{Token, opaque, translucent};

/// Whether a theme has a dark or a light background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Dark,
    Light,
}

/// Where a bundled third-party scheme comes from (design-system 4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    /// SPDX license identifier.
    pub license: &'static str,
    /// The official definition the colors were copied from.
    pub source: &'static str,
}

/// One entry of [`REGISTRY`].
#[derive(Clone, Copy, Debug)]
pub struct BuiltinFile {
    /// The slug of `{"builtin": "<id>"}` references: the file name without
    /// `.json` (docs/data-model.md, `theme_ref`).
    pub id: &'static str,
    pub tone: Tone,
    /// Added in this release: theme lists mark it "new" (UI 4.2).
    pub new: bool,
    /// License and source of a bundled third-party scheme; `None` for our
    /// own themes.
    pub origin: Option<Origin>,
    /// The theme file (7.2).
    pub json: &'static str,
}

/// The built-in themes in list order: the default (Graphite) first, then
/// Paper, then the bundled open schemes of design-system 4.4.
///
/// The colors of a bundled scheme are copied unchanged from the source named
/// in its comment (retrieved 2026-10-20). Ours on top of them, by the rules
/// of 4.4: `searchMatch` and `searchMatchActive` (ANSI yellow over the
/// background at 25 % and 40 %) and, where the source names none, `cursor`
/// = `foreground` and `cursorText` = `background`.
pub const REGISTRY: [BuiltinFile; 10] = [
    BuiltinFile {
        id: "graphite",
        tone: Tone::Dark,
        new: false,
        origin: None,
        json: include_str!("terminal/graphite.json"),
    },
    BuiltinFile {
        id: "paper",
        tone: Tone::Light,
        new: false,
        origin: None,
        json: include_str!("terminal/paper.json"),
    },
    // Dracula: "Dracula Classic" palette and ANSI colors of the official
    // specification https://draculatheme.com/spec (Selection #44475A; the spec
    // names no cursor). License: MIT, https://github.com/dracula/dracula-theme
    // (LICENSE, "Copyright (c) 2023 Dracula Theme").
    BuiltinFile {
        id: "dracula",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://draculatheme.com/spec",
        }),
        json: include_str!("terminal/dracula.json"),
    },
    // Nord: the official Alacritty port, nordtheme/alacritty src/nord.yaml at
    // d6aca4d3e861ae17ee1066b51fb521f5145cdd91 (primary, cursor, selection
    // background, normal and bright colors; the selection keeps the cell
    // colors). License: MIT, "Copyright (c) 2016-present Sven Greb".
    BuiltinFile {
        id: "nord",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/nordtheme/alacritty/blob/main/src/nord.yaml",
        }),
        json: include_str!("terminal/nord.json"),
    },
    // Solarized Dark: altercation/solarized at
    // 62f656a02f93c5190a8753159e34b385588d5ff3, README "The Values" (the 16
    // colors with their 16/8 TERMCOL slots; background base03, body text base0;
    // highlighted text base02:base1 as the selection) and
    // vim-colors-solarized/colors/solarized.vim (Cursor: base0 with base03
    // text). License: MIT, "Copyright (c) 2011 Ethan Schoonover".
    BuiltinFile {
        id: "solarized-dark",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/altercation/solarized",
        }),
        json: include_str!("terminal/solarized-dark.json"),
    },
    // Solarized Light: the same source; the light mode swaps the monotones
    // (background base3, body text base00, selection base2:base01, cursor
    // base00 with base3 text), the 16 ANSI colors stay.
    BuiltinFile {
        id: "solarized-light",
        tone: Tone::Light,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/altercation/solarized",
        }),
        json: include_str!("terminal/solarized-light.json"),
    },
    // Gruvbox Dark: morhetz/gruvbox colors/gruvbox.vim at
    // 040138616bec342d5ea94d4db296f8ddca17007a, dark background, medium
    // contrast: Normal (fg1 on bg0), the Neovim g:terminal_color_0..15
    // mapping, Cursor (inverse: fg1 with bg0 text) and Visual (bg3, as with
    // g:gruvbox_invert_selection = 0). License: MIT/X11 as declared in the
    // README; the repository has no LICENSE file or copyright line (author
    // "morhetz").
    BuiltinFile {
        id: "gruvbox-dark",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim",
        }),
        json: include_str!("terminal/gruvbox-dark.json"),
    },
    // Tokyo Night: folke/tokyonight.nvim
    // extras/windows_terminal/tokyonight_night.json at
    // 6e0bfd6836e09a4c66184a33f1266e964f920299 (Apache-2.0), a port of the
    // VS Code theme enkia/tokyo-night-vscode-theme (MIT, "Copyright (c)
    // 2018-present Enkia").
    BuiltinFile {
        id: "tokyo-night",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "Apache-2.0",
            source: "https://github.com/folke/tokyonight.nvim/blob/main/extras/windows_terminal/tokyonight_night.json",
        }),
        json: include_str!("terminal/tokyo-night.json"),
    },
    // Catppuccin Mocha: catppuccin/palette palette.json v1.8.0 at
    // 07d02aa110ef9eb7e7427afca5c73ba9cf7f8ebd (base, text and the 16
    // ansiColors) and the style guide catppuccin/catppuccin
    // docs/style-guide.md (cursor Rosewater, cursor text Crust, selection
    // Overlay 2 at 30 %). License: MIT, "Copyright (c) 2021 Catppuccin".
    BuiltinFile {
        id: "catppuccin-mocha",
        tone: Tone::Dark,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/catppuccin/palette/blob/main/palette.json",
        }),
        json: include_str!("terminal/catppuccin-mocha.json"),
    },
    // Catppuccin Latte: the same sources (cursor Rosewater, cursor text Base,
    // selection Overlay 2 at 30 %).
    BuiltinFile {
        id: "catppuccin-latte",
        tone: Tone::Light,
        new: true,
        origin: Some(Origin {
            license: "MIT",
            source: "https://github.com/catppuccin/palette/blob/main/palette.json",
        }),
        json: include_str!("terminal/catppuccin-latte.json"),
    },
];

/// A parsed entry of [`REGISTRY`].
#[derive(Clone, Debug)]
pub struct BuiltinEntry {
    pub file: BuiltinFile,
    pub theme: TerminalTheme,
}

impl BuiltinEntry {
    pub fn id(&self) -> &'static str {
        self.file.id
    }

    /// The name in theme lists (the file's `name`).
    pub fn name(&self) -> &str {
        &self.theme.name
    }
}

/// The parsed registry, in [`REGISTRY`] order.
pub fn registry() -> &'static [BuiltinEntry] {
    static PARSED: OnceLock<Vec<BuiltinEntry>> = OnceLock::new();
    PARSED.get_or_init(|| {
        REGISTRY
            .iter()
            .map(|file| BuiltinEntry {
                file: *file,
                theme: TerminalTheme::parse(file.json).expect("built-in terminal theme"),
            })
            .collect()
    })
}

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

/// The built-in themes in [`REGISTRY`] order: Graphite (dark, default),
/// Paper (light), then the bundled open schemes.
pub fn builtin() -> Vec<TerminalTheme> {
    registry().iter().map(|entry| entry.theme.clone()).collect()
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
        assert_eq!(themes.len(), REGISTRY.len());
    }

    /// Every built-in theme parses and has every role of 7.2: the 16 ANSI
    /// colors, foreground, background, cursor, cursor text, selection and
    /// the two search colors.
    #[test]
    fn every_builtin_theme_has_all_roles() {
        for file in REGISTRY {
            let theme = TerminalTheme::parse(file.json)
                .unwrap_or_else(|error| panic!("{}: {error}", file.id));
            for key in AnsiColors::KEYS.into_iter().chain([
                "background",
                "foreground",
                "cursor",
                "cursorText",
                "selection",
                "searchMatch",
                "searchMatchActive",
            ]) {
                assert!(theme.role(key).is_some(), "{}: {key}", file.id);
            }
        }
    }

    /// Ids and names are unique, every id names its file in
    /// `src/theme/terminal/`, every file there is registered, and the core
    /// accepts exactly these slugs in `theme_ref`.
    #[test]
    fn registry_ids_are_unique_file_names_and_core_slugs() {
        let ids: Vec<&str> = REGISTRY.iter().map(|file| file.id).collect();
        let names: Vec<&str> = registry().iter().map(BuiltinEntry::name).collect();
        for (index, id) in ids.iter().enumerate() {
            assert!(!ids[..index].contains(id), "duplicate id {id}");
            assert!(!names[..index].contains(&names[index]), "duplicate name");
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/theme/terminal");
        for file in REGISTRY {
            let on_disk = std::fs::read_to_string(dir.join(format!("{}.json", file.id)))
                .unwrap_or_else(|error| panic!("{}: {error}", file.id));
            assert_eq!(on_disk, file.json, "{}", file.id);
        }
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .expect("theme directory")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        files.sort();
        let mut expected: Vec<String> = ids.iter().map(|id| format!("{id}.json")).collect();
        expected.sort();
        assert_eq!(files, expected);
        assert_eq!(shellpp_core::model::fields::BUILTIN_THEMES, ids.as_slice());
    }

    /// The registry's dark/light mark agrees with the background rule of
    /// design-system 4.3.
    #[test]
    fn tones_follow_the_background_rule() {
        for entry in registry() {
            let light = super::super::palette::is_light_background(entry.theme.background);
            assert_eq!(entry.file.tone == Tone::Light, light, "{}", entry.id());
        }
    }

    /// Graphite and Paper are ours and not new; every bundled scheme names a
    /// permissive license and its source, is marked new, and both tones are
    /// bundled (design-system 4.4).
    #[test]
    fn bundled_schemes_name_license_and_source() {
        assert_eq!([REGISTRY[0].id, REGISTRY[1].id], ["graphite", "paper"]);
        for file in &REGISTRY[..2] {
            assert!(file.origin.is_none() && !file.new, "{}", file.id);
        }
        let bundled = &REGISTRY[2..];
        assert!((6..=8).contains(&bundled.len()));
        for file in bundled {
            let origin = file
                .origin
                .unwrap_or_else(|| panic!("{}: no origin", file.id));
            assert!(
                ["MIT", "Apache-2.0"].contains(&origin.license),
                "{}",
                file.id
            );
            assert!(origin.source.starts_with("https://"), "{}", file.id);
            assert!(file.new, "{}", file.id);
        }
        for tone in [Tone::Dark, Tone::Light] {
            assert!(bundled.iter().any(|file| file.tone == tone), "{tone:?}");
        }
    }

    /// The search colors of a bundled scheme are ours: its ANSI yellow drawn
    /// over its background at 25 % (`searchMatch`) and 40 %
    /// (`searchMatchActive`), each channel rounded (design-system 4.4).
    #[test]
    fn bundled_search_colors_blend_yellow_over_the_background() {
        fn blend(background: u32, color: u32, amount: f64) -> u32 {
            [16u32, 8, 0].into_iter().fold(0, |rgb, shift| {
                let below = f64::from((background >> shift) & 0xFF);
                let above = f64::from((color >> shift) & 0xFF);
                let channel = (below + (above - below) * amount).round() as u32;
                rgb | (channel << shift)
            })
        }
        for entry in &registry()[2..] {
            let theme = &entry.theme;
            let background = theme.background.rgb;
            let yellow = theme.ansi.0[3].rgb;
            assert_eq!(
                theme.search_match,
                opaque(blend(background, yellow, 0.25)),
                "{}",
                entry.id()
            );
            assert_eq!(
                theme.search_match_active,
                opaque(blend(background, yellow, 0.4)),
                "{}",
                entry.id()
            );
        }
    }

    /// The table of design-system 4.4 (both languages) lists the bundled
    /// schemes in registry order, with their names, ids and licenses.
    #[test]
    fn design_system_lists_the_bundled_schemes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for path in ["docs/design-system.md", "docs/ru/design-system.md"] {
            let text = std::fs::read_to_string(root.join(path)).expect(path);
            let rows: Vec<Vec<&str>> = text
                .lines()
                .skip_while(|line| !line.starts_with("### 4.4 "))
                .skip(1)
                .take_while(|line| !line.starts_with('#'))
                .skip_while(|line| !line.starts_with('|'))
                .take_while(|line| line.starts_with('|'))
                .skip(2)
                .map(|line| line.trim_matches('|').split('|').map(str::trim).collect())
                .collect();
            let bundled = &registry()[2..];
            assert_eq!(rows.len(), bundled.len(), "{path}");
            for (row, entry) in rows.iter().zip(bundled) {
                assert_eq!(row[0], entry.name(), "{path}");
                assert_eq!(row[1], format!("`{}`", entry.id()), "{path}");
                let license = entry.file.origin.expect("origin").license;
                assert!(row[3].starts_with(license), "{path}: {}", entry.id());
            }
        }
    }

    /// THIRD_PARTY_NOTICES.md names every bundled scheme and its license in
    /// its "Terminal themes" section.
    #[test]
    fn notices_list_the_bundled_schemes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = std::fs::read_to_string(root.join("THIRD_PARTY_NOTICES.md")).expect("notices");
        let rows: Vec<&str> = text
            .lines()
            .skip_while(|line| *line != "## Terminal themes")
            .skip(1)
            .take_while(|line| !line.starts_with("## "))
            .filter(|line| line.starts_with("| "))
            .collect();
        for entry in &registry()[2..] {
            let license = entry.file.origin.expect("origin").license;
            assert!(
                rows.iter()
                    .any(|row| row.contains(entry.name()) && row.contains(license)),
                "{}",
                entry.id()
            );
        }
    }

    #[test]
    fn version_1_rejects_unknown_fields_and_versions() {
        let graphite = REGISTRY[0].json;
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
