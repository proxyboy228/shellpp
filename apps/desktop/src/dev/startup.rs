//! Cold start measurement (`SPEC.md`, "Performance": from launch to the
//! first frame of the main window under 1 s; `scripts/measure-startup.ps1`).
//!
//! With `SHELLPP_BENCH_STARTUP=1` in a `dev-tools` build, the app prints
//! `bench: first frame <ms> ms` once the main window has presented its first
//! frame and quits. The time counts from the start of `main`; the script
//! also times the launch from outside (process creation, loading). Nothing
//! else changes: the run is a normal launch of whatever the data directory
//! holds (a fresh profile shows the first-run screen).

use std::sync::OnceLock;
use std::time::Instant;

use gpui::{AnyWindowHandle, App};

/// The environment variable that turns the measurement on.
pub const ENV: &str = "SHELLPP_BENCH_STARTUP";

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Marks the start of `main`. Call it first.
pub fn mark_start() {
    let _ = STARTED.set(Instant::now());
}

fn armed() -> bool {
    std::env::var_os(ENV).is_some_and(|value| value == "1")
}

#[allow(clippy::print_stdout)]
fn say(line: &str) {
    println!("bench: {line}");
}

/// After the main window opened: once its first frame is on screen, prints
/// the time since [`mark_start`] and quits (only when [`ENV`] is `1`).
pub fn arm(window: AnyWindowHandle, cx: &mut App) {
    if !armed() {
        return;
    }
    let _ = window.update(cx, |_, window, _| {
        // Next-frame callbacks run at the start of a frame, before it is
        // drawn: the inner one runs at the start of the frame after the
        // first, once the first frame was drawn and presented (at most one
        // frame late).
        window.on_next_frame(|window, _| {
            window.on_next_frame(|_, cx| {
                let started = STARTED.get().copied().unwrap_or_else(Instant::now);
                say(&format!("first frame {} ms", started.elapsed().as_millis()));
                cx.quit();
            });
        });
    });
}
