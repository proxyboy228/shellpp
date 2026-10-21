//! Keyword highlighting (UI 12.4; stage 6, D67): words of the categories
//! Error, Warning, OK, Info and Debug, and IP and MAC addresses, are drawn in
//! their category's color. It changes only the display: the text, the
//! copy and the search are untouched. Only cells in the terminal's default
//! text color are recolored (a program's own colors stay), never in the
//! alternate screen (vim, htop), and only when a row is laid out anew (the
//! row cache keeps the result).

use std::ops::Range;
use std::sync::LazyLock;

use alacritty_terminal::vte::ansi::Rgb;
use regex::Regex;

use crate::theme::highlight::{self as colors, CategoryColors};

/// A highlighting category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Error,
    Warning,
    Ok,
    Info,
    Debug,
    Address,
}

impl Category {
    const ALL: [Category; 6] = [
        Category::Error,
        Category::Warning,
        Category::Ok,
        Category::Info,
        Category::Debug,
        Category::Address,
    ];

    fn group(self) -> &'static str {
        match self {
            Category::Error => "error",
            Category::Warning => "warning",
            Category::Ok => "ok",
            Category::Info => "info",
            Category::Debug => "debug",
            Category::Address => "address",
        }
    }

    fn colors(self) -> CategoryColors {
        match self {
            Category::Error => colors::ERROR,
            Category::Warning => colors::WARNING,
            Category::Ok => colors::OK,
            Category::Info => colors::INFO,
            Category::Debug => colors::DEBUG,
            Category::Address => colors::ADDRESS,
        }
    }

    /// The color on a dark (`true`) or light terminal background.
    pub fn color(self, dark: bool) -> Rgb {
        let (r, g, b) = self.colors()[usize::from(!dark)];
        Rgb { r, g, b }
    }
}

/// One regex with a named group per category, case-insensitive, on ASCII
/// word boundaries and ASCII digits (the fast matcher stays on rows with
/// other scripts).
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)",
        r"(?P<error>(?-u:\b)(?:error|errors|err|fail|failed|failure|fatal|denied|refused|panic|critical|exception)(?-u:\b))",
        r"|(?P<warning>(?-u:\b)(?:warn|warning|warnings|deprecated)(?-u:\b))",
        r"|(?P<ok>(?-u:\b)(?:ok|success|successful|succeeded|passed)(?-u:\b))",
        r"|(?P<info>(?-u:\b)(?:info|notice)(?-u:\b))",
        r"|(?P<debug>(?-u:\b)(?:debug|trace)(?-u:\b))",
        r"|(?P<address>(?-u:\b)(?:(?:25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1?[0-9]?[0-9])(?-u:\b)",
        r"|(?-u:\b)(?:[0-9a-f]{2}[:-]){5}[0-9a-f]{2}(?-u:\b))",
    ))
    .expect("the highlighting pattern")
});

/// An address inside a longer run of the same kind (a hex dump, a
/// fingerprint, an OID, a four-part version): not an address.
fn inside_longer_run(text: &str, start: usize, end: usize) -> bool {
    let joins = |c: char| matches!(c, '.' | ':' | '-');
    let before = text[..start].chars().next_back();
    let mut after = text[end..].chars();
    let (after, beyond) = (after.next(), after.next());
    before.is_some_and(joins)
        || (after.is_some_and(joins) && beyond.is_some_and(|c| c.is_ascii_hexdigit()))
}

/// The highlighted spans of `text`, as character ranges with their
/// category.
pub fn spans(text: &str) -> Vec<(Range<usize>, Category)> {
    let mut out = Vec::new();
    // Character positions counted as the matches go (one pass).
    let (mut byte, mut chars) = (0, 0);
    for captures in PATTERN.captures_iter(text) {
        let Some(category) = Category::ALL
            .iter()
            .copied()
            .find(|category| captures.name(category.group()).is_some())
        else {
            continue;
        };
        let Some(found) = captures.get(0) else {
            continue;
        };
        if category == Category::Address && inside_longer_run(text, found.start(), found.end()) {
            continue;
        }
        chars += text[byte..found.start()].chars().count();
        byte = found.start();
        let len = found.as_str().chars().count();
        out.push((chars..chars + len, category));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(&str, Category)> {
        let chars: Vec<char> = text.chars().collect();
        spans(text)
            .into_iter()
            .map(|(range, category)| {
                let word: String = chars[range].iter().collect();
                (Box::leak(word.into_boxed_str()) as &str, category)
            })
            .collect()
    }

    #[test]
    fn categories_are_found_on_word_boundaries() {
        assert_eq!(
            found("ERROR: disk failed, Warning low, ok"),
            [
                ("ERROR", Category::Error),
                ("failed", Category::Error),
                ("Warning", Category::Warning),
                ("ok", Category::Ok),
            ]
        );
        // Not inside other words: "errors" is one, "terror" and "booking"
        // are not.
        assert_eq!(found("terror booking"), []);
        // Not inside hex dumps, fingerprints, OIDs or versions.
        assert_eq!(
            found("00:b4:31:98:0a:c4:bc:11:22:33 1.3.6.1.4.1 4.0.0.0.1"),
            []
        );
        assert_eq!(
            found("from 192.168.1.20 via aa:bb:cc:00:11:22, not 999.1.1.1"),
            [
                ("192.168.1.20", Category::Address),
                ("aa:bb:cc:00:11:22", Category::Address),
            ]
        );
        // Character positions, not bytes, across several matches.
        assert_eq!(spans("ü error")[0].0, 2..7);
        let both = spans("ü error ü ok");
        assert_eq!((both[0].0.clone(), both[1].0.clone()), (2..7, 10..12));
        assert_ne!(Category::Error.color(true), Category::Error.color(false));
    }
}
