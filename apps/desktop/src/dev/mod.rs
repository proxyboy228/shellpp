//! Developer tools (cargo feature `dev-tools`; never in release builds).
//!
//! - [`demo`]: `shellpp --demo` runs the app on a temporary profile with
//!   generated data (300 hosts, 5 groups) and deletes it on exit.
//! - `screenshots` (feature `screenshots`, which adds gpui-kit's
//!   `test-support` for offscreen rendering): `shellpp --screenshots <dir>
//!   --size WxH` saves PNGs of our own window on demo data in one launch
//!   (`scripts/ui-screens.ps1`).

pub mod demo;
#[cfg(feature = "screenshots")]
pub mod screenshots;
