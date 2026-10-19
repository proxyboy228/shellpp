// Shell++ desktop entry point. Hides the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shellpp_desktop::run();
}
