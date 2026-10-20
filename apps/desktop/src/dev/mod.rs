//! Developer tools (cargo feature `dev-tools`; never in release builds).
//!
//! - [`demo`]: `shellpp --demo` runs the app on a temporary profile with
//!   generated data (300 hosts, 5 groups) and deletes it on exit.
//! - [`bench`]: `--bench-sessions N`, `--bench-output`, `--bench-latency`
//!   (memory with sessions, `seq 1 3000000` throughput through our terminal
//!   and through a bare PTY reader as the baseline, input latency;
//!   `scripts/measure-terminal.ps1`).
//! - [`test_server`]: the SSH test server of the benchmarks and the
//!   terminal screenshots (OpenSSH in Docker from `tests/openssh`, or the
//!   russh test server as a child process); never a real host.
//! - `screenshots` (feature `screenshots`, which adds gpui-kit's
//!   `test-support` for offscreen rendering): `shellpp --screenshots <dir>
//!   --size WxH` saves PNGs of our own window on demo data in one launch
//!   (`scripts/ui-screens.ps1`), including the terminal states (session
//!   tabs to the test server: `ls --color`, htop, the host key dialogs, a
//!   dropped connection, a local terminal, several tabs).

pub mod bench;
mod bench_pty;
mod bench_run;
pub mod demo;
#[cfg(feature = "screenshots")]
pub mod screenshots;
pub mod test_server;
