//! Strings of the terminal sidebar (UI 4.2), catalog `en`.

// The icon tabs.
pub const TOOLS: &str = "Tools";
pub const SNIPPETS: &str = "Snippets";
pub const HISTORY: &str = "History";
pub const APPEARANCE: &str = "Appearance";

// Tools: search in the terminal output.
pub const SEARCH: &str = "Search in output";
pub const SEARCH_PLACEHOLDER: &str = "Find text";
pub const SEARCH_HINT: &str = "Searches the output and the scrollback. Case does not matter.";
pub const SEARCH_NO_SESSION: &str = "Open a session in this pane to search its output.";
pub const NO_MATCHES: &str = "No matches";
pub const PREVIOUS_MATCH: &str = "Previous match (Shift+Enter)";
pub const NEXT_MATCH: &str = "Next match (Enter)";

/// "3 of 12", or "3 of 10000+" when the search stopped counting.
pub fn match_counter(current: Option<usize>, count: usize, truncated: bool) -> String {
    let more = if truncated { "+" } else { "" };
    match current {
        Some(current) => format!("{current} of {count}{more}"),
        None => format!("{count}{more} matches"),
    }
}

// Snippets (stage 5).
pub const SNIPPETS_LATER: &str = "Snippets will be available in stage 5.";

// Appearance.
pub const FONT: &str = "Font";
pub const FONT_FAMILY: &str = "Family";
pub const FONT_SIZE: &str = crate::settings::strings::FONT_SIZE;
pub const LINE_HEIGHT: &str = crate::settings::strings::LINE_HEIGHT;
pub const FONT_SEARCH_PLACEHOLDER: &str = crate::settings::strings::FONT_SEARCH_PLACEHOLDER;
pub const FONT_BUNDLED: &str = crate::settings::strings::FONT_BUNDLED;
pub const FONTS_LOADING: &str = crate::settings::strings::FONTS_LOADING;
pub const FONT_SCOPE: &str = "Applies to all terminals.";
pub const BACK: &str = "Back";
pub const THEMES: &str = "Themes";
pub const THEME_SAVED_FOR_HOST: &str =
    "A theme you pick is used for this session and saved for this host.";
pub const THEME_SESSION_ONLY: &str = "A theme you pick is used for this session only.";
pub const THEME_NO_SESSION: &str = "Open a session in this pane to change its theme.";
pub const THEME_NOT_SAVED: &str =
    "The theme is used for this session, but it was not saved for the host.";
pub const SETTING_NOT_SAVED: &str = crate::settings::strings::FAILED;

/// "JetBrains Mono, 14 px, spacing 1.2" under the Font row.
pub fn font_summary(family: &str, size: u32, line_height: &str) -> String {
    format!("{family}, {size} px, spacing {line_height}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_read_naturally() {
        assert_eq!(match_counter(Some(3), 12, false), "3 of 12");
        assert_eq!(match_counter(Some(1), 10_000, true), "1 of 10000+");
        assert_eq!(match_counter(None, 4, false), "4 matches");
        assert_eq!(
            font_summary("JetBrains Mono", 14, "1.2"),
            "JetBrains Mono, 14 px, spacing 1.2"
        );
    }
}
