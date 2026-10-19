//! Developer tools (cargo feature `dev-tools`; never in release builds).
//!
//! - [`demo`]: `shellpp --demo` runs the app on a temporary profile with
//!   generated data (300 hosts, 5 groups) and deletes it on exit.

pub mod demo;
