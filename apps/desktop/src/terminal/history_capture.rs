//! Command capture for the history (`shellpp-terminal`, run 13): the
//! model's part lives in the crate; the view's insertions of a History
//! entry or a snippet are here.

use alacritty_terminal::term::TermMode;
use gpui::Context;
pub use shellpp_terminal::history_capture::*;

use super::paste;
use super::view::TerminalView;

impl TerminalView {
    /// Inserts `text` at the prompt without running it (UI 4.2, a History
    /// entry): as injected input, on one line (line breaks become spaces),
    /// bracketed when the program asked for bracketed paste.
    pub fn insert_injected(&mut self, text: &str, cx: &mut Context<Self>) {
        let one_line: String = paste::sanitize(text)
            .chars()
            .map(|c| if matches!(c, '\r' | '\n') { ' ' } else { c })
            .collect();
        if one_line.is_empty() {
            return;
        }
        let handle = self.handle();
        handle.scroll_to_bottom();
        let bracketed = handle
            .term()
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        handle.inject_input(paste::paste_bytes(&one_line, bracketed));
        cx.notify();
    }

    /// Whether [`Self::insert_snippet`] takes `script` without Enter now: a
    /// script of several lines needs the shell's bracketed paste (D68: the
    /// sidebar shows its insert as unavailable otherwise).
    pub fn can_insert(&self, script: &str) -> bool {
        let text = paste::sanitize(script).replace("\r\n", "\n");
        let text = text.trim_end_matches(['\n', '\r']);
        !paste::has_line_break(text)
            || self
                .handle()
                .term()
                .lock()
                .mode()
                .contains(TermMode::BRACKETED_PASTE)
    }

    /// A snippet from the terminal sidebar (UI 4.2), as injected input:
    /// `run` sends Enter after it. A script of several lines goes as one
    /// bracketed paste when the shell takes one, so nothing runs before
    /// Enter; without bracketed paste each line would run as it arrives, so
    /// an insert of such a script is refused (`false`).
    pub fn insert_snippet(&mut self, script: &str, run: bool, cx: &mut Context<Self>) -> bool {
        let text = paste::sanitize(script).replace("\r\n", "\n");
        let text = text.trim_end_matches(['\n', '\r']);
        if text.trim().is_empty() {
            return false;
        }
        let handle = self.handle();
        handle.scroll_to_bottom();
        let bracketed = handle
            .term()
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        if !run && !bracketed && paste::has_line_break(text) {
            return false;
        }
        let mut bytes = paste::paste_bytes(text, bracketed);
        if run {
            bytes.push(b'\r');
        }
        handle.inject_input(bytes);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests;
