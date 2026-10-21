//! Keyword highlighting (UI 12.4; stage 6, D67): words of the categories
//! Error, Warning, OK, Info and Debug, and IP and MAC addresses, are drawn in
//! their category's color. It changes only the display: the text, the
//! copy and the search are untouched. Only cells in the terminal's default
//! text color are recolored (a program's own colors stay), never in the
//! alternate screen (vim, htop), and only when a row is laid out anew (the
//! row cache keeps the result).
//!
//! The user's rules (D77; data-model 14.2): custom regular expressions
//! with their colors, applied first in creation order, and each built-in
//! category's colors and on/off. Where matches overlap, the earlier rule
//! wins. They are one process-wide set ([`install`]); a new set has a new
//! generation, which is part of the row cache's key.

use std::ops::Range;
#[cfg(not(test))]
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

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

    /// The category's name in a `highlight_rule` record.
    pub fn record_name(self) -> &'static str {
        match self {
            Category::Address => "ip_mac",
            other => other.group(),
        }
    }

    fn index(self) -> usize {
        Category::ALL
            .iter()
            .position(|category| *category == self)
            .unwrap_or(0)
    }
}

/// `#RRGGBB` as a color.
pub fn parse_color(text: &str) -> Option<Rgb> {
    if !shellpp_core::model::highlight_rule::is_color(text) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).ok();
    Some(Rgb {
        r: byte(1)?,
        g: byte(3)?,
        b: byte(5)?,
    })
}

/// A custom rule ready to match.
struct Custom {
    regex: Regex,
    colors: [Rgb; 2],
}

/// Everything highlighting uses: the custom rules, then each category's
/// on/off and colors (`[dark, light]`).
pub struct Rules {
    custom: Vec<Custom>,
    categories: [(bool, [Rgb; 2]); 6],
    generation: u64,
}

impl std::fmt::Debug for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rules")
            .field("custom", &self.custom.len())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

static GENERATION: AtomicU64 = AtomicU64::new(0);
#[cfg(not(test))]
static RULES: LazyLock<RwLock<Arc<Rules>>> =
    LazyLock::new(|| RwLock::new(Arc::new(Rules::builtin())));
// Tests run side by side: each thread has its own rules.
#[cfg(test)]
thread_local! {
    static RULES: std::cell::RefCell<Arc<Rules>> =
        std::cell::RefCell::new(Arc::new(Rules::builtin()));
}

impl Rules {
    /// The built-in categories in their colors, no custom rule.
    pub fn builtin() -> Self {
        Self {
            custom: Vec::new(),
            categories: Category::ALL
                .map(|category| (true, [category.color(true), category.color(false)])),
            generation: 0,
        }
    }

