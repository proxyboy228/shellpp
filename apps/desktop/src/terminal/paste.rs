//! Pasting into the terminal (UI 4.3, SPEC "Terminal", docs/security.md
//! sections 9.4 and 12).
//!
//! - The text is cleaned first: the bracketed paste markers `ESC[200~` and
//!   `ESC[201~`, then ESC and every other C0/C1 control character except
//!   tab, CR and LF are removed, so pasted text can neither end bracketed
//!   paste mode early nor carry escape sequences.
//! - Text with a line break needs confirmation (the setting
//!   `terminal.confirm_multiline_paste`, on by default): each line break
//!   runs a command in a shell. The dialog shows the number of lines and a
//!   preview.
//! - Line breaks are sent as CR (what Enter sends). When the remote
//!   application enabled bracketed paste (mode 2004), the text is wrapped in
//!   `ESC[200~` ... `ESC[201~`.

/// The bracketed paste markers.
pub const BRACKET_START: &str = "\x1b[200~";
pub const BRACKET_END: &str = "\x1b[201~";

/// Lines shown in the confirmation preview.
pub const PREVIEW_LINES: usize = 5;
/// Characters shown per preview line.
pub const PREVIEW_LINE_CHARS: usize = 100;

/// Removes the bracketed paste markers and control characters except tab,
/// CR and LF.
///
/// Removing one marker can join the parts of another
/// (`"\x1b[20\x1b[201~0~"`), so markers are removed as they complete, in one
/// pass with stack semantics: each character is appended, and a marker that
/// now ends the output is cut off. The output never holds a marker, and
/// since no marker overlaps another, the result is the same as removing
/// markers until none is left, in linear time (a crafted clipboard cannot
/// stall the UI thread).
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        out.push(ch);
        if out.ends_with(BRACKET_START) || out.ends_with(BRACKET_END) {
            out.truncate(out.len() - BRACKET_START.len());
        }
    }
    out.retain(|ch| matches!(ch, '\t' | '\r' | '\n') || !ch.is_control());
    out
}

/// Whether the text contains a line break (CR or LF).
pub fn has_line_break(text: &str) -> bool {
    text.contains(['\n', '\r'])
}

