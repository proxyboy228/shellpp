//! The terminal emulator view (decision 30; UI 4.1, 4.3, 12.4, 12.5;
//! design-system 4.1-4.2 and 5): alacritty_terminal for parsing, the grid
//! and the scrollback, drawn by our own GPUI element. No code from GPL
//! terminal crates is used.
//!
//! For the session layer (SSH or local PTY):
//!
//! ```ignore
//! // On the UI thread: the view, shown as a child of the tab.
//! let view = cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx));
//! let handle = view.read(cx).handle();
//! // Where input and size changes go (non-blocking; called on the UI thread
//! // and on the parser thread). It gets the current size at once.
//! handle.set_sink(Some(Arc::new(MySink { .. })));
//! // From the session's task or thread: output, with flow control.
//! handle.feed(&bytes).await?;          // async, any runtime
//! handle.feed_blocking(&bytes)?;       // a plain thread (PTY reader)
//! // Title changes, the bell and paste confirmations.
//! cx.subscribe(&view, |this, view, event: &TerminalEvent, cx| { .. });
//! ```
//!
//! - [`model`]: `Term` behind a fair mutex, the parser thread, the bounded
//!   feed queue (flow control), the sink and the coalesced wake-up.
//! - [`view`]: the GPUI view: keyboard (UI 12.5), mouse (selection, mouse
//!   reports, wheel), clipboard, the paste confirmation request (UI 4.3:
//!   [`TerminalEvent::ConfirmPaste`], the tab shows the dialog), focus.
//! - [`limits`]: caps on what output can make the emulator store (OSC
//!   string length, zero-width characters per cell).
//! - [`journal`]: the compact part of the scrollback (D85): rows older than
//!   the newest few hundred scrollback lines, as text and attribute runs.
//! - [`render`]: the element: only visible rows, cached row layouts.
//! - [`input`], [`mouse`], [`paste`], [`selection`], [`colors`]: the pure
//!   rules, unit tested.
//! - [`contrast`]: the minimum contrast of text (UI 12.4 "Adjust
//!   indistinguishable colors"); [`search`]: search in the output and the
//!   scrollback with match positions (UI 4.2 Tools).
//! - [`fonts`]: the embedded JetBrains Mono; [`settings`]: the view's
//!   settings, from Settings → Terminal's `TerminalPrefs` global.
//! - [`history_capture`]: command history capture (docs/security.md,
//!   section 10.1): the prompt boundary, the line at Enter read from the
//!   screen, injected input; candidates go to a [`HistorySink`].
//!
//! Security (docs/security.md, section 12): OSC 52 clipboard access is
//! disabled, window operations and text area size reports are not
//! answered, titles are plain text of at most 256 characters, pasted text
//! loses control characters and bracketed paste markers, OSC strings and
//! zero-width characters per cell are capped. Nothing logs terminal data.

pub mod autocomplete;
pub mod colors;
pub mod contrast;
pub mod fonts;
pub mod highlight;
pub mod history_capture;
pub mod input;
pub mod journal;
pub mod limits;
pub mod model;
pub mod mouse;
pub mod paste;
pub mod render;
pub mod search;
pub mod selection;
pub mod settings;
pub mod view;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

use gpui::App;

pub use autocomplete::HistorySource;
pub use contrast::MinContrast;
pub use history_capture::HistorySink;
pub use model::{FeedError, TerminalHandle, TerminalSink, TerminalSize};
pub use settings::{CursorStyleSetting, DEFAULT_SCROLLBACK, TerminalSettings};
pub use view::{
    KEY_CONTEXT, PendingPaste, SearchSummary, SearchUpdated, TerminalEvent, TerminalOptions,
    TerminalView,
};

/// Registers the embedded font and the terminal's key bindings (once; the
/// first [`TerminalView`] does it too). Call at startup, after
/// `gpui_kit::init`.
pub fn init(cx: &mut App) {
    fonts::register(cx);
    view::bind_keys(cx);
}
