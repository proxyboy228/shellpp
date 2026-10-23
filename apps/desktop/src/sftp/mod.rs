//! The SFTP tab (UI 5; stage 4): two file panels ([`panel`]) on the logic
//! of a panel's rows ([`listing`]) and the local file system ([`local`]),
//! their menus and dialogs ([`menu`]), the transfer queue ([`queue`]) and
//! the tab itself ([`screen`]). A remote panel is an SFTP session of the
//! session layer (`SessionManager::open_sftp`); the transfers run in
//! `shellpp_ssh::{sftp, transfer, transfer_queue}`.

use gpui::prelude::*;
use gpui::{Context, IntoElement, SharedString, Window, div};
use gpui_kit::component::ActiveTheme as _;

pub mod listing;
pub mod local;
pub mod menu;
pub mod open;
pub mod panel;
pub mod queue;
pub mod screen;
pub mod strings;
#[cfg(test)]
mod tests;

pub use screen::SftpScreen;

use crate::theme::{self, metrics};

/// A file name as the tab shows it (security.md 11.4): control and
/// bidirectional override characters made visible as `<U+202E>`, so
/// `photo\u{202e}gpj.exe` cannot pass for a picture.
pub fn display_name(name: &str) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    };
    if !name.chars().any(hidden) {
        return name.to_owned();
    }
    name.chars()
        .map(|c| {
            if hidden(c) {
                format!("<U+{:04X}>", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// The extensions of files the system would run or install (security.md
/// 11.4): Open asks first.
pub const RUNNABLE: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "scr",
    "pif",
    "lnk",
    "url",
    "reg",
    "hta",
    "cpl",
    "jar",
    "app",
    "command",
    "sh",
    "desktop",
    "appimage",
    "run",
    "pkg",
    "dmg", // Run 8 (C5): more types Windows runs.
    "msc",
    "msu",
    "appref-ms",
    "application",
    "appx",
    "msix",
    "appinstaller",
    "chm",
    "gadget",
    "settingcontent-ms",
    "inf",
    "scf",
    "ws",
    "wsc",
    "sct",
];

/// The extension of `name` when the system would run such a file.
pub fn runnable_extension(name: &str) -> Option<String> {
    let extension = name.rsplit_once('.')?.1.trim().to_lowercase();
    RUNNABLE.contains(&extension.as_str()).then_some(extension)
}

/// The type of the downloaded copy `local_name` of an Open that still asks
/// before it runs: the system picks the program (`by_system`) and the user
/// has not agreed to run this type already. A link listed under one name
/// brings the name of the file it points to (run 8, C5; security.md 11.4).
pub fn copy_needs_question(
    local_name: &str,
    by_system: bool,
    confirmed: Option<&str>,
) -> Option<String> {
    runnable_extension(local_name)
        .filter(|extension| by_system && confirmed != Some(extension.as_str()))
}

/// What a drag from a file panel carries: the entries and their panel.
#[derive(Clone, Debug)]
pub struct DraggedFiles {
    pub side: panel::Side,
    pub entries: Vec<listing::Entry>,
}

/// The label under the pointer while files are dragged.
#[derive(Default)]
pub struct DragLabel {
    text: SharedString,
}

impl std::fmt::Debug for DragLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DragLabel").finish_non_exhaustive()
    }
}

impl DragLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(metrics::SPACE_3)
            .py(metrics::SPACE_1)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(theme.popover)
            .border_1()
            .border_color(theme::tokens(cx).border_strong.hsla())
            .text_color(theme.popover_foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(self.text.clone())
    }
}