    /// The rules of the vault's `highlight_rule` records (in creation
    /// order; of two records of one category the newer counts). A pattern
    /// that does not compile any more is left out.
    pub fn from_views(views: &[shellpp_core::service::snippets::HighlightRuleView]) -> Self {
        let mut rules = Self::builtin();
        for view in views {
            let Some(dark) = parse_color(&view.color_dark) else {
                continue;
            };
            let light = view
                .color_light
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(dark);
            match view.kind.as_str() {
                "custom" if view.enabled => {
                    let Some(pattern) = &view.pattern else {
                        continue;
                    };
                    if let Ok(regex) =
                        shellpp_core::model::highlight_rule::compile(pattern, view.case_insensitive)
                    {
                        rules.custom.push(Custom {
                            regex,
                            colors: [dark, light],
                        });
                    }
                }
                "category" => {
                    if let Some(category) = Category::ALL
                        .iter()
                        .find(|category| Some(category.record_name()) == view.category.as_deref())
                    {
                        rules.categories[category.index()] = (view.enabled, [dark, light]);
                    }
                }
                _ => {}
            }
        }
        rules
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn custom_count(&self) -> usize {
        self.custom.len()
    }
}

/// The rules in use.
pub fn current() -> Arc<Rules> {
    #[cfg(not(test))]
    {
        RULES
            .read()
            .map(|rules| rules.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
    #[cfg(test)]
    {
        RULES.with(|rules| rules.borrow().clone())
    }
}

/// Puts `rules` in use, with a new generation (rows are laid out again).
pub fn install(mut rules: Rules) {
    rules.generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let rules = Arc::new(rules);
    #[cfg(not(test))]
    match RULES.write() {
        Ok(mut current) => *current = rules,
        Err(poisoned) => *poisoned.into_inner() = rules,
    }
    #[cfg(test)]
    RULES.with(|current| *current.borrow_mut() = rules);
}

/// Reads the vault's rules and puts them in use; the windows draw again.
pub fn load(state: &gpui::Entity<crate::state::AppState>, cx: &mut gpui::App) {
    let views = {
        let state = state.read(cx);
        state
            .core()
            .filter(|_| state.is_unlocked())
            .zip(state.vault_id())
            .and_then(|(core, vault)| core.highlight_rules_list(vault).ok())
            .unwrap_or_default()
    };
    install(Rules::from_views(&views));
    cx.refresh_windows();
}

/// The colored spans of `text` under `rules`, as character ranges: the
/// custom rules first, then the enabled categories, an overlap left to the
/// earlier one.
pub fn colored_spans(text: &str, rules: &Rules, dark: bool) -> Vec<(Range<usize>, Rgb)> {
    let shade = usize::from(!dark);
    let mut out: Vec<(Range<usize>, Rgb)> = Vec::new();
    let free = |out: &Vec<(Range<usize>, Rgb)>, range: &Range<usize>| {
        out.iter()
            .all(|(taken, _)| taken.end <= range.start || range.end <= taken.start)
    };
    if !rules.custom.is_empty() {
        // Character position of each byte offset (once per row).
        let mut chars_at = vec![0usize; text.len() + 1];
        let mut count = 0;
        for (byte, c) in text.char_indices() {
            chars_at[byte] = count;
            count += 1;
            for slot in &mut chars_at[byte + 1..byte + c.len_utf8()] {
                *slot = count;
            }
        }
        chars_at[text.len()] = count;
        for custom in &rules.custom {
            for found in custom.regex.find_iter(text) {
                if found.is_empty() {
                    continue;
                }
                let range = chars_at[found.start()]..chars_at[found.end()];
                if free(&out, &range) {
                    out.push((range, custom.colors[shade]));
                }
            }
        }
    }
    for (range, category) in spans(text) {
        let (enabled, colors) = rules.categories[category.index()];
        if enabled && free(&out, &range) {
            out.push((range, colors[shade]));
        }
    }
    out.sort_by_key(|(range, _)| range.start);
    out
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

    /// D77: custom rules first, in order, then the categories that are on,
    /// in their own colors; an overlap stays with the earlier rule.
    #[test]
    fn custom_rules_and_category_colors() {
        use shellpp_core::service::dto::RecordRef;
        use shellpp_core::service::snippets::HighlightRuleView;
        let view = |kind: &str,
                    category: Option<&str>,
                    pattern: Option<&str>,
                    dark: &str,
                    enabled: bool| {
            HighlightRuleView {
                record: RecordRef {
                    vault_id: uuid::Uuid::nil(),
                    record_id: uuid::Uuid::now_v7(),
                },
                kind: kind.into(),
                category: category.map(str::to_owned),
                pattern: pattern.map(str::to_owned),
                case_insensitive: true,
                color_dark: dark.into(),
                color_light: None,
                enabled,
            }
        };
        let rules = Rules::from_views(&[
            view("custom", None, Some("time\\s?out"), "#112233", true),
            view("custom", None, Some("disk ERROR"), "#445566", true),
            view("custom", None, Some("never"), "#778899", false),
            view("category", Some("warning"), None, "#ABCDEF", true),
            view("category", Some("ok"), None, "#000000", false),
        ]);
        assert_eq!(rules.custom_count(), 2);
        let text = "ü Timeout: disk error, warning ok";
        let found: Vec<(String, Rgb)> = colored_spans(text, &rules, true)
            .into_iter()
            .map(|(range, color)| {
                (
                    text.chars().skip(range.start).take(range.len()).collect(),
                    color,
                )
            })
            .collect();
        let rgb = |text: &str| parse_color(text).unwrap();
        assert_eq!(
            found,
            [
                ("Timeout".to_owned(), rgb("#112233")),
                // The custom rule takes "disk error"; the Error category
                // does not color its "error" again.
                ("disk error".to_owned(), rgb("#445566")),
                ("warning".to_owned(), rgb("#ABCDEF")),
            ]
        );
        assert_eq!(parse_color("#zzzzzz"), None);
        assert_eq!(
            colored_spans("error", &Rules::builtin(), false)[0].1,
            Category::Error.color(false)
        );
    }
}
