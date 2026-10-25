//! Terminal sessions: the session layer is the crate `shellpp-sessions`
//! (run 13, D212), shared with a phone client; this module re-exports it
//! under its old path and adds the desktop's GPUI glue ([`ui`]). The
//! session tests stay here: they open the real core through the desktop's
//! test profile.

pub use shellpp_sessions::*;

pub mod ui;

#[cfg(test)]
mod tests;
