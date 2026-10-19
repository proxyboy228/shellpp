//! Injected-input tracking for history capture (docs/security.md, section
//! 10.1; stage 2 plan, decision 4).
//!
//! The session manager sees every `session_write` (typed input, pastes) and
//! every `session_inject` (input the app sends on its own: a history insert,
//! later snippets). A command line that contains injected input is never
//! recorded, so the tracker remembers whether the current line (since the
//! last line break) received injected input. Fail-closed: only a typed CR
//! (Enter; pastes reach the session with CR line breaks) starts a clean
//! line; editing keys, Ctrl+C and LF do not. LF (Ctrl+J) also runs the line
//! in most shells, but the webview takes candidates only at Enter, so a
//! candidate that follows an LF is dropped as injected rather than recorded
//! with text the app inserted.

/// Whether a typed byte starts a clean command line: CR (Enter) only.
fn ends_typed_line(byte: u8) -> bool {
    byte == b'\r'
}

/// Whether a byte of injected input ends a command line (a snippet may use
/// CR or LF to run its lines).
fn is_line_break(byte: u8) -> bool {
    byte == b'\r' || byte == b'\n'
}

/// Line state of one session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineTracker {
    injected: bool,
}

impl LineTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the current line contains injected input.
    pub fn current_injected(&self) -> bool {
        self.injected
    }

    /// Typed input. When `data` contains a CR, returns whether the line that
    /// the first CR ends contained injected input; the text after it starts
    /// a new, clean line.
    pub fn typed(&mut self, data: &[u8]) -> Option<bool> {
        if !data.iter().copied().any(ends_typed_line) {
            return None;
        }
        let ended = self.injected;
        self.injected = false;
        Some(ended)
    }

    /// Input sent by the app. The current line now contains injected input;
    /// when `data` ends lines, the text after its last line break does.
    pub fn injected(&mut self, data: &[u8]) {
        match data.iter().rposition(|byte| is_line_break(*byte)) {
            Some(last) => self.injected = last + 1 < data.len(),
            None => {
                if !data.is_empty() {
                    self.injected = true;
                }
            }
        }
    }

    /// A new connection starts with a clean line.
    pub fn reset(&mut self) {
        self.injected = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_lines_are_clean() {
        let mut lines = LineTracker::new();
        assert_eq!(lines.typed(b"ls -la"), None);
        assert_eq!(lines.typed(b"\r"), Some(false));
        assert!(!lines.current_injected());
    }

    #[test]
    fn injected_text_marks_the_line_until_enter() {
        let mut lines = LineTracker::new();
        lines.injected(b"git status");
        assert!(lines.current_injected());
        // Editing does not clear the mark (fail-closed).
        assert_eq!(lines.typed(b"\x7f\x7f\x03"), None);
        assert!(lines.current_injected());
        assert_eq!(lines.typed(b" --short\r"), Some(true));
        // The next line is clean.
        assert_eq!(lines.typed(b"pwd\r"), Some(false));
    }

    #[test]
    fn typed_enter_with_more_text_starts_a_clean_line() {
        let mut lines = LineTracker::new();
        lines.injected(b"echo a");
        assert_eq!(lines.typed(b"\recho b"), Some(true));
        assert!(!lines.current_injected());
        assert_eq!(lines.typed(b"\r"), Some(false));
    }

    /// Ctrl+J (LF) typed after an inserted text keeps the line marked: a
    /// stale candidate taken later is dropped (fail-closed).
    #[test]
    fn a_typed_line_feed_does_not_clean_the_line() {
        let mut lines = LineTracker::new();
        assert_eq!(lines.typed(b"a"), None);
        lines.injected(b"X");
        assert_eq!(lines.typed(b"\n"), None);
        assert!(lines.current_injected());
        assert_eq!(lines.typed(b"ls\r"), Some(true));
        assert_eq!(lines.typed(b"pwd\r"), Some(false));
    }

    #[test]
    fn injected_line_breaks() {
        let mut lines = LineTracker::new();
        lines.injected(b"run\r");
        assert!(!lines.current_injected());
        lines.injected(b"one\rtwo");
        assert!(lines.current_injected());
        lines.injected(b"");
        assert!(lines.current_injected());
        lines.reset();
        assert!(!lines.current_injected());
        lines.injected(b"x\n");
        assert!(!lines.current_injected());
    }
}