/// The lines of the text as the user sees them: CRLF, CR and LF end a
/// line; a final line break does not start another line.
pub fn lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['\r', '\n']) {
            Some(at) => {
                lines.push(&rest[..at]);
                let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[at + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

/// Whether pasting `text` (already cleaned) asks first: it has a line break
/// and the setting is on.
pub fn needs_confirmation(text: &str, confirm_setting: bool) -> bool {
    confirm_setting && has_line_break(text)
}

/// What the confirmation dialog shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastePreview {
    /// Number of lines (see [`lines`]).
    pub line_count: usize,
    /// The first [`PREVIEW_LINES`] lines, each cut to
    /// [`PREVIEW_LINE_CHARS`] characters (with "…" when cut).
    pub lines: Vec<String>,
    /// Lines not shown.
    pub more: usize,
}

impl PastePreview {
    pub fn new(text: &str) -> Self {
        let all = lines(text);
        let shown = all
            .iter()
            .take(PREVIEW_LINES)
            .map(|line| {
                // Tabs would break the preview's alignment.
                let line = line.replace('\t', "    ");
                if line.chars().count() > PREVIEW_LINE_CHARS {
                    let mut cut: String = line.chars().take(PREVIEW_LINE_CHARS).collect();
                    cut.push('…');
                    cut
                } else {
                    line
                }
            })
            .collect::<Vec<_>>();
        Self {
            line_count: all.len(),
            more: all.len().saturating_sub(shown.len()),
            lines: shown,
        }
    }
}

/// The bytes to send for cleaned text: line breaks as CR, wrapped in the
/// bracketed paste markers when the application asked for them.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut bytes =
            Vec::with_capacity(normalized.len() + BRACKET_START.len() + BRACKET_END.len());
        bytes.extend_from_slice(BRACKET_START.as_bytes());
        bytes.extend_from_slice(normalized.as_bytes());
        bytes.extend_from_slice(BRACKET_END.as_bytes());
        bytes
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_lines_paste_without_confirmation() {
        assert!(!needs_confirmation("ls -la", true));
        assert!(!needs_confirmation("tab\tseparated", true));
        assert!(!needs_confirmation("", true));
    }

    #[test]
    fn any_line_break_needs_confirmation_when_the_setting_is_on() {
        // Even one trailing line break runs the command.
        assert!(needs_confirmation("rm -rf build\n", true));
        assert!(needs_confirmation("one\r\ntwo", true));
        assert!(needs_confirmation("one\rtwo", true));
        assert!(!needs_confirmation("one\ntwo", false));
    }

    #[test]
    fn line_count_and_preview() {
        assert_eq!(lines("a\nb\n"), ["a", "b"]);
        assert_eq!(lines("a\r\nb\rc"), ["a", "b", "c"]);
        assert_eq!(lines("a\n\nb"), ["a", "", "b"]);
        assert!(lines("").is_empty());

        let text: String = (1..=8).map(|n| format!("line {n}\n")).collect();
        let preview = PastePreview::new(&text);
        assert_eq!(preview.line_count, 8);
        assert_eq!(preview.lines.len(), PREVIEW_LINES);
        assert_eq!(preview.lines[0], "line 1");
        assert_eq!(preview.more, 3);

        let long = "x".repeat(PREVIEW_LINE_CHARS + 10);
        let preview = PastePreview::new(&format!("{long}\nshort"));
        assert_eq!(preview.lines[0].chars().count(), PREVIEW_LINE_CHARS + 1);
        assert!(preview.lines[0].ends_with('…'));
        assert_eq!(preview.more, 0);
    }

    #[test]
    fn bracketed_paste_wraps_and_converts_line_breaks() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("a\nb", true), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(paste_bytes("plain", true), b"\x1b[200~plain\x1b[201~");
    }

    #[test]
    fn markers_and_control_characters_are_removed() {
        // An attempt to end bracketed paste early and run a command.
        let attack = "echo safe\x1b[201~\nrm -rf ~\n";
        let cleaned = sanitize(attack);
        assert_eq!(cleaned, "echo safe\nrm -rf ~\n");
        assert_eq!(
            paste_bytes(&cleaned, true),
            b"\x1b[200~echo safe\rrm -rf ~\r\x1b[201~"
        );
        // A marker split by another marker.
        assert_eq!(sanitize("a\x1b[20\x1b[201~0~b"), "ab");
        assert_eq!(sanitize("x\x1b[200~y"), "xy");
        // ESC, BEL, backspace, DEL and C1 controls (CSI 0x9b) go; tab, CR,
        // LF and ordinary text stay.
        assert_eq!(
            sanitize("a\x1b]0;t\x07b\x08c\x7fd\u{9b}31me\tf\r\ng"),
            "a]0;tbcd31me\tf\r\ng"
        );
        assert_eq!(sanitize("привет 你好"), "привет 你好");
    }

    /// Markers nested inside each other many times over: removed in one
    /// linear pass (repeated replacement would take one pass per level).
    #[test]
    fn deeply_nested_markers_are_removed_in_linear_time() {
        let nested = |depth: usize| {
            format!(
                "a{}{BRACKET_END}{}b",
                "\x1b[20".repeat(depth),
                "0~".repeat(depth)
            )
        };
        assert_eq!(sanitize(&nested(3)), "ab");
        let started = std::time::Instant::now();
        assert_eq!(sanitize(&nested(200_000)), "ab");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "took {:?}",
            started.elapsed()
        );
        // The same result as removing markers until none is left.
        let repeated = |text: &str| {
            let mut text = text.to_owned();
            loop {
                let cleaned = text.replace(BRACKET_START, "").replace(BRACKET_END, "");
                if cleaned == text {
                    return text;
                }
                text = cleaned;
            }
        };
        for sample in [
            "x\x1b[20\x1b[200~1~y",
            "\x1b[2\x1b[201~00~",
            "\x1b[200\x1b[200~~",
            "\x1b\x1b[200~[201~z",
            "\x1b[201~\x1b[200~",
        ] {
            let expected: String = repeated(sample)
                .chars()
                .filter(|ch| !ch.is_control())
                .collect();
            assert_eq!(sanitize(sample), expected, "{sample:?}");
        }
    }
}
