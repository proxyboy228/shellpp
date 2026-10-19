//! Test: the theme values agree with the tables of docs/design-system.md
//! (sections 2-6 and 7.4) and with the same tables of
//! docs/ru/design-system.md (design-system 7.1).
//!
//! The tables are read from the documents at test time, so a value changed
//! in a document without the theme (or the other way round) fails here.

use std::path::PathBuf;

use gpui::{Hsla, TestAppContext};
use gpui_kit::component::{ActiveTheme as _, ThemeColor};

use super::palette::{self, DARK, InterfaceTokens, LIGHT, Shadow, Token, opaque, token_by_name};
use super::terminal::{self, TerminalTheme};
use super::{Appearance, THEME_COLOR_MAPPING, install, metrics};

fn doc(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Both language versions of the design system.
fn docs() -> [(&'static str, String); 2] {
    [
        ("docs/design-system.md", doc("docs/design-system.md")),
        ("docs/ru/design-system.md", doc("docs/ru/design-system.md")),
    ]
}

/// The lines of section `number` ("2.1", "7.4"): from its heading to the
/// next heading of any level.
fn section<'a>(text: &'a str, number: &str) -> Vec<&'a str> {
    let heading = format!("### {number} ");
    let mut lines = text.lines().skip_while(|line| !line.starts_with(&heading));
    assert!(lines.next().is_some(), "section {number} missing");
    lines.take_while(|line| !line.starts_with('#')).collect()
}

/// The body rows of the first table of a section, as trimmed cells.
fn table_rows(text: &str, number: &str) -> Vec<Vec<String>> {
    section(text, number)
        .into_iter()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .skip(2) // header and separator
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect()
}

/// The backticked parts of a cell.
fn code_spans(cell: &str) -> Vec<&str> {
    cell.split('`').skip(1).step_by(2).collect()
}

/// `#RRGGBB`, `#RRGGBBAA` or `rgba(r,g,b,a)`: the first color in a cell.
fn parse_color(cell: &str) -> Option<Token> {
    for span in code_spans(cell) {
        if let Some(token) = color_value(span) {
            return Some(token);
        }
    }
    None
}

fn color_value(text: &str) -> Option<Token> {
    let text = text.trim();
    if let Some(start) = text.find("rgba(") {
        let inner = &text[start + 5..text[start..].find(')')? + start];
        let parts: Vec<f64> = inner
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [r, g, b, a] = parts[..] else {
            return None;
        };
        let rgb = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        return Some(palette::translucent(rgb, (a * 255.).round() as u8));
    }
    let hex = text.strip_prefix('#')?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(opaque(value)),
        8 => Some(palette::translucent(value >> 8, (value & 0xFF) as u8)),
        _ => None,
    }
}

/// `0 8px 24px rgba(...)`.
fn parse_shadow(cell: &str) -> Shadow {
    let span = code_spans(cell)[0];
    let mut parts = span.split_whitespace();
    let x = parts.next().unwrap();
    assert_eq!(x, "0", "shadow x offset");
    let number = |part: &str| part.trim_end_matches("px").parse::<u8>().unwrap();
    let y = number(parts.next().unwrap());
    let blur = number(parts.next().unwrap());
    Shadow {
        y,
        blur,
        color: color_value(span).expect("shadow color"),
    }
}

/// The token name of a first cell: `` `--bg-base` `` -> `bg-base`.
fn token_name(cell: &str) -> &str {
    code_spans(cell)[0].trim_start_matches("--")
}

fn lookup(name: &str, tokens: &InterfaceTokens) -> Token {
    token_by_name(name, tokens).unwrap_or_else(|| panic!("no palette token --{name}"))
}

/// Sections 2.1-2.5: every token row, dark and light.
#[test]
fn interface_tokens_match_the_design_system() {
    for (path, text) in docs() {
        let mut checked = 0;
        for number in ["2.1", "2.2", "2.3", "2.4", "2.5"] {
            for row in table_rows(&text, number) {
                let name = token_name(&row[0]);
                for (cell, tokens, mode) in [(&row[2], &DARK, "dark"), (&row[3], &LIGHT, "light")] {
                    let expected = parse_color(cell)
                        .unwrap_or_else(|| panic!("{path} {number}: no color for --{name}"));
                    assert_eq!(
                        lookup(name, tokens),
                        expected,
                        "{path} {number}: --{name} ({mode})"
                    );
                }
                checked += 1;
            }
        }
        // 10 surfaces, 2 borders, 4 text, 7 accent, 11 semantic.
        assert_eq!(checked, 34, "{path}: token rows");
    }
}

