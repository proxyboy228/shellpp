//! Autocomplete (UI 4.2, D142; `shellpp-terminal`, run 13): the commands
//! and the suggestion slot live in the crate; the view's drawing and
//! accepting of a suggestion are here.

use gpui::{AppContext as _, Context};
pub use shellpp_terminal::autocomplete::*;

use super::view::TerminalView;

impl TerminalView {
    /// The suggestion to draw after the cursor now: the rest of a command
    /// of the history, when autocomplete is on and the typed line allows
    /// it. Starts reading the history the first time.
    pub fn suggestion(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if !self.settings().autocomplete {
            return None;
        }
        let slot = self.handle().autocomplete();
        if slot.typed_ahead() {
            return None;
        }
        match slot.list_state() {
            ListState::Ready => {}
            ListState::Missing => return None,
            ListState::Load { generation, source } => {
                let load = cx.background_spawn(async move { Suggestions::new(source.commands()) });
                let slot = slot.clone();
                cx.spawn(async move |view, cx| {
                    slot.finish_load(generation, load.await);
                    let _ = view.update(cx, |_, cx| cx.notify());
                })
                .detach();
                return None;
            }
        }
        let typed = self.handle().typed_line()?;
        slot.complete(&typed)
    }

    /// The suggestion the next frame draws: [`Self::suggestion`] while the
    /// cursor can be seen (the view is at the bottom, the cursor is shown,
    /// the session is live, no IME text is being composed).
    pub fn visible_suggestion(&mut self, cx: &mut Context<Self>) -> Option<String> {
        if self.dimmed()
            || self.composing()
            || self.handle().scrolled_back()
            || !self.cursor_shown()
        {
            return None;
        }
        self.suggestion(cx)
    }

    /// Tab or →: types the suggestion when the last frame drew it and it
    /// is still the one in effect; `false` (the key goes to the shell)
    /// otherwise.
    pub fn accept_suggestion(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(shown) = self.take_shown_suggestion() else {
            return false;
        };
        if self.visible_suggestion(cx).as_deref() != Some(shown.as_str()) {
            return false;
        }
        // Typed input: the user chose it, and the line is recorded as any
        // typed line is.
        self.type_bytes(shown.into_bytes(), cx);
        true
    }
}
