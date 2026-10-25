//! The terminal model (run 13, D212): alacritty's parser and grid on a
//! parser thread of each terminal ([`model`]), the compact scrollback
//! journal (D85), the memory limits, search, selection, paste, the cells'
//! colors and the minimum contrast, the capture of command lines for the
//! history (docs/security.md, section 10.1) and the autocomplete
//! suggestions.
//!
//! Shared by the desktop app and a phone client: no GPUI. Each client draws
//! the cells and turns its input into bytes (the desktop's
//! `apps/desktop/src/terminal`: render, view, input, mouse); the model wakes
//! it through the channel given to [`TerminalHandle::spawn`] and the client
//! reads the screen under the terminal's lock.

pub mod autocomplete;
pub mod colors;
pub mod contrast;
pub mod history_capture;
pub mod journal;
pub mod limits;
pub mod model;
pub mod paste;
pub mod search;
pub mod selection;

pub use autocomplete::HistorySource;
pub use contrast::MinContrast;
pub use history_capture::HistorySink;
pub use model::{FeedError, TerminalHandle, TerminalSink, TerminalSize};