/// Section 2.6: shadows.
#[test]
fn shadows_match_the_design_system() {
    for (path, text) in docs() {
        let rows = table_rows(&text, "2.6");
        assert_eq!(rows.len(), 2, "{path}");
        for row in rows {
            let name = token_name(&row[0]);
            let pick = |tokens: &InterfaceTokens| match name {
                "shadow-overlay" => tokens.shadow_overlay,
                "shadow-modal" => tokens.shadow_modal,
                other => panic!("{path}: unknown shadow --{other}"),
            };
            assert_eq!(pick(&DARK), parse_shadow(&row[1]), "{path}: --{name} dark");
            assert_eq!(
                pick(&LIGHT),
                parse_shadow(&row[2]),
                "{path}: --{name} light"
            );
        }
    }
}

/// Section 3.1: tile colors and `--text-on-tag`, the same in both themes.
#[test]
fn tile_colors_match_the_design_system() {
    for (path, text) in docs() {
        let rows = table_rows(&text, "3.1");
        assert_eq!(rows.len(), 9, "{path}");
        for row in rows {
            let name = token_name(&row[0]);
            let expected = parse_color(&row[1]).expect("color");
            assert_eq!(lookup(name, &DARK), expected, "{path}: --{name}");
            assert_eq!(lookup(name, &LIGHT), expected, "{path}: --{name}");
        }
    }
}

/// Sections 4.1 and 4.2: the built-in terminal themes.
#[test]
fn terminal_themes_match_the_design_system() {
    let themes = terminal::builtin();
    for (path, text) in docs() {
        for (number, theme) in [("4.1", &themes[0]), ("4.2", &themes[1])] {
            let mut roles = 0;
            for row in table_rows(&text, number) {
                for pair in row.chunks(2) {
                    let role = code_spans(&pair[0])[0];
                    let expected = parse_color(&pair[1]);
                    assert_eq!(
                        theme.role(role),
                        expected,
                        "{path} {number}: {role} of {}",
                        theme.name
                    );
                    roles += 1;
                }
            }
            assert_eq!(roles, 24, "{path} {number}: roles");
        }
    }
}

/// Section 4.3: keyword highlighting defaults, in table order.
#[test]
fn keyword_colors_match_the_design_system() {
    for (path, text) in docs() {
        let rows = table_rows(&text, "4.3");
        assert_eq!(rows.len(), 6, "{path}");
        let dark = palette::KEYWORDS_ON_DARK.all();
        let light = palette::KEYWORDS_ON_LIGHT.all();
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                Some(dark[index]),
                parse_color(&row[1]),
                "{path}: {}",
                row[0]
            );
            assert_eq!(
                Some(light[index]),
                parse_color(&row[2]),
                "{path}: {}",
                row[0]
            );
        }
        // Error, Warning, OK and Info reuse the interface tokens.
        assert_eq!(
            dark[..4],
            [DARK.danger, DARK.warning, DARK.success, DARK.info]
        );
        assert_eq!(
            light[..4],
            [LIGHT.danger, LIGHT.warning, LIGHT.success, LIGHT.info]
        );
    }
}

/// The column rule of 4.3 picks dark for Graphite and light for Paper.
#[test]
fn keyword_column_follows_the_background() {
    let themes: Vec<TerminalTheme> = terminal::builtin();
    assert_eq!(
        palette::keyword_colors(themes[0].background),
        &palette::KEYWORDS_ON_DARK
    );
    assert_eq!(
        palette::keyword_colors(themes[1].background),
        &palette::KEYWORDS_ON_LIGHT
    );
}

