//! Test: no color literals outside `src/theme/` (docs/design-system.md 7.1:
//! raw color values only in `themes/graphite.json` and `src/theme/`; views
//! take colors from `cx.theme()`, `theme::tokens(cx)` and `palette`).
//!
//! Flagged in Rust code outside comments: GPUI color constructors (`rgb(`,
//! `rgba(`, `hsla(`, `hsl(`, `Rgba {`, `Hsla {`, `white()`, `black()`,
//! `red()`, `green()`, `blue()`, `yellow()`, `transparent_black()`,
//! `transparent_white()`), palette constructors (`opaque(`, `translucent(`),
//! hex color strings (`"#RRGGBB"`) and six or eight digit hex numbers
//! (`0xRRGGBB`). Method calls such as `token.hsla()` are fine.

use std::path::{Path, PathBuf};

const CALLS: &[&str] = &[
    "rgb(",
    "rgba(",
    "hsla(",
    "hsl(",
    "white()",
    "black()",
    "red()",
    "green()",
    "blue()",
    "yellow()",
    "transparent_black()",
    "transparent_white()",
    "opaque(",
    "translucent(",
];
const STRUCTS: &[&str] = &["Rgba {", "Hsla {"];

fn rust_files(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src") {
        let path = entry.expect("entry").path();
        if path.starts_with(skip) {
            continue;
        }
        if path.is_dir() {
            rust_files(&path, skip, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The code part of a line (without a `//` comment).
fn code(line: &str) -> &str {
    let mut in_string = false;
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if in_string => index += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(index + 1) == Some(&b'/') => return &line[..index],
            _ => {}
        }
        index += 1;
    }
    line
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The color literals of one line of code.
pub(super) fn findings(line: &str) -> Vec<String> {
    let code = code(line);
    let bytes = code.as_bytes();
    let mut found = Vec::new();
    for call in CALLS {
        let mut from = 0;
        while let Some(at) = code[from..].find(call) {
            let start = from + at;
            let before = start.checked_sub(1).map(|index| bytes[index]);
            // `x.hsla()` and `my_rgb(` are not color literals.
            if !before.is_some_and(|byte| is_ident(byte) || byte == b'.') {
                found.push((*call).to_owned());
            }
            from = start + call.len();
        }
    }
    for item in STRUCTS {
        if let Some(at) = code.find(item)
            && !at
                .checked_sub(1)
                .is_some_and(|index| is_ident(bytes[index]))
        {
            found.push((*item).to_owned());
        }
    }
    // "#RRGGBB" / "#RRGGBBAA" (also 3 digits) inside a string.
    let mut from = 0;
    while let Some(at) = code[from..].find("\"#") {
        let start = from + at + 2;
        let digits = bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if matches!(digits, 3 | 6 | 8) && bytes.get(start + digits) == Some(&b'"') {
            found.push(code[start - 1..start + digits].to_owned());
        }
        from = start;
    }
    // 0xRRGGBB / 0xRRGGBBAA.
    let mut from = 0;
    while let Some(at) = code[from..].find("0x") {
        let start = from + at;
        let before = start.checked_sub(1).map(|index| bytes[index]);
        let digits = bytes[start + 2..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit() || **byte == b'_')
            .filter(|byte| **byte != b'_')
            .count();
        if !before.is_some_and(is_ident) && matches!(digits, 6 | 8) {
            found.push(code[start..start + 2 + digits].to_owned());
        }
        from = start + 2;
    }
    found
}

#[test]
fn the_scanner_finds_color_literals() {
    assert!(!findings("div().bg(gpui::white())").is_empty());
    assert!(!findings("let c = rgb(0x2EC4A6);").is_empty());
    assert!(!findings("hsla(0., 0., 1., 1.)").is_empty());
    assert!(!findings("let hex = \"#2EC4A6\";").is_empty());
    assert!(!findings("Hsla { h: 0., s: 0., l: 0., a: 1. }").is_empty());
    assert!(findings("palette::TEXT_ON_TAG.hsla()").is_empty());
    assert!(findings("// rgb(0x2EC4A6) in a comment").is_empty());
    assert!(findings("let url = \"https://example.com\"; // white()").is_empty());
    assert!(findings("let mask = 0xFF;").is_empty());
}

#[test]
fn views_have_no_color_literals() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let theme = src.join("theme");
    let mut files = Vec::new();
    rust_files(&src, &theme, &mut files);
    assert!(files.len() > 10, "source files not found");
    let mut violations = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for (number, line) in text.lines().enumerate() {
            for literal in findings(line) {
                let relative = file.strip_prefix(&src).unwrap_or(&file);
                violations.push(format!(
                    "src/{}:{}: {literal}",
                    relative.display(),
                    number + 1
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "color literals outside src/theme (use cx.theme(), theme::tokens or palette):\n{}",
        violations.join("\n")
    );
}
