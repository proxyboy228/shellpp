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