/// Section 7.4: every listed theme-file key holds the listed token, every
/// listed `ThemeColor` field exists, and every "palette.rs" token is in
/// `palette.rs`.
#[test]
fn theme_color_mapping_matches_section_7_4() {
    let fields = serde_json::to_value(ThemeColor::default()).expect("ThemeColor serializes");
    let fields = fields.as_object().expect("object");
    for (path, text) in docs() {
        let rows = table_rows(&text, "7.4");
        assert!(rows.len() >= 30, "{path}: 7.4 rows");
        for row in rows {
            let tokens: Vec<&str> = code_spans(&row[0])
                .into_iter()
                .map(|token| token.trim_start_matches("--"))
                .collect();
            if row[1].contains("palette.rs") {
                for token in tokens {
                    match token {
                        "shadow-overlay" | "shadow-modal" => {}
                        "tag-*" => {
                            for tag in palette::TAG_COLORS {
                                let _ = palette::tag_color(tag);
                            }
                        }
                        name => {
                            let _ = lookup(name, &DARK);
                            let _ = lookup(name, &LIGHT);
                        }
                    }
                }
                continue;
            }
            assert_eq!(tokens.len(), 1, "{path}: one token per mapped row");
            let token = tokens[0];
            for field in code_spans(&row[1]) {
                assert!(fields.contains_key(field), "{path}: no ThemeColor.{field}");
            }
            for key in code_spans(&row[2]) {
                let mapped = THEME_COLOR_MAPPING
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, token)| *token);
                assert_eq!(mapped, Some(token), "{path}: {key} should be --{token}");
            }
        }
    }
}

/// Section 7.4 against the installed theme: each listed `ThemeColor` field
/// holds its token in both modes.
#[gpui_kit::test]
fn applied_fields_hold_the_section_7_4_tokens(cx: &mut TestAppContext) {
    let text = doc("docs/design-system.md");
    let rows = table_rows(&text, "7.4");
    cx.update(|cx| {
        gpui_kit::init(cx);
        for (appearance, tokens) in [(Appearance::Dark, &DARK), (Appearance::Light, &LIGHT)] {
            install(appearance, cx);
            let applied = serde_json::to_value(cx.theme().colors).expect("serializes");
            for row in &rows {
                if row[1].contains("palette.rs") {
                    continue;
                }
                let token = token_name(&row[0]);
                let expected = lookup(token, tokens).hsla();
                for field in code_spans(&row[1]) {
                    let value: Hsla = serde_json::from_value(applied[field].clone()).expect("Hsla");
                    assert!(
                        close(value, expected),
                        "{appearance:?}: ThemeColor.{field} should be --{token}"
                    );
                }
            }
        }
    });
}

fn close(a: Hsla, b: Hsla) -> bool {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    (a.r - b.r).abs() < 1e-3
        && (a.g - b.g).abs() < 1e-3
        && (a.b - b.b).abs() < 1e-3
        && (a.a - b.a).abs() < 1e-3
}

/// Every "<n> px (`--name`)" and "<n> ms (`--name`)" of sections 5 and 6.
fn dimension_tokens(text: &str) -> Vec<(String, f32)> {
    let mut found = Vec::new();
    let mut body = String::new();
    for number in ["5.", "6."] {
        let heading = format!("## {number} ");
        let mut lines = text.lines().skip_while(|line| !line.starts_with(&heading));
        lines.next();
        for line in lines.take_while(|line| !line.starts_with("## ")) {
            body.push_str(line);
            body.push(' ');
        }
    }
    let mut rest = body.as_str();
    while let Some(start) = rest.find("(`--") {
        let before = rest[..start].trim_end();
        let after = &rest[start + 4..];
        let end = after.find('`').unwrap();
        let name = after[..end].to_owned();
        let number = before
            .trim_end_matches("px")
            .trim_end_matches("ms")
            .trim_end_matches("мс")
            .trim_end()
            .rsplit(' ')
            .next()
            .unwrap_or_default();
        if let Ok(value) = number.parse::<f32>() {
            found.push((name, value));
        }
        rest = &after[end..];
    }
    found
}

/// Sizes, spacing, radii, type sizes, motion and opacity of sections 5 and
/// 6 against `metrics`.
#[test]
fn metrics_match_the_design_system() {
    let ms = |duration: std::time::Duration| duration.as_millis() as f32;
    for (path, text) in docs() {
        let found = dimension_tokens(&text);
        let mut checked = 0;
        for (name, value) in &found {
            let ours: f32 = match name.as_str() {
                "space-1" => metrics::SPACE_1.into(),
                "space-2" => metrics::SPACE_2.into(),
                "space-3" => metrics::SPACE_3.into(),
                "space-4" => metrics::SPACE_4.into(),
                "space-5" => metrics::SPACE_5.into(),
                "space-6" => metrics::SPACE_6.into(),
                "radius-control" => metrics::RADIUS_CONTROL.into(),
                "radius-tile" => metrics::RADIUS_TILE.into(),
                "radius-card" => metrics::RADIUS_CARD.into(),
                "radius-overlay" => metrics::RADIUS_OVERLAY.into(),
                "height-top-bar" => metrics::HEIGHT_TOP_BAR.into(),
                "height-control" => metrics::HEIGHT_CONTROL.into(),
                "height-row" => metrics::HEIGHT_ROW.into(),
                "height-host-card" => metrics::HEIGHT_HOST_CARD.into(),
                "width-sidebar" => metrics::WIDTH_SIDEBAR.into(),
                "width-details" => metrics::WIDTH_DETAILS.into(),
                "width-details-min" => metrics::WIDTH_DETAILS_MIN.into(),
                "width-details-max" => metrics::WIDTH_DETAILS_MAX.into(),
                "font-size-caption" => metrics::FONT_SIZE_CAPTION.into(),
                "font-size-dense" => metrics::FONT_SIZE_DENSE.into(),
                "font-size-body" => metrics::FONT_SIZE_BODY.into(),
                "font-size-heading" => metrics::FONT_SIZE_HEADING.into(),
                "font-size-title" => metrics::FONT_SIZE_TITLE.into(),
                "font-size-mono" => metrics::FONT_SIZE_MONO.into(),
                "focus-ring-width" => metrics::FOCUS_RING_WIDTH.into(),
                "focus-ring-offset" => metrics::FOCUS_RING_OFFSET.into(),
                "size-status-dot" => metrics::SIZE_STATUS_DOT.into(),
                "size-tab-indicator" => metrics::SIZE_TAB_INDICATOR.into(),
                "duration-fast" => ms(metrics::DURATION_FAST),
                "duration-normal" => ms(metrics::DURATION_NORMAL),
                "opacity-disabled" => metrics::OPACITY_DISABLED,
                "line-height-mono" => metrics::LINE_HEIGHT_MONO,
                "font-weight-regular" => metrics::FONT_WEIGHT_REGULAR.0,
                "font-weight-medium" => metrics::FONT_WEIGHT_MEDIUM.0,
                "font-weight-semibold" => metrics::FONT_WEIGHT_SEMIBOLD.0,
                other => panic!("{path}: no metrics constant for --{other}"),
            };
            assert_eq!(ours, *value, "{path}: --{name}");
            checked += 1;
        }
        assert!(checked >= 28, "{path}: only {checked} sizes found");
    }
    assert_eq!(metrics::OPACITY_DISABLED, 0.45);
    assert!((metrics::LINE_HEIGHT_MONO - 1.2).abs() < f32::EPSILON);
    assert_eq!(metrics::FONT_WEIGHT_REGULAR.0, 400.);
    assert_eq!(metrics::FONT_WEIGHT_MEDIUM.0, 500.);
    assert_eq!(metrics::FONT_WEIGHT_SEMIBOLD.0, 600.);
}

/// The kit's base values in graphite.json come from the metrics (7.1):
/// `font.size` = `--font-size-body`, `radius` = `--radius-control`,
/// `radius.lg` = `--radius-overlay`.
#[test]
fn theme_file_base_values_are_the_metrics() {
    let file: serde_json::Value = serde_json::from_str(super::THEME_FILE).unwrap();
    for theme in file["themes"].as_array().unwrap() {
        let number = |key: &str| theme[key].as_f64().unwrap() as f32;
        assert_eq!(number("font.size"), f32::from(metrics::FONT_SIZE_BODY));
        assert_eq!(number("mono_font.size"), f32::from(metrics::FONT_SIZE_MONO));
        assert_eq!(number("radius"), f32::from(metrics::RADIUS_CONTROL));
        assert_eq!(number("radius.lg"), f32::from(metrics::RADIUS_OVERLAY));
        // The kit's own control shadows stay off: the design system has
        // shadows only for overlays and modal windows (2.6, palette.rs).
        assert_eq!(theme["shadow"], false);
    }
}
